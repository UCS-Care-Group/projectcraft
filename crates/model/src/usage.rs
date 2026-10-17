//! Resource usage and overallocation: each work resource's work, day by day, and the days it is
//! **overallocated**: given more work in a working day than its Max Units allow on its calendar
//! that day (Max Units × the day's working time), as Microsoft Project's resource workloads
//! (<https://support.microsoft.com/en-US/project/view-resource-workloads-and-availability-in-project-desktop>),
//! counted day by day.
//!
//! Work contours are flat: an assignment's work is spread over the working minutes it spans, each
//! day taking its share (rounded so the days add up to the assignment's work exactly). A work
//! assignment worked on its resource's calendar spans what the schedule says
//! ([`Schedule::assignments`]); any other runs from its task's start for its own duration.
//!
//! Materials and cost resources have no calendar and no Max Units: they are never overallocated
//! and are not listed here.
//!
//! Worked out from a project and its schedule, never stored; the UI shows it. Bounded: a plan
//! whose assignments span more than [`MAX_USAGE_DAYS`] days in all is refused, not walked.

use std::borrow::Cow;
use std::collections::{BTreeMap, BTreeSet};

use crate::{Calendar, Date, FULL_TIME, MAX_MINUTES, ModelError, Moment, Project, ResourceId, ResourceType, Schedule, TaskId, Units};

/// Most days walked over every assignment together: far more than any real plan's resources
/// work (300 assignments of a year each is about 110,000), so a hostile file costs a bounded time.
pub const MAX_USAGE_DAYS: usize = 5_000_000;

/// One work resource's work, day by day.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResourceUsage {
    pub id: ResourceId,
    /// Work on each day it works, in working minutes, all its assignments together.
    pub days: BTreeMap<Date, i64>,
    /// Each assignment's work by day, by task, in the project's assignment order.
    pub assignments: Vec<(TaskId, BTreeMap<Date, i64>)>,
    /// The days it is overallocated.
    pub over: BTreeSet<Date>,
}

impl ResourceUsage {
    /// Whether it is overallocated on any day.
    pub fn is_overallocated(&self) -> bool {
        !self.over.is_empty()
    }

    /// Its total work, in working minutes.
    pub fn work(&self) -> i64 {
        self.days.values().fold(0i64, |a, w| a.saturating_add(*w))
    }
}

/// Every work resource's usage, in Resource Sheet order.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Usage {
    pub resources: Vec<ResourceUsage>,
}

impl Usage {
    /// A work resource's usage; `None` for a material, a cost resource or an unknown one.
    pub fn resource(&self, id: ResourceId) -> Option<&ResourceUsage> {
        self.resources.iter().find(|r| r.id == id)
    }

    /// Whether a resource is overallocated on any day; never a material or cost resource.
    pub fn is_overallocated(&self, id: ResourceId) -> bool {
        self.resource(id).is_some_and(ResourceUsage::is_overallocated)
    }

    /// The tasks with an overallocated assignment: one whose resource is overallocated on a day
    /// the assignment works.
    pub fn overallocated_tasks(&self) -> BTreeSet<TaskId> {
        let mut out = BTreeSet::new();
        for r in self.resources.iter().filter(|r| r.is_overallocated()) {
            for (task, days) in &r.assignments {
                if days.keys().any(|d| r.over.contains(d)) {
                    out.insert(*task);
                }
            }
        }
        out
    }
}

/// The working minutes of `times` between minutes `lo` and `hi` of the day.
fn clipped(times: &[crate::WorkingTime], lo: u16, hi: u16) -> i64 {
    times.iter().map(|t| i64::from(t.to.min(hi).saturating_sub(t.from.max(lo)))).sum()
}

/// Walks a calendar from `from`, day by day: the working minutes of each day up to `to` (if
/// given) and no more than `budget` minutes in all. Counts each day it steps against `steps`;
/// `None` when that runs out.
fn walk(cal: &Calendar, from: Moment, to: Option<Moment>, budget: i64, steps: &mut usize) -> Option<Vec<(Date, i64)>> {
    let mut out = Vec::new();
    let mut left = budget.clamp(0, MAX_MINUTES);
    let mut d = from.date;
    while left > 0 {
        if to.is_some_and(|t| d > t.date) {
            break;
        }
        *steps = steps.checked_sub(1)?;
        let lo = if d == from.date { from.minute } else { 0 };
        let hi = match to {
            Some(t) if t.date == d => t.minute,
            _ => u16::MAX,
        };
        let m = clipped(cal.times_on(d), lo, hi).min(left);
        if m > 0 {
            out.push((d, m));
            left -= m;
        }
        d = d.add_days(1)?;
    }
    Some(out)
}

