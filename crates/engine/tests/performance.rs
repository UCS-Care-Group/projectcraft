//! Rescheduling speed on a realistic plan, from a deterministic generator: summaries three levels
//! deep, all four link types with lag and lead, constraints of every type, deadlines, and a
//! calendar with holidays and working Saturdays.
//!
//! The gate (`reschedule_200_tasks_under_10_ms`) is `#[ignore]`d because timings mean nothing in
//! a debug build; `cargo xtask ci` runs it in release (`cargo xtask perf`). A 2,000-task plan,
//! and the 200-task plan staffed with 30 resources on calendars of their own, are timed and
//! reported, not gated. Typing a long Resource Names text into a large plan is gated too
//! (`resource_names_on_a_large_plan_stay_fast`), and so is the resource usage of a plan at the
//! assignment cap.

use std::time::{Duration, Instant};

use projectcraft_engine::Session;
use projectcraft_engine::model::{Assignment, Constraint, Date, Link, LinkKind, Project, ResourceId, ResourceType, TaskId, Units};
use serde_json::json;

/// The gate: the median reschedule of the 200-task plan, in release.
const GATE: Duration = Duration::from_millis(10);
/// Timed runs per plan; the median is reported.
const RUNS: usize = 31;
/// The most a hostile plan's finish-type hold search may cost a reschedule, in release. The search
/// is bounded by its step budget, not by this ceiling: about 190 ms on a developer machine and
/// 310 ms on a shared Windows CI runner. The ceiling still catches a budget raised far enough to
/// make every edit of a hostile file take seconds.
const HOSTILE_GATE: Duration = Duration::from_secs(1);
/// The most working out the resource usage of a plan at the assignment cap may take, in release:
/// about 0.4 s on a developer machine. The UI works it out once per change, not every frame.
const USAGE_GATE: Duration = Duration::from_secs(2);

/// A day of duration at the default 8 hours, in working minutes.
const DAY: i64 = 480;

/// A small linear congruential generator (Knuth's MMIX constants): the same seed, the same plan.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    /// A number in `0..n`.
    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }
}

fn date(s: &str) -> Date {
    s.parse().unwrap()
}

/// A plan of `n` tasks. Phases (level 1) hold 3 work packages (level 2), each holding 3 groups
/// (level 3) of 8 tasks (level 4). Tasks run 0–10 days (some milestones) and link to 1–2 of the
/// previous 15 tasks with any link type and a lag of −2 to +3 days; work packages are chained FS
/// to the one before. Every 9th task has a constraint, every 11th a deadline, and about one group
/// in four must finish no earlier than a date. The calendar has
/// about one holiday a month and a few working Saturdays.
fn plan(n: usize, seed: u64) -> Project {
    let start = date("2026-01-05");
    let mut rng = Lcg(seed);
    let mut p = Project::new(format!("Generated {n}"), start);
    for k in 0..(n as i64 / 8).max(12) {
        let d = start.add_days(k * 31 + rng.below(28) as i64).unwrap();
        p.calendar.add_exception(d, false).unwrap();
    }
    for k in 0..6 {
        // A Saturday: 2026-01-10 is one.
        let sat = date("2026-01-10").add_days(7 * (5 + 9 * k)).unwrap();
        p.calendar.add_exception(sat, true).unwrap();
    }

    let mut leaves: Vec<TaskId> = Vec::new();
    let mut prev_package: Option<TaskId> = None;
    'all: for phase in 1.. {
        for (level, name) in phase_rows(phase) {
            if p.tasks.len() >= n {
                break 'all;
            }
            let id = p.add_task(name, DAY).unwrap();
            p.task_mut(id).unwrap().outline_level = level;
            match level {
                2 => {
                    if let Some(prev) = prev_package {
                        p.set_preds(id, vec![Link { pred: prev, kind: LinkKind::FS, lag: 0, lag_unit: Default::default() }]).unwrap();
                    }
                    prev_package = Some(id);
                }
                3 if rng.chance(25) => {
                    // A group that must finish no earlier than a date: a finish-type summary hold.
                    let d = start.add_days(leaves.len() as i64 * 2 + 10 + rng.below(30) as i64).unwrap();
                    p.task_mut(id).unwrap().constraint = Constraint::FinishNoEarlierThan(d);
                }
                4 => {
                    let k = leaves.len();
                    let t = p.task_mut(id).unwrap();
                    t.duration = if rng.chance(8) { 0 } else { (1 + rng.below(10) as i64) * DAY };
                    let near = |rng: &mut Lcg| start.add_days(k as i64 * 2 + rng.below(40) as i64).unwrap();
                    if k % 9 == 4 {
                        let d = near(&mut rng);
                        t.constraint = match rng.below(6) {
                            0 => Constraint::StartNoEarlierThan(d),
                            1 => Constraint::FinishNoEarlierThan(d),
                            2 => Constraint::StartNoLaterThan(d),
                            3 => Constraint::FinishNoLaterThan(d),
                            4 => Constraint::MustStartOn(d),
                            _ => Constraint::MustFinishOn(d),
                        };
                    }
                    if k % 11 == 7 {
                        t.deadline = Some(near(&mut rng).add_days(20).unwrap());
                    }
                    let mut links = Vec::new();
                    if k > 0 && rng.chance(90) {
                        for _ in 0..1 + rng.below(2) {
                            let back = 1 + rng.below(15.min(k as u64)) as usize;
                            let pred = leaves[k - back];
                            if links.iter().any(|l: &Link| l.pred == pred) {
                                continue;
                            }
                            let kind = match rng.below(20) {
                                0..12 => LinkKind::FS,
                                12..15 => LinkKind::SS,
                                15..18 => LinkKind::FF,
                                _ => LinkKind::SF,
                            };
                            links.push(Link { pred, kind, lag: (rng.below(6) as i64 - 2) * DAY, lag_unit: Default::default() });
                        }
                    }
                    p.set_preds(id, links).unwrap();
                    leaves.push(id);
                }
                _ => {}
            }
        }
    }
    p
}

