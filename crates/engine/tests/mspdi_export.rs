//! Microsoft Project XML export round trips: export → import → export is byte for byte the same,
//! and import → export → import gives the same project, over generated projects (every field
//! the export writes, with hostile text) and the import fixtures.

use projectcraft_engine::Session;
use projectcraft_engine::interchange::Format;
use projectcraft_engine::model::{
    AccrueAt, Constraint, Currency, Date, DurationUnit, Link, LinkKind, Outline, Project, ResourceId, ResourceType, TaskId, TaskType, Units,
    WorkingTime,
};
use proptest::prelude::*;

/// A small linear congruential generator: the same seed, the same project.
struct Lcg(u64);

impl Lcg {
    fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    fn signed(&mut self, n: i64) -> i64 {
        self.below(2 * n as u64 + 1) as i64 - n
    }

    fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Text with what XML must escape, non-ASCII letters and a character XML can't hold at all.
fn text(rng: &mut Lcg) -> String {
    const BITS: [&str; 12] = ["Dig", "&", "<b>", "\"quoted\"", "it's", "café", "Ω", "  ", "\u{1}", "日本", "]]>", "\t"];
    let n = 1 + rng.below(4);
    let s: String = (0..n).map(|_| *rng.pick(&BITS)).collect::<Vec<_>>().join(" ");
    s.chars().filter(|c| !c.is_control()).collect()
}

fn times(rng: &mut Lcg) -> Vec<WorkingTime> {
    let wt = |a, b| WorkingTime::new(a, b).unwrap();
    match rng.below(5) {
        0 => vec![],
        1 => vec![wt(480, 720), wt(780, 1020)],
        2 => vec![wt(540, 780)],
        3 => vec![wt(0, 1440)],
        _ => vec![wt(420, 600), wt(660, 900), wt(960, 1200)],
    }
}

/// A project with every field the export writes, built from `seed` through the model's own
/// edits, then saved and loaded so it is exactly what a session holds.
fn generated(seed: u64) -> Session {
    let mut rng = Lcg(seed);
    let start = Date::from_ymd(2026, 1, 5).unwrap().add_days(rng.below(400) as i64).unwrap();
    let mut p = Project::new(text(&mut rng), start);
    p.options.minutes_per_day = *rng.pick(&[420, 450, 480, 600]);
    p.options.minutes_per_week = *rng.pick(&[2100, 2250, 2400, 3000]);
    p.options.days_per_month = *rng.pick(&[20, 21, 22]);
    p.options.task_type = *rng.pick(&[TaskType::FixedUnits, TaskType::FixedDuration, TaskType::FixedWork]);
    p.options.effort_driven = rng.chance(50);
    p.currency = rng
        .pick(&[
            Currency::default(),
            Currency::new("USD", None, None).unwrap(),
            Currency::new("JPY", None, None).unwrap(),
            Currency::new("BHD", Some("BD"), Some(3)).unwrap(),
            Currency::new("EUR", Some("€"), None).unwrap(),
        ])
        .clone();
    for i in 0..7 {
        if rng.chance(30) {
            let _ = p.calendar.set_weekday_times(i, times(&mut rng));
        }
    }
    for _ in 0..rng.below(12) {
        let d = start.add_days(rng.signed(60) + 30).unwrap();
        let _ = p.calendar.set_exception(d, times(&mut rng));
        // Runs of the same exception, written as one.
        if rng.chance(30) {
            let t = p.calendar.times_on(d).to_vec();
            for k in 1..=rng.below(4) as i64 {
                let _ = p.calendar.set_exception(d.add_days(k).unwrap(), t.clone());
            }
        }
    }
    let n = rng.below(14) as usize;
    let mut levels: Vec<u32> = Vec::new();
    for row in 0..n {
        let level = match levels.last() {
            None => 1,
            Some(&l) => 1 + rng.below(u64::from(l) + 1) as u32,
        };
        levels.push(level);
        let id = p.add_task(text(&mut rng), (rng.below(20) as i64) * 60 * rng.below(9) as i64).unwrap();
        let near = |rng: &mut Lcg| start.add_days(rng.signed(30) + 20).unwrap();
        let constraint = match rng.below(9) {
            0 => Constraint::MustStartOn(near(&mut rng)),
            1 => Constraint::MustFinishOn(near(&mut rng)),
            2 => Constraint::StartNoEarlierThan(near(&mut rng)),
            3 => Constraint::StartNoLaterThan(near(&mut rng)),
            4 => Constraint::FinishNoEarlierThan(near(&mut rng)),
            5 => Constraint::FinishNoLaterThan(near(&mut rng)),
            _ => Constraint::Asap,
        };
        let deadline = rng.chance(20).then(|| near(&mut rng));
        let unit = *rng.pick(&[DurationUnit::Minutes, DurationUnit::Hours, DurationUnit::Days, DurationUnit::Weeks, DurationUnit::Months]);
        let fixed_cost = if rng.chance(30) { rng.below(100_000_000) as i64 } else { 0 };
        {
            let t = p.task_mut(id).unwrap();
            t.outline_level = level;
            t.constraint = constraint;
            t.deadline = deadline;
            t.duration_unit = unit;
            t.fixed_cost = fixed_cost;
            t.priority = if rng.chance(30) { rng.below(1001) as u32 } else { 500 };
            t.leveling_delay = if rng.chance(20) { rng.below(5_000) as i64 } else { 0 };
        }
        let ty = *rng.pick(&[TaskType::FixedUnits, TaskType::FixedDuration, TaskType::FixedWork]);
        let _ = p.set_task_type(id, Some(ty), Some(rng.chance(50)));
        // Links to earlier rows that aren't this row's summaries.
        let outline = Outline { levels: levels.clone() };
        let mut preds: Vec<Link> = Vec::new();
        for _ in 0..rng.below(3) {
            if row == 0 {
                break;
            }
            let j = rng.below(row as u64) as usize;
            if outline.subtree_end(j) > row {
                continue;
            }
            let pred = p.tasks[j].id;
            if preds.iter().any(|l| l.pred == pred) {
                continue;
            }
            let kind = *rng.pick(&[LinkKind::FS, LinkKind::SS, LinkKind::FF, LinkKind::SF]);
            let lag_unit = *rng.pick(&[DurationUnit::Minutes, DurationUnit::Hours, DurationUnit::Days, DurationUnit::Weeks]);
            preds.push(Link { pred, kind, lag: rng.signed(4) * 240 + rng.signed(1) * 7, lag_unit });
        }
        let _ = p.set_preds(id, preds);
    }
    let mut people: Vec<ResourceId> = Vec::new();
    for k in 0..rng.below(6) {
        let kind = *rng.pick(&[ResourceType::Work, ResourceType::Work, ResourceType::Material, ResourceType::Cost]);
        let name = format!("{} {k}", text(&mut rng).replace([',', '[', ']'], ""));
        let Ok(id) = p.add_resource(Some(&name), kind, None) else { continue };
        people.push(id);
        let r = p.resources.iter_mut().find(|r| r.id == id).unwrap();
        r.initials = if rng.chance(20) { String::new() } else { "A&B".into() };
        r.group = if rng.chance(50) { "Ops <east>".into() } else { String::new() };
        r.material_label = if rng.chance(50) { "m³".into() } else { String::new() };
        r.max_units = (rng.below(40) as u32) * 2_500;
        r.standard_rate = rng.below(1_000_000) as i64;
        r.overtime_rate = rng.below(1_000_000) as i64;
        r.cost_per_use = rng.below(100_000) as i64;
        r.accrue_at = *rng.pick(&[AccrueAt::Start, AccrueAt::End, AccrueAt::Prorated]);
        if kind == ResourceType::Work {
            for i in 0..7 {
                // A weekday of the resource's own the same as the project's is read back as
                // following the project's (`comparable`); the scheduler can give such a plan other
                // slack, so the generated plans leave it out (see `a_resource_weekday_like_the_project_s`).
                let t = times(&mut rng);
                if rng.chance(20) && t != p.calendar.weekday_times(i) {
                    let _ = p.set_resource_weekday_times(id, i, Some(t));
                }
            }
            for _ in 0..rng.below(4) {
                let _ = p.set_resource_exception(id, start.add_days(rng.below(60) as i64).unwrap(), times(&mut rng));
            }
        }
    }
    let ids: Vec<TaskId> = p.tasks.iter().map(|t| t.id).collect();
    for _ in 0..rng.below(12) {
        if ids.is_empty() || people.is_empty() {
            break;
        }
        let (t, r) = (*rng.pick(&ids), *rng.pick(&people));
        let kind = p.resources.iter().find(|x| x.id == r).map(|x| x.kind).unwrap();
        let units = match kind {
            ResourceType::Work => Units::Work((rng.below(16) as u32) * 2_500),
            ResourceType::Material => Units::Fixed(rng.below(100_000) as i64),
            ResourceType::Cost => Units::Cost(rng.below(10_000_000) as i64),
        };
        let _ = p.assign(t, r, Some(units));
    }
    let s = Session::new(p);
    Session::load_json(&s.save_json().unwrap()).unwrap()
}

/// What an export → import keeps of `like`: everything but the counters and views the file has
/// no place for, with names trimmed of spaces at their ends, as the importer reads every name.
fn comparable(mut p: Project, like: &Project) -> Project {
    for (t, l) in p.tasks.iter_mut().zip(&like.tasks) {
        if t.name == l.name.trim() {
            t.name = l.name.clone();
        }
    }
    // A project without a name is written with the name an import gives one.
    if p.name == like.name.trim() || (like.name.trim().is_empty() && p.name == "Imported project") {
        p.name = like.name.clone();
    }
    // MSPDI holds an assignment's work and units, not how long it lasts: that is worked out again
    // as work ÷ units (or the task's duration when the work fits it), which can be a minute from
    // what the project held when its work was rounded to the minute.
    for (a, l) in p.assignments.iter_mut().zip(&like.assignments) {
        if a.units == l.units && a.work == l.work && (a.duration - l.duration).abs() <= 1 {
            a.duration = l.duration;
        }
    }
    // A resource's own weekday the same as the project's reads back as following the project's.
    for (r, l) in p.resources.iter_mut().zip(&like.resources) {
        for (i, (day, want)) in r.calendar.week.iter_mut().zip(&l.calendar.week).enumerate() {
            if day.is_none() && want.as_deref() == Some(like.calendar.weekday_times(i)) {
                *day = want.clone();
            }
        }
    }
    p.next_id = like.next_id;
    p.next_resource_id = like.next_resource_id;
    p.columns = like.columns.clone();
    p.options.units_format = like.options.units_format;
    p
}

/// The first part of two projects that differs, to read a failure by.
fn difference(a: &Project, b: &Project) -> String {
    let parts = |p: &Project| -> Vec<String> {
        let mut v = vec![format!("{:?} {:?} {:?} {:?}", p.name, p.start, p.options, p.currency), format!("{:?}", p.calendar)];
        v.extend(p.tasks.iter().map(|t| format!("{t:?}")));
        v.extend(p.resources.iter().map(|r| format!("{r:?}")));
        v.extend(p.assignments.iter().map(|x| format!("{x:?}")));
        v
    };
    let (pa, pb) = (parts(a), parts(b));
    pa.iter()
        .zip(&pb)
        .find(|(x, y)| x != y)
        .map_or_else(|| format!("{} parts vs {}", pa.len(), pb.len()), |(x, y)| format!("\n  got  {x}\n  want {y}"))
}

/// The first line two files differ on, with the lines before it.
fn first_line_difference(a: &str, b: &str) -> String {
    let (la, lb): (Vec<&str>, Vec<&str>) = (a.lines().collect(), b.lines().collect());
    let i = la.iter().zip(&lb).position(|(x, y)| x != y).unwrap_or(la.len().min(lb.len()));
    let from = i.saturating_sub(12);
    format!("line {}:\n{}\n  got  {:?}\n  want {:?}", i + 1, lb[from..i].join("\n"), la.get(i), lb.get(i))
}

fn round_trip(s0: &Session) -> Result<(), TestCaseError> {
    let x0 = s0.export(Format::Mspdi).unwrap();
    let s1 = Session::import(Format::Mspdi, x0.as_bytes()).unwrap();
    let report = s1.import_report().unwrap();
    prop_assert!(report.lines.is_empty(), "{}\n{x0}", report.text());
    let back = comparable(s1.project.clone(), &s0.project);
    prop_assert!(back == s0.project, "export → import is the same project: {}\n{x0}", difference(&back, &s0.project));
    let x1 = s1.export(Format::Mspdi).unwrap();
    prop_assert!(x1 == x0, "export → import → export is byte for byte the same; first difference: {}", first_line_difference(&x1, &x0));
    let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
    prop_assert_eq!(&s2.project, &s1.project, "import → export → import is the same project");
    Ok(())
}

proptest! {
    #![proptest_config(ProptestConfig { cases: 200, ..ProptestConfig::default() })]

    #[test]
    fn generated_projects_round_trip_exactly(seed in any::<u64>()) {
        round_trip(&generated(seed))?;
    }
}

/// Plans that once didn't round trip: an assignment's duration a minute from its work ÷ units
/// (53 minutes at 50%; an effort-driven Fixed Work task's work shared out rounded), material
/// and cost resources, and every kind of text.
#[test]
fn plans_that_once_failed_round_trip() {
    for seed in [12171069806144644276, 12171255513429957426, 16235999543351105870, 2118350416455198788, 9653999539, 5239053456218871172] {
        round_trip(&generated(seed)).unwrap();
    }
}

/// A resource weekday set to the project's own times (Saturday off, when the project has
/// Saturdays off) is written as following the project calendar, as the importer reads one: the
/// resource has no calendar of its own after the round trip, and from then on the project
/// imports and exports unchanged.
#[test]
fn a_resource_weekday_like_the_project_s() {
    let mut p = Project::new("Weekdays", Date::from_ymd(2026, 3, 2).unwrap());
    let t = p.add_task("Build", 960).unwrap();
    let r = p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
    p.set_resource_weekday_times(r, 5, Some(vec![])).unwrap();
    p.assign(t, r, None).unwrap();
    let s0 = Session::new(p);
    let x0 = s0.export(Format::Mspdi).unwrap();
    assert!(!x0.contains("<CalendarUID>2</CalendarUID>"), "{x0}");
    let s1 = Session::import(Format::Mspdi, x0.as_bytes()).unwrap();
    assert!(s1.project.resources[0].calendar.is_empty());
    let x1 = s1.export(Format::Mspdi).unwrap();
    let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
    assert_eq!(s2.project, s1.project);
    assert_eq!(s2.export(Format::Mspdi).unwrap(), x1);
}

#[test]
fn the_import_fixtures_round_trip() {
    let dir = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../interchange/tests/fixtures");
    let mut seen = 0;
    for entry in std::fs::read_dir(&dir).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_none_or(|e| e != "xml") {
            continue;
        }
        seen += 1;
        let s1 = Session::import_file(&path, None).unwrap();
        let x1 = s1.export(Format::Mspdi).unwrap();
        let s2 = Session::import(Format::Mspdi, x1.as_bytes()).unwrap();
        assert_eq!(s2.project, s1.project, "{}: import → export → import", path.display());
        assert!(s2.import_report().unwrap().lines.is_empty(), "{}: {}", path.display(), s2.import_report().unwrap().text());
        assert_eq!(s2.export(Format::Mspdi).unwrap(), x1, "{}: export → import → export", path.display());
    }
    assert!(seen >= 2, "the fixtures were found");
}

#[test]
fn a_project_that_cannot_be_scheduled_still_exports_without_dates() {
    let mut p = Project::new("Loop", Date::from_ymd(2026, 3, 2).unwrap());
    let a = p.add_task("A", 480).unwrap();
    let b = p.add_task("B", 480).unwrap();
    p.set_preds(b, vec![Link { pred: a, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days }]).unwrap();
    // A loop only a damaged file could hold.
    p.task_mut(a).unwrap().preds.push(Link { pred: b, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days });
    let s = Session::new(p);
    assert!(s.schedule.is_err());
    let x = s.export(Format::Mspdi).unwrap();
    assert!(!x.contains("<Start>") && !x.contains("<TotalSlack>"), "{x}");
    assert!(x.contains("<PredecessorUID>2</PredecessorUID>"));
}