/// Spreads `work` over the days in proportion to their minutes, rounding so they add up to it.
fn spread(work: i64, minutes: &[(Date, i64)]) -> BTreeMap<Date, i64> {
    let total: i128 = minutes.iter().map(|(_, m)| i128::from(*m)).sum();
    let mut out = BTreeMap::new();
    if total <= 0 {
        return out;
    }
    let (mut done, mut given) = (0i128, 0i64);
    for (d, m) in minutes {
        done += i128::from(*m);
        let upto = i64::try_from(i128::from(work.max(0)) * done / total).unwrap_or(i64::MAX);
        let w = upto.saturating_sub(given);
        given = upto;
        if w > 0 {
            *out.entry(*d).or_insert(0) += w;
        }
    }
    out
}

/// Each work resource's usage over `s`, the schedule of `p`. An error when the assignments span
/// more than [`MAX_USAGE_DAYS`] days in all.
pub fn usage(p: &Project, s: &Schedule) -> Result<Usage, ModelError> {
    let too_big = || ModelError::Invalid(format!("the resources' work spans more than {MAX_USAGE_DAYS} days: too much to show day by day"));
    let mut steps = MAX_USAGE_DAYS;
    let off = p.resources_on_the_project_calendar();
    let mut resources = Vec::new();
    // Each resource's assignments, in the project's order: one pass, however many resources.
    let mut by_resource: std::collections::HashMap<ResourceId, Vec<&crate::Assignment>> = std::collections::HashMap::new();
    for a in &p.assignments {
        by_resource.entry(a.resource).or_default().push(a);
    }
    for r in p.resources.iter().filter(|r| r.kind == ResourceType::Work) {
        // The calendar it is scheduled on: its own, or the project's (no calendar of its own, or
        // one a file left with no working weekday).
        let cal: Cow<'_, Calendar> =
            if r.calendar.is_empty() || off.contains(&r.name.as_str()) { Cow::Borrowed(&p.calendar) } else { Cow::Owned(r.calendar.on(&p.calendar)) };
        let mut u = ResourceUsage { id: r.id, days: BTreeMap::new(), assignments: Vec::new(), over: BTreeSet::new() };
        for a in by_resource.get(&r.id).into_iter().flatten() {
            let Units::Work(_) = a.units else { continue };
            let Some(dates) = s.tasks.get(&a.task) else { continue };
            let minutes = match s.assignments.get(&(a.task, r.id)) {
                Some((from, to)) => walk(&cal, *from, Some(*to), MAX_MINUTES, &mut steps),
                None => walk(&cal, dates.start_at, None, a.duration, &mut steps),
            }
            .ok_or_else(too_big)?;
            let days = spread(a.work, &minutes);
            for (d, w) in &days {
                let e = u.days.entry(*d).or_insert(0);
                *e = e.saturating_add(*w);
            }
            u.assignments.push((a.task, days));
        }
        // Over when work × 100% > Max Units × the day's working time.
        for (d, w) in &u.days {
            let room = i128::from(r.max_units) * i128::from(cal.minutes_on(*d));
            if i128::from(*w) * i128::from(FULL_TIME) > room {
                u.over.insert(*d);
            }
        }
        resources.push(u);
    }
    Ok(Usage { resources })
}