/// `plan` staffed: 30 work resources, each with about a week's leave a quarter and every third
/// one part time (Fridays off), and one or two of them on each task with duration.
fn staffed(n: usize, seed: u64) -> Project {
    let mut p = plan(n, seed);
    let mut rng = Lcg(seed ^ 0x5eed);
    let people: Vec<ResourceId> = (0..30).map(|k| p.add_resource(Some(&format!("Person {k}")), ResourceType::Work, None).unwrap()).collect();
    for (k, r) in people.iter().enumerate() {
        for q in 0..4 {
            let first = date("2026-01-05").add_days(q * 91 + rng.below(80) as i64).unwrap();
            for d in 0..5 {
                p.set_resource_exception(*r, first.add_days(d).unwrap(), vec![]).unwrap();
            }
        }
        if k % 3 == 0 {
            p.set_resource_weekday_times(*r, 4, Some(vec![])).unwrap();
        }
    }
    let tasks: Vec<TaskId> = p.tasks.iter().filter(|t| t.outline_level == 4 && t.duration > 0).map(|t| t.id).collect();
    for t in tasks {
        for _ in 0..1 + rng.below(2) {
            let _ = p.assign(t, people[rng.below(30) as usize], None);
        }
    }
    p
}

/// One phase's rows: (outline level, name).
fn phase_rows(phase: usize) -> Vec<(u32, String)> {
    let mut rows = vec![(1, format!("Phase {phase}"))];
    for w in 1..=3 {
        rows.push((2, format!("Package {phase}.{w}")));
        for g in 1..=3 {
            rows.push((3, format!("Group {phase}.{w}.{g}")));
            rows.extend((1..=8).map(|t| (4, format!("Task {phase}.{w}.{g}.{t}"))));
        }
    }
    rows
}

/// The median time of a reschedule after an edit: one task's duration changes back and forth,
/// through the command the task sheet uses, so the time includes the undo snapshot and roll-up.
fn median_reschedule(s: &mut Session, runs: usize) -> Duration {
    let id = s.project.tasks.iter().find(|t| t.outline_level == 4 && t.duration > 0).map(|t| t.id).unwrap();
    let mut times: Vec<Duration> = (0..runs + 3)
        .map(|k| {
            let t0 = Instant::now();
            s.run("task.setDuration", &json!({"id": id.0, "duration": 2 + k % 2})).unwrap();
            t0.elapsed()
        })
        .skip(3) // warm-up
        .collect();
    assert!(s.schedule.is_ok());
    times.sort();
    times[times.len() / 2]
}

