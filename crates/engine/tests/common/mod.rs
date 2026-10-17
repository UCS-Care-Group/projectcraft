//! What the export round-trip tests share: projects generated from a seed, with every field the
//! exports write and hostile text.
#![allow(dead_code)]

use projectcraft_engine::Session;
use projectcraft_engine::model::{
    AccrueAt, Constraint, Currency, Date, DurationUnit, Link, LinkKind, Outline, Project, ResourceId, ResourceType, TaskId, TaskType, Units,
    WorkingTime,
};

/// A small linear congruential generator: the same seed, the same project.
pub struct Lcg(pub u64);

impl Lcg {
    pub fn next(&mut self) -> u64 {
        self.0 = self.0.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
        self.0 >> 33
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }

    pub fn signed(&mut self, n: i64) -> i64 {
        self.below(2 * n as u64 + 1) as i64 - n
    }

    pub fn chance(&mut self, percent: u64) -> bool {
        self.below(100) < percent
    }

    pub fn pick<'a, T>(&mut self, items: &'a [T]) -> &'a T {
        &items[self.below(items.len() as u64) as usize]
    }
}

/// Text with what XML must escape, non-ASCII letters and a character XML can't hold at all.
pub fn text(rng: &mut Lcg) -> String {
    const BITS: [&str; 12] = ["Dig", "&", "<b>", "\"quoted\"", "it's", "café", "Ω", "  ", "\u{1}", "日本", "]]>", "\t"];
    let n = 1 + rng.below(4);
    let s: String = (0..n).map(|_| *rng.pick(&BITS)).collect::<Vec<_>>().join(" ");
    s.chars().filter(|c| !c.is_control()).collect()
}

pub fn times(rng: &mut Lcg) -> Vec<WorkingTime> {
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
pub fn generated(seed: u64) -> Session {
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
            6 => Constraint::AsLateAsPossible,
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