impl Project {
    /// Each work resource's usage over `s`, this project's schedule ([`usage`]).
    pub fn usage(&self, s: &Schedule) -> Result<Usage, ModelError> {
        usage(self, s)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    #[test]
    fn work_is_spread_by_minutes_and_adds_up_exactly() {
        let days = [(d("2026-01-05"), 240), (d("2026-01-06"), 480), (d("2026-01-07"), 240)];
        let w = spread(1000, &days);
        assert_eq!(w.values().sum::<i64>(), 1000);
        assert_eq!(w[&d("2026-01-06")], 500);
        assert!(spread(100, &[]).is_empty());
        assert_eq!(spread(-5, &days).values().sum::<i64>(), 0);
    }

    #[test]
    fn a_walk_clips_its_first_and_last_days_and_stops_at_its_budget() {
        let cal = Calendar::default();
        let mon = d("2026-01-05");
        let from = Moment { date: mon, minute: 13 * 60 };
        let to = Moment { date: d("2026-01-06"), minute: 10 * 60 };
        let mut steps = 100;
        assert_eq!(walk(&cal, from, Some(to), MAX_MINUTES, &mut steps), Some(vec![(mon, 240), (d("2026-01-06"), 120)]));
        // Over the weekend: Friday afternoon and Monday.
        let fri = Moment { date: d("2026-01-09"), minute: 13 * 60 };
        assert_eq!(walk(&cal, fri, None, 600, &mut steps), Some(vec![(d("2026-01-09"), 240), (d("2026-01-12"), 360)]));
        let mut none = 1;
        assert_eq!(walk(&cal, fri, None, 600, &mut none), None, "out of steps");
    }

    proptest::proptest! {
        /// Plans whose work resources have calendars of their own, Max Units and assignments on
        /// overlapping tasks: never a panic, each assignment's days add up to its work, a
        /// resource's days are its assignments' together, and a day is over exactly when its work
        /// is more than Max Units × that day's working time.
        #[test]
        fn usage_adds_up_and_flags_only_days_over_max_units(
            durations in proptest::collection::vec(0i64..3000, 1..8),
            preds in proptest::collection::vec((0usize..8, 0u32..9), 0..6),
            leave in proptest::collection::vec((0usize..3, 0i64..30), 0..8),
            mornings in proptest::collection::vec((0usize..3, 0usize..7), 0..6),
            max in proptest::collection::vec(0u32..40_000, 3),
            assigned in proptest::collection::vec((0usize..8, 0usize..3, 1u32..30_000), 0..12),
        ) {
            let mut p = Project::new("t", d("2026-01-05"));
            let ids: Vec<TaskId> = durations.iter().enumerate().map(|(i, m)| p.add_task(format!("T{i}"), *m).unwrap()).collect();
            for (succ, pred) in preds {
                if let Some(&id) = ids.get(succ) {
                    let _ = p.set_preds(id, vec![crate::Link { pred: TaskId(pred), kind: crate::LinkKind::FS, lag: 0, lag_unit: crate::DurationUnit::Days }]);
                }
            }
            let people: Vec<ResourceId> = ["Ann", "Bob", "Cat"].iter().map(|n| p.add_resource(Some(n), ResourceType::Work, None).unwrap()).collect();
            for (r, m) in people.iter().zip(&max) {
                p.resource_mut(*r).unwrap().max_units = *m;
            }
            for (r, day) in leave {
                let _ = p.set_resource_exception(people[r], d("2026-01-05").add_days(day).unwrap(), vec![]);
            }
            for (r, weekday) in mornings {
                let _ = p.set_resource_weekday_times(people[r], weekday, Some(vec![crate::parse_working_time("8:00-12:00").unwrap()]));
            }
            let steel = p.add_resource(Some("Steel"), ResourceType::Material, None).unwrap();
            for (row, r, units) in assigned {
                if let Some(&id) = ids.get(row) {
                    let _ = p.assign(id, people[r], Some(Units::Work(units)));
                    let _ = p.assign(id, steel, None);
                }
            }
            let Ok(s) = crate::schedule(&p) else { return Ok(()) };
            let u = usage(&p, &s).unwrap();
            proptest::prop_assert!(u.resource(steel).is_none());
            for r in &u.resources {
                let mut sum = BTreeMap::new();
                for (task, days) in &r.assignments {
                    let a = p.assignments_of(*task).find(|a| a.resource == r.id).unwrap();
                    let total: i64 = days.values().sum();
                    proptest::prop_assert!(total == a.work, "{:?} on {:?}: {} of {}", r.id, task, total, a.work);
                    for (day, w) in days {
                        *sum.entry(*day).or_insert(0i64) += w;
                    }
                }
                proptest::prop_assert_eq!(&sum, &r.days);
                let cal = p.resource_calendar(r.id).unwrap();
                let max_units = p.resource(r.id).unwrap().max_units;
                for (day, w) in &r.days {
                    let over = i128::from(*w) * i128::from(FULL_TIME) > i128::from(max_units) * i128::from(cal.minutes_on(*day));
                    proptest::prop_assert_eq!(over, r.over.contains(day));
                }
            }
        }
    }
}