/// The generated plans have what they claim, schedule, and give the same dates every time.
#[test]
fn the_generated_plan_is_realistic_and_schedules_deterministically() {
    let p = plan(200, 19);
    assert_eq!(p.tasks.len(), 200);
    assert_eq!(p, plan(200, 19), "same seed, same plan");
    for level in 1..=4 {
        assert!(p.tasks.iter().any(|t| t.outline_level == level), "level {level}");
    }
    let links: Vec<&Link> = p.tasks.iter().flat_map(|t| &t.preds).collect();
    for kind in [LinkKind::FS, LinkKind::SS, LinkKind::FF, LinkKind::SF] {
        assert!(links.iter().any(|l| l.kind == kind), "{kind:?}");
    }
    assert!(links.iter().any(|l| l.lag > 0) && links.iter().any(|l| l.lag < 0));
    assert!(p.tasks.iter().filter(|t| !t.constraint.is_asap()).count() >= 10);
    assert!(p.tasks.iter().any(|t| t.outline_level == 3 && !t.constraint.is_asap()), "a summary with a finish-type hold");
    assert!(p.tasks.iter().filter(|t| t.deadline.is_some()).count() >= 10);
    assert!(p.calendar.exceptions.values().any(|w| *w) && p.calendar.exceptions.values().any(|w| !*w));

    let a = Session::new(p.clone());
    let b = Session::new(p);
    assert_eq!(a.schedule.as_ref().unwrap(), b.schedule.as_ref().unwrap());
    // The saved file schedules to the same dates.
    let c = Session::load_json(&a.save_json().unwrap()).unwrap();
    assert_eq!(a.schedule.as_ref().unwrap(), c.schedule.as_ref().unwrap());
    assert!(Session::new(plan(2_000, 19)).schedule.is_ok());
    // Staffed, on resource calendars: the same dates every time, and some tasks moved by them.
    let staffed_plan = staffed(200, 19);
    let (a, b) = (Session::new(staffed_plan.clone()), Session::new(staffed_plan.clone()));
    assert_eq!(a.schedule.as_ref().unwrap(), b.schedule.as_ref().unwrap());
    let mut bare = staffed_plan;
    for r in &mut bare.resources {
        r.calendar = Default::default();
    }
    assert_ne!(a.schedule.as_ref().unwrap().tasks, Session::new(bare).schedule.unwrap().tasks, "leave moves tasks");
}

/// The gate: a 200-task plan reschedules in under 10 ms in release (median of 31 edits). Also
/// times and prints a 2,000-task plan, not gated. In a debug build it only reports.
#[test]
#[ignore = "timing; run in release by `cargo xtask perf`"]
fn reschedule_200_tasks_under_10_ms() {
    let mut small = Session::new(plan(200, 19));
    let t200 = median_reschedule(&mut small, RUNS);
    let mut large = Session::new(plan(2_000, 19));
    let t2000 = median_reschedule(&mut large, RUNS);
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    let mut crewed = Session::new(staffed(200, 19));
    let t_crewed = median_reschedule(&mut crewed, RUNS);
    let crewed_schedule = crewed.schedule.clone().unwrap();
    let t0 = Instant::now();
    let u = projectcraft_engine::model::usage(&crewed.project, &crewed_schedule);
    let t_usage = t0.elapsed();
    assert!(u.is_ok());
    println!("perf ({build}, median of {RUNS}): reschedule 200 tasks {t200:.2?}, 2,000 tasks {t2000:.2?} (gate: 200 under {GATE:?})");
    println!(
        "perf ({build}, median of {RUNS}): reschedule 200 tasks with 30 resources on their own calendars {t_crewed:.2?}, their usage {t_usage:.2?} (reported)"
    );
    if !cfg!(debug_assertions) {
        assert!(t200 < GATE, "200-task reschedule took {t200:.2?}, over the {GATE:?} gate");
    }
}

/// A hostile plan for the finish-type hold search: one summary over everything, Finish No
/// Earlier Than a far date, holding blocks of `depth` nested summaries with the same constraint,
/// each over two subtasks the hold can't move (Must Start On and Start No Later Than the project
/// start). No hold ever reaches its date, so every search runs to its bounds, and the outer one
/// reschedules the whole plan at each try. The worst shape found within the search's bounds.
fn hostile(n: usize, depth: u32) -> Project {
    let start = date("2026-01-05");
    let far = date("9000-01-01");
    let mut p = Project::new("Hostile", start);
    let add = |p: &mut Project, level: u32, duration: i64, c: Constraint| {
        let id = p.add_task("T", duration * DAY).unwrap();
        let t = p.task_mut(id).unwrap();
        t.outline_level = level;
        t.constraint = c;
    };
    add(&mut p, 1, 1, Constraint::FinishNoEarlierThan(far));
    while p.tasks.len() + depth as usize + 2 <= n {
        for l in 1..=depth {
            add(&mut p, 1 + l, 1, Constraint::FinishNoEarlierThan(far));
        }
        add(&mut p, depth + 2, 2, Constraint::MustStartOn(start));
        add(&mut p, depth + 2, 2, Constraint::StartNoLaterThan(start));
    }
    p
}

/// [`hostile`] staffed: 10 people with a day's leave each on every subtask, and a project
/// calendar of 2,000 exceptions, so every hold-search step also walks resource calendars.
fn hostile_staffed() -> Project {
    let mut p = hostile(projectcraft_engine::model::MAX_TASKS, 9);
    let start = date("2026-01-05");
    for k in 0..2_000 {
        p.calendar.add_exception(start.add_days(k).unwrap(), k % 7 != 5).unwrap();
    }
    let people: Vec<ResourceId> = (0..10).map(|k| p.add_resource(Some(&format!("P{k}")), ResourceType::Work, None).unwrap()).collect();
    for r in &people {
        p.set_resource_exception(*r, date("2026-02-02"), vec![]).unwrap();
    }
    let deepest = p.tasks.iter().map(|t| t.outline_level).max().unwrap_or(1);
    let leaves: Vec<(TaskId, i64)> = p.tasks.iter().filter(|t| t.outline_level == deepest).map(|t| (t.id, t.duration)).collect();
    for (t, d) in leaves {
        for r in &people {
            p.assignments.push(Assignment { task: t, resource: *r, units: Units::Work(10_000), work: d, duration: d });
        }
    }
    p
}

/// The most assignments a plan holds, on resource calendars: 100,000 one-day tasks, each with all
/// 10 people (each with a day's leave) on it, so 1,000,000 assignments, and a project calendar of
/// 2,000 exceptions. No links: the cost is walking every assignment on its calendar.
fn staffed_to_the_cap() -> Project {
    let mut p = Project::new("Capped", date("2026-01-05"));
    let start = date("2026-01-05");
    for k in 0..2_000 {
        p.calendar.add_exception(start.add_days(k).unwrap(), k % 7 != 5).unwrap();
    }
    for k in 0..projectcraft_engine::model::MAX_TASKS {
        p.add_task(format!("T{k}"), DAY).unwrap();
    }
    let people: Vec<ResourceId> = (0..10).map(|k| p.add_resource(Some(&format!("P{k}")), ResourceType::Work, None).unwrap()).collect();
    for r in &people {
        p.set_resource_exception(*r, date("2026-02-02"), vec![]).unwrap();
    }
    let tasks: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
    for t in tasks {
        for r in &people {
            p.assignments.push(Assignment { task: t, resource: *r, units: Units::Work(10_000), work: DAY, duration: DAY });
        }
    }
    p
}

/// The worst case the finish-type hold search allows: a full-size hostile plan (`MAX_TASKS`
/// tasks) schedules within [`HOSTILE_GATE`] in release (median of 3), the same dates every time.
#[test]
#[ignore = "timing; run in release by `cargo xtask perf`"]
fn hostile_finish_holds_stay_bounded() {
    let p = hostile(projectcraft_engine::model::MAX_TASKS, 9);
    let mut times = Vec::new();
    let mut last = None;
    for _ in 0..3 {
        let t0 = Instant::now();
        let s = projectcraft_engine::model::schedule(&p);
        times.push(t0.elapsed());
        assert!(s.is_ok());
        if let Some(prev) = &last {
            assert_eq!(prev, &s, "same dates every time");
        }
        last = Some(s);
    }
    times.sort();
    let t = times[1];
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!("perf ({build}, median of 3): hostile finish holds, {} tasks {t:.2?} (gate: under {HOSTILE_GATE:?})", p.tasks.len());
    if !cfg!(debug_assertions) {
        assert!(t < HOSTILE_GATE, "hostile schedule took {t:.2?}, over the {HOSTILE_GATE:?} gate");
    }
    // Staffed on resource calendars: the hold searches' budget counts the calendar walks too.
    let p = hostile_staffed();
    let t0 = Instant::now();
    let s = projectcraft_engine::model::schedule(&p);
    let t = t0.elapsed();
    assert!(s.is_ok());
    assert_eq!(s, projectcraft_engine::model::schedule(&p), "same dates every time");
    println!(
        "perf ({build}): hostile finish holds staffed on resource calendars, {} tasks, {} assignments, 2,000 exceptions {t:.2?} (gate: under {HOSTILE_GATE:?})",
        p.tasks.len(),
        p.assignments.len()
    );
    if !cfg!(debug_assertions) {
        assert!(t < HOSTILE_GATE, "staffed hostile schedule took {t:.2?}, over the {HOSTILE_GATE:?} gate");
    }
    // As many assignments as a plan holds, every one walked on its resource's calendar.
    let p = staffed_to_the_cap();
    let t0 = Instant::now();
    let s = projectcraft_engine::model::schedule(&p);
    let t = t0.elapsed();
    assert!(s.is_ok());
    println!(
        "perf ({build}): {} tasks with {} assignments on resource calendars, 2,000 exceptions {t:.2?} (gate: under {HOSTILE_GATE:?})",
        p.tasks.len(),
        p.assignments.len()
    );
    if !cfg!(debug_assertions) {
        assert!(t < HOSTILE_GATE, "a schedule at the assignment cap took {t:.2?}, over the {HOSTILE_GATE:?} gate");
    }
    // Its resource usage, day by day: every assignment walked once more.
    let s = s.unwrap();
    let t0 = Instant::now();
    let u = projectcraft_engine::model::usage(&p, &s);
    let t = t0.elapsed();
    assert!(u.is_ok());
    println!("perf ({build}): resource usage of {} assignments {t:.2?} (gate: under {USAGE_GATE:?})", p.assignments.len());
    if !cfg!(debug_assertions) {
        assert!(t < USAGE_GATE, "usage at the assignment cap took {t:.2?}, over the {USAGE_GATE:?} gate");
    }
}

/// The most typing 1,000 names into Resource Names on the large plan may take, in release,
/// undo snapshot and reschedule included: it was about half a second when each step copied the
/// whole project.
const NAMES_GATE: Duration = Duration::from_millis(100);

/// A plan of 5,000 one-day tasks and 1,000 work resources, each task with 4 of them at 100%:
/// 20,000 assignments.
fn crowded() -> Project {
    let mut p = Project::new("Crowded", date("2026-01-05"));
    for k in 0..5_000 {
        p.add_task(format!("Task {k}"), DAY).unwrap();
    }
    let people: Vec<ResourceId> = (0..1_000).map(|k| p.add_resource(Some(&format!("R{k}")), ResourceType::Work, None).unwrap()).collect();
    let tasks: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
    for (k, t) in tasks.iter().enumerate() {
        for j in 0..4 {
            let resource = people[(k * 4 + j) % people.len()];
            p.assignments.push(Assignment { task: *t, resource, units: Units::Work(10_000), work: DAY, duration: DAY });
        }
    }
    p
}

/// Resource Names with 1,000 names (the most a task takes), typed into a 5,000-task plan with
/// 20,000 assignments, then replaced by 1,000 others: each a single command, under
/// [`NAMES_GATE`] in release (median of 5).
#[test]
#[ignore = "timing; run in release by `cargo xtask perf`"]
fn resource_names_on_a_large_plan_stay_fast() {
    let names = |from: usize| (from..from + 1_000).map(|k| format!("R{}", k % 1_000)).collect::<Vec<_>>().join(", ");
    let (first, second) = (names(0), names(500));
    let mut times = Vec::new();
    for _ in 0..5 {
        let mut s = Session::new(crowded());
        let id = s.project.tasks[2_500].id.0;
        for text in [&first, &second] {
            let t0 = Instant::now();
            s.run("task.setResourceNames", &json!({"id": id, "text": text})).unwrap();
            times.push(t0.elapsed());
        }
        assert_eq!(s.project.assignments_of(TaskId(id)).count(), 1_000);
    }
    times.sort();
    let t = times[times.len() / 2];
    let build = if cfg!(debug_assertions) { "debug" } else { "release" };
    println!(
        "perf ({build}, median of {}): Resource Names with 1,000 names on 5,000 tasks and 20,000 assignments {t:.2?} (gate: under {NAMES_GATE:?})",
        times.len()
    );
    if !cfg!(debug_assertions) {
        assert!(t < NAMES_GATE, "Resource Names took {t:.2?}, over the {NAMES_GATE:?} gate");
    }
}
