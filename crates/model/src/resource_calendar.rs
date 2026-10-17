//! Resource calendars: each work resource's own working time, **based on the project calendar**,
//! as Microsoft Project's resource calendars are based on the Standard calendar. A resource keeps
//! only what differs: weekdays with working times of their own (a part-time week) and exception
//! days of its own (leave, a short day). Everything else follows the project calendar, so a
//! holiday added there reaches every resource. On a day, the resource's own exception comes first,
//! then the project calendar's, then the resource's own weekday, then the project's.
//!
//! The scheduler works each assignment on its resource's calendar; see `schedule.rs`. Materials
//! and costs have no calendar.
//!
//! A resource's own times are worked as they are, inside or outside the project calendar's
//! working time: a Saturday the project doesn't work, a working day on a project holiday, hours
//! before or after the project's day. Microsoft Project schedules an assignment on its resource's
//! calendar in those times too.
//!
//! Also here: walking a calendar by working minutes between [`Moment`]s, quickly however many
//! exceptions it has ([`Counted`], [`ResView`]), which the scheduler uses to move between the
//! project calendar and a resource's; and the plan's working time ([`plan_time`]): the project
//! calendar with the resource calendars' time outside it added, the line the scheduler counts on.

use std::borrow::Cow;
use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

use crate::{
    Calendar, DEFAULT_WORKING_TIMES, Date, ExceptionFile, MAX_EXCEPTIONS, ModelError, Moment, Project, ResourceId, ResourceType, WorkingTime,
    check_working_times, own_times,
};

/// A work resource's working time where it differs from the project calendar.
///
/// Written to the file as `{"week": [null, null, null, null, [{"from": 480, "to": 720}], [], null],
/// "exceptions": [{"date": "2026-01-12", "working": false}]}`: `null` follows the project
/// calendar, an empty list is a day off. A resource without one has none.
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(into = "ResourceCalendarFile", try_from = "ResourceCalendarFile")]
pub struct ResourceCalendar {
    /// Each weekday's own working times, Monday first: `None` follows the project calendar, an
    /// empty list makes the day non-working.
    pub week: [Option<Vec<WorkingTime>>; 7],
    /// Days of its own: their working times, none for a day off (leave).
    pub exceptions: BTreeMap<Date, Vec<WorkingTime>>,
}

#[derive(Serialize, Deserialize)]
struct ResourceCalendarFile {
    #[serde(default, skip_serializing_if = "follows_the_project")]
    week: [Option<Vec<WorkingTime>>; 7],
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    exceptions: Vec<ExceptionFile>,
}

fn follows_the_project(week: &[Option<Vec<WorkingTime>>; 7]) -> bool {
    week.iter().all(Option::is_none)
}

impl TryFrom<ResourceCalendarFile> for ResourceCalendar {
    type Error = ModelError;

    /// Refuses working times out of order, overlapping or outside the day, times on a day off,
    /// and more than [`MAX_EXCEPTIONS`] exceptions.
    fn try_from(f: ResourceCalendarFile) -> Result<ResourceCalendar, ModelError> {
        for t in f.week.iter().flatten() {
            check_working_times(t)?;
        }
        if f.exceptions.len() > MAX_EXCEPTIONS {
            return Err(ModelError::Invalid(format!("a resource calendar holds at most {MAX_EXCEPTIONS} exceptions")));
        }
        let mut exceptions = BTreeMap::new();
        for e in f.exceptions {
            check_working_times(&e.times)?;
            let times = match (e.working, e.times.is_empty()) {
                (false, false) => return Err(ModelError::Invalid(format!("the non-working exception on {} has working times", e.date))),
                (false, true) => Vec::new(),
                (true, true) => DEFAULT_WORKING_TIMES.to_vec(),
                (true, false) => e.times,
            };
            exceptions.insert(e.date, times);
        }
        Ok(ResourceCalendar { week: f.week, exceptions })
    }
}

impl From<ResourceCalendar> for ResourceCalendarFile {
    fn from(c: ResourceCalendar) -> ResourceCalendarFile {
        let exceptions =
            c.exceptions.into_iter().map(|(date, times)| ExceptionFile { date, working: !times.is_empty(), times: own_times(times) }).collect();
        ResourceCalendarFile { week: c.week, exceptions }
    }
}

impl ResourceCalendar {
    /// Nothing of its own: the project calendar as it is.
    pub fn is_empty(&self) -> bool {
        follows_the_project(&self.week) && self.exceptions.is_empty()
    }

    /// Weekday `i`'s working times: its own, or the project calendar's (`base`).
    pub fn weekday_times<'a>(&'a self, base: &'a Calendar, i: usize) -> &'a [WorkingTime] {
        match self.week.get(i) {
            Some(Some(t)) => t,
            _ => base.weekday_times(i),
        }
    }

    /// The working times on `d`: its own exception that day, else the project calendar's
    /// exception, else its weekday's.
    pub fn times_on<'a>(&'a self, base: &'a Calendar, d: Date) -> &'a [WorkingTime] {
        match self.exceptions.get(&d) {
            Some(t) => t,
            None if base.exceptions.contains_key(&d) => base.times_on(d),
            None => self.weekday_times(base, usize::from(d.weekday())),
        }
    }

    /// Whether any weekday is worked.
    pub fn works_a_weekday(&self, base: &Calendar) -> bool {
        (0..7).any(|i| !self.weekday_times(base, i).is_empty())
    }

    /// Whether all its own time lies within the project calendar's working time: its weekdays
    /// within the project's, each exception within the project's times that day.
    pub fn within(&self, base: &Calendar) -> bool {
        (0..7).all(|i| self.week.get(i).and_then(Option::as_ref).is_none_or(|t| inside(t, base.weekday_times(i))))
            && self.exceptions.iter().all(|(d, t)| inside(t, base.times_on(*d)))
    }

    /// The calendar it makes on top of `base` (the project calendar): its own exceptions win, then
    /// the project's exceptions, then its own weekdays. A copy of the project calendar: for one
    /// resource at a time (the scheduler never makes one).
    pub fn on(&self, base: &Calendar) -> Calendar {
        let mut c = base.clone();
        for i in 0..7 {
            if let Some(Some(t)) = self.week.get(i) {
                let t = t.clone();
                if let (Some(w), Some(wt)) = (c.working.get_mut(i), c.week_times.get_mut(i)) {
                    *w = !t.is_empty();
                    *wt = own_times(t);
                }
            }
        }
        for (d, t) in &self.exceptions {
            let t = t.clone();
            c.exceptions.insert(*d, !t.is_empty());
            c.exception_times.remove(d);
            let t = own_times(t);
            if !t.is_empty() {
                c.exception_times.insert(*d, t);
            }
        }
        c
    }
}

/// Whether every working time in `a` lies within one of `b`'s.
fn inside(a: &[WorkingTime], b: &[WorkingTime]) -> bool {
    a.iter().all(|x| b.iter().any(|y| y.from <= x.from && x.to <= y.to))
}

/// The working times of both lists together, in order, overlapping or touching ones merged.
fn union(a: &[WorkingTime], b: &[WorkingTime]) -> Vec<WorkingTime> {
    let mut all: Vec<WorkingTime> = a.iter().chain(b).copied().collect();
    all.sort_by_key(|t| (t.from, t.to));
    let mut out: Vec<WorkingTime> = Vec::with_capacity(all.len());
    for t in all {
        match out.last_mut() {
            Some(last) if t.from <= last.to => last.to = last.to.max(t.to),
            _ => out.push(t),
        }
    }
    out
}

/// The plan's working time: the project calendar with the time of `own` (resource calendars on
/// top of it) that lies outside it added, weekday by weekday and on each exception day of their
/// own. `None` when each works only within the project calendar's working time, as most do: then
/// the project calendar is the plan's working time. The scheduler counts its offsets on it, so
/// every minute any of these calendars works has an offset. Built in time proportional to their
/// own weekdays and exceptions.
pub(crate) fn plan_time(base: &Calendar, own: &[&ResourceCalendar]) -> Option<Calendar> {
    if own.iter().all(|o| o.within(base)) {
        return None;
    }
    let mut c = base.clone();
    for i in 0..7 {
        let t = own.iter().filter_map(|o| o.week.get(i).and_then(Option::as_ref)).fold(base.weekday_times(i).to_vec(), |t, o| union(&t, o));
        if let (Some(w), Some(wt)) = (c.working.get_mut(i), c.week_times.get_mut(i)) {
            *w = !t.is_empty();
            *wt = own_times(t);
        }
    }
    // An exception day of its own: what the plan works that day already (the project's exception,
    // or the weekday with every resource's own times), with the exception's times added.
    for o in own {
        for (d, t) in &o.exceptions {
            let now = c.times_on(*d);
            let both = union(now, t);
            if both != now {
                c.exceptions.insert(*d, !both.is_empty());
                c.exception_times.remove(d);
                let both = own_times(both);
                if !both.is_empty() {
                    c.exception_times.insert(*d, both);
                }
            }
        }
    }
    Some(c)
}

impl Project {
    /// A resource's calendar as the scheduler uses it ([`ResourceCalendar::on`]). A material or
    /// cost resource has the project calendar.
    pub fn resource_calendar(&self, id: ResourceId) -> Result<Calendar, ModelError> {
        let r = self.resource(id)?;
        Ok(if r.kind == ResourceType::Work { r.calendar.on(&self.calendar) } else { self.calendar.clone() })
    }

    /// The calendar of a work resource, to change; refused for a material or cost resource.
    fn own_calendar(&mut self, id: ResourceId) -> Result<&mut ResourceCalendar, ModelError> {
        let r = self.resource(id)?;
        if r.kind != ResourceType::Work {
            return Err(ModelError::Invalid(format!(
                "{} is a {} resource: only work resources have calendars",
                r.name,
                r.kind.name().to_lowercase()
            )));
        }
        Ok(&mut self.resource_mut(id)?.calendar)
    }

    /// Applies `edit` to a work resource's calendar, refusing a week left with no working day
    /// within the project calendar's working time.
    fn edit_calendar(&mut self, id: ResourceId, edit: impl FnOnce(&mut ResourceCalendar) -> Result<(), ModelError>) -> Result<(), ModelError> {
        let mut c = self.own_calendar(id)?.clone();
        edit(&mut c)?;
        if !c.works_a_weekday(&self.calendar) {
            return Err(ModelError::Invalid(NO_WORKING_WEEKDAY.into()));
        }
        *self.own_calendar(id)? = c;
        Ok(())
    }

    /// Refuses a project calendar that leaves a work resource with no working weekday (its own
    /// days off and the project's together), naming the first: for a project-calendar edit, so a
    /// resource is never left with no working time.
    pub fn check_resource_weeks(&self) -> Result<(), ModelError> {
        match self.resources.iter().find(|r| r.kind == ResourceType::Work && !r.calendar.is_empty() && !r.calendar.works_a_weekday(&self.calendar)) {
            Some(r) => Err(ModelError::Invalid(format!(
                "{} would have no working weekday: none of their own, and none left on the project calendar",
                r.name
            ))),
            None => Ok(()),
        }
    }

    /// The work resources whose calendar leaves them no working weekday (only a file can hold
    /// that: the commands refuse it), by name. The scheduler works them on the project calendar.
    pub fn resources_on_the_project_calendar(&self) -> Vec<&str> {
        self.resources
            .iter()
            .filter(|r| r.kind == ResourceType::Work && !r.calendar.is_empty() && !r.calendar.works_a_weekday(&self.calendar))
            .map(|r| r.name.as_str())
            .collect()
    }

    /// Sets which weekdays a work resource works (Monday first). A weekday made to match the
    /// project calendar follows it again; one it stops working has none; one it starts working
    /// that the project doesn't work gets the default times. The others are left as they are.
    pub fn set_resource_working(&mut self, id: ResourceId, working: [bool; 7]) -> Result<(), ModelError> {
        let base = self.calendar.working;
        let now: Vec<bool> = {
            let r = self.resource(id)?;
            (0..7).map(|i| !r.calendar.weekday_times(&self.calendar, i).is_empty()).collect()
        };
        self.edit_calendar(id, |c| {
            for (i, own) in c.week.iter_mut().enumerate() {
                let (was, is) = (now.get(i).copied().unwrap_or(false), working.get(i).copied().unwrap_or(false));
                if was != is {
                    *own = match (is, base.get(i).copied().unwrap_or(false)) {
                        (is, project) if is == project => None,
                        (true, _) => Some(DEFAULT_WORKING_TIMES.to_vec()),
                        (false, _) => Some(Vec::new()),
                    };
                }
            }
            Ok(())
        })
    }

    /// Sets a work resource's working times on weekday `i` (0 is Monday): `Some` of its own (none
    /// is a day off), `None` to follow the project calendar again.
    pub fn set_resource_weekday_times(&mut self, id: ResourceId, i: usize, times: Option<Vec<WorkingTime>>) -> Result<(), ModelError> {
        if let Some(t) = &times {
            check_working_times(t)?;
        }
        self.edit_calendar(id, |c| {
            *c.week.get_mut(i).ok_or_else(|| ModelError::Invalid(format!("no weekday {i}")))? = times;
            Ok(())
        })
    }

    /// Makes `date` an exception on a work resource's calendar: these working times, or none for a
    /// day off (leave). Replaces any it already had on that day.
    pub fn set_resource_exception(&mut self, id: ResourceId, date: Date, times: Vec<WorkingTime>) -> Result<(), ModelError> {
        check_working_times(&times)?;
        self.edit_calendar(id, |c| {
            if c.exceptions.len() >= MAX_EXCEPTIONS && !c.exceptions.contains_key(&date) {
                return Err(ModelError::Invalid(format!("a calendar holds at most {MAX_EXCEPTIONS} exceptions")));
            }
            c.exceptions.insert(date, times);
            Ok(())
        })
    }

    /// Removes a work resource's own exception on `date`: the day follows the project calendar.
    pub fn remove_resource_exception(&mut self, id: ResourceId, date: Date) -> Result<(), ModelError> {
        self.edit_calendar(id, |c| {
            c.exceptions.remove(&date).map(|_| ()).ok_or_else(|| ModelError::Invalid(format!("there is no exception on {date}")))
        })
    }
}

/// Why a resource calendar edit is refused when it would leave no working weekday.
const NO_WORKING_WEEKDAY: &str = "a calendar needs at least one working weekday";

/// The last date a calendar walk reaches.
fn last_date() -> Date {
    Date::from_ymd(9999, 12, 31).unwrap_or(Date::EPOCH)
}

/// Weekday `i`'s days on or after `from` and before `to`: whole weeks, and the rest.
fn weekdays_between(from: Date, to: Date, i: usize) -> i64 {
    let n = to.days_since(from);
    if n <= 0 {
        return 0;
    }
    let ahead = (i as i64 - i64::from(from.weekday())).rem_euclid(7);
    n / 7 + i64::from(ahead < n % 7)
}

/// A calendar the scheduler walks by working minutes.
pub(crate) trait Work {
    /// Working minutes on or after `from` and before `to`; 0 when `to` is not after `from`.
    fn minutes_between(&self, from: Date, to: Date) -> i64;
    /// The working times on `d`.
    fn times_on(&self, d: Date) -> Cow<'_, [WorkingTime]>;
    /// Working minutes in a week of plain weekdays: a guess's scale, never 0 when any is worked.
    fn per_week(&self) -> i64;
}

impl Work for Calendar {
    fn minutes_between(&self, from: Date, to: Date) -> i64 {
        Calendar::minutes_between(self, from, to)
    }

    fn times_on(&self, d: Date) -> Cow<'_, [WorkingTime]> {
        Cow::Borrowed(Calendar::times_on(self, d))
    }

    fn per_week(&self) -> i64 {
        (0..7).map(|i| self.weekday_minutes(i)).sum()
    }
}

/// The project calendar counted quickly, built once per schedule: its weekly pattern and its
/// exceptions in date order with their corrections (minutes worked less the weekday's) added up,
/// so the working minutes between two dates are two binary searches, however many exceptions.
pub(crate) struct Counted<'a> {
    pub cal: &'a Calendar,
    week: [i64; 7],
    dates: Vec<Date>,
    /// `before[k]`: the corrections of `dates[..k]`, added up.
    before: Vec<i64>,
    /// `on_weekday[k][i]`: how many of `dates[..k]` are on weekday `i`.
    on_weekday: Vec<[i64; 7]>,
}

impl<'a> Counted<'a> {
    pub fn new(cal: &'a Calendar) -> Counted<'a> {
        let week: [i64; 7] = std::array::from_fn(|i| cal.weekday_minutes(i));
        let n = cal.exceptions.len();
        let (mut dates, mut before, mut on_weekday) = (Vec::with_capacity(n), Vec::with_capacity(n + 1), Vec::with_capacity(n + 1));
        let (mut sum, mut counts) = (0i64, [0i64; 7]);
        before.push(0);
        on_weekday.push(counts);
        for d in cal.exceptions.keys() {
            let wd = usize::from(d.weekday());
            sum = sum.saturating_add(cal.minutes_on(*d) - week.get(wd).copied().unwrap_or(0));
            if let Some(c) = counts.get_mut(wd) {
                *c += 1;
            }
            dates.push(*d);
            before.push(sum);
            on_weekday.push(counts);
        }
        Counted { cal, week, dates, before, on_weekday }
    }

    /// How many exceptions fall before `d`.
    fn rank(&self, d: Date) -> usize {
        self.dates.partition_point(|x| *x < d)
    }

    fn has_exception(&self, d: Date) -> bool {
        self.cal.exceptions.contains_key(&d)
    }
}

impl Work for Counted<'_> {
    fn minutes_between(&self, from: Date, to: Date) -> i64 {
        if to <= from {
            return 0;
        }
        let weekly: i64 = (0..7).map(|i| self.week.get(i).copied().unwrap_or(0).saturating_mul(weekdays_between(from, to, i))).sum();
        let at = |d: Date| self.before.get(self.rank(d)).copied().unwrap_or(0);
        weekly.saturating_add(at(to) - at(from))
    }

    fn times_on(&self, d: Date) -> Cow<'_, [WorkingTime]> {
        Cow::Borrowed(self.cal.times_on(d))
    }

    fn per_week(&self) -> i64 {
        self.week.iter().sum()
    }
}

/// A work resource's calendar as the scheduler walks it ([`ResourceCalendar::on`]), without a
/// copy of the project calendar: only what the resource's own weekdays and exceptions take away
/// from (or add to) the project's working time, counted the same quick way. Built from its own
/// calendar alone, in time proportional to that.
pub(crate) struct ResView<'a> {
    own: &'a ResourceCalendar,
    /// Each weekday's working times, and the minutes fewer than the project's (negative: more).
    week: [Vec<WorkingTime>; 7],
    cut: [i64; 7],
    per_week: i64,
    /// Its own exception days in order, and what each takes away beyond its weekday's cut, added up.
    dates: Vec<Date>,
    before: Vec<i64>,
}

impl<'a> ResView<'a> {
    pub fn new(base: &Counted<'_>, own: &'a ResourceCalendar) -> ResView<'a> {
        let week: [Vec<WorkingTime>; 7] = std::array::from_fn(|i| own.weekday_times(base.cal, i).to_vec());
        let cut: [i64; 7] =
            std::array::from_fn(|i| base.week.get(i).copied().unwrap_or(0) - week.get(i).map_or(0, |t| i64::from(crate::minutes_of(t))));
        let per_week = week.iter().map(|t| i64::from(crate::minutes_of(t))).sum();
        let (mut dates, mut before, mut sum) = (Vec::with_capacity(own.exceptions.len()), vec![0], 0i64);
        for (d, t) in &own.exceptions {
            let project = base.cal.times_on(*d);
            let taken = i64::from(crate::minutes_of(project)) - i64::from(crate::minutes_of(t));
            // A plain day of its weekday already has the weekday's cut counted.
            let counted = if base.has_exception(*d) { 0 } else { cut.get(usize::from(d.weekday())).copied().unwrap_or(0) };
            sum = sum.saturating_add(taken - counted);
            dates.push(*d);
            before.push(sum);
        }
        ResView { own, week, cut, per_week, dates, before }
    }

    #[cfg(test)]
    pub fn per_week(&self) -> i64 {
        self.per_week
    }
}

/// A resource's view with the project calendar it is based on: what the walks take.
pub(crate) struct OnProject<'r, 'a> {
    pub base: &'r Counted<'a>,
    pub view: &'r ResView<'a>,
}

impl Work for OnProject<'_, '_> {
    fn minutes_between(&self, from: Date, to: Date) -> i64 {
        if to <= from {
            return 0;
        }
        let (b, v) = (self.base, self.view);
        // One search each way in the project's exceptions serves the weekly pattern, the
        // corrections and the weekday counts alike.
        let (lo, hi) = (b.rank(from), b.rank(to));
        let (lo_c, hi_c) = (b.on_weekday.get(lo).copied().unwrap_or_default(), b.on_weekday.get(hi).copied().unwrap_or_default());
        let mut total = b.before.get(hi).copied().unwrap_or(0) - b.before.get(lo).copied().unwrap_or(0);
        for i in 0..7 {
            let plain = weekdays_between(from, to, i);
            let on_exceptions = hi_c.get(i).copied().unwrap_or(0) - lo_c.get(i).copied().unwrap_or(0);
            total = total
                .saturating_add(b.week.get(i).copied().unwrap_or(0).saturating_mul(plain))
                .saturating_sub(v.cut.get(i).copied().unwrap_or(0).saturating_mul(plain - on_exceptions));
        }
        let at = |d: Date| v.before.get(v.dates.partition_point(|x| *x < d)).copied().unwrap_or(0);
        total.saturating_sub(at(to) - at(from))
    }

    fn times_on(&self, d: Date) -> Cow<'_, [WorkingTime]> {
        match self.view.own.exceptions.get(&d) {
            Some(t) => Cow::Borrowed(t),
            None if self.base.has_exception(d) => Cow::Borrowed(self.base.cal.times_on(d)),
            None => Cow::Borrowed(self.view.week.get(usize::from(d.weekday())).map_or(&[][..], Vec::as_slice)),
        }
    }

    fn per_week(&self) -> i64 {
        self.view.per_week
    }
}

/// Working minutes in `times` before `minute` of the day.
fn minutes_before(times: &[WorkingTime], minute: u16) -> i64 {
    times.iter().map(|t| i64::from(minute.max(t.from).min(t.to).saturating_sub(t.from))).sum()
}

/// Working minutes on `w` from the start of `r` up to the moment `m` (negative before `r`).
pub(crate) fn count_to(w: &dyn Work, r: Date, m: Moment) -> i64 {
    let days = if m.date >= r { w.minutes_between(r, m.date) } else { -w.minutes_between(m.date, r) };
    days + minutes_before(&w.times_on(m.date), m.minute)
}

/// The moment `k` working minutes on `w` after the start of `r`: the start of the next working
/// minute (`start`), or the end of the one before. `None` when it has no working time left before
/// year 9999. A point before `r` is the start of `r`'s first working minute.
pub(crate) fn moment_at(w: &dyn Work, r: Date, k: i64, start: bool) -> Option<Moment> {
    moment_near(w, r, k, start, r)
}

/// [`moment_at`], searching from `near` (a day on or before the answer, such as where a walk
/// starts) rather than from `r`: a few steps for a task's length instead of the whole way.
pub(crate) fn moment_near(w: &dyn Work, r: Date, k: i64, start: bool, near: Date) -> Option<Moment> {
    let (k, start) = if start || k <= 0 { (k.max(0), true) } else { (k - 1, false) };
    // The working day holding minute k: the last day with no more than k working minutes before
    // it. Days run in order, so gallop out from a guess (a week's working time a week), then halve
    // the range; minutes_between is quick on any range.
    let before = |n: i64| r.add_days(n).map(|d| w.minutes_between(r, d));
    let max = last_date().days_since(r).max(0);
    let per_week = w.per_week();
    let lo = near.days_since(r).clamp(0, max);
    let lo = if before(lo)? <= k { lo } else { 0 };
    let ahead = k - before(lo)?;
    let mut step = if per_week > 0 { (ahead / per_week).saturating_add(2).saturating_mul(7) } else { 64 };
    let (mut lo, mut hi) = (lo, lo.saturating_add(step).min(max));
    while before(hi)? <= k {
        if hi >= max {
            // Minute k is on the last day, or past it: the calendar runs out.
            let last = r.add_days(max)?;
            let fits = before(max)?.saturating_add(i64::from(crate::minutes_of(&w.times_on(last)))) > k;
            return if fits { on_day(w, r, max, k, start) } else { None };
        }
        lo = hi;
        step = step.saturating_mul(2);
        hi = lo.saturating_add(step).min(max);
    }
    while hi - lo > 1 {
        let mid = lo + (hi - lo) / 2;
        if before(mid)? <= k { lo = mid } else { hi = mid }
    }
    on_day(w, r, lo, k, start)
}

/// Days a scan from a known count goes, either way, before it falls back to [`moment_near`]'s search.
const SCAN_DAYS: usize = 64;

/// [`moment_at`] from a day whose count is known: `before_near` working minutes lie before `near`
/// (from the start of `r`). A task's walk is a few days long, so this steps day by day, forward
/// or back, with one lookup a day and no counting from `r`; past [`SCAN_DAYS`] it searches.
pub(crate) fn moment_from(w: &dyn Work, r: Date, near: Date, before_near: i64, k: i64, start: bool) -> Option<Moment> {
    let (idx, start_side) = if start || k <= 0 { (k.max(0), true) } else { (k - 1, false) };
    let (mut d, mut acc) = (near, before_near);
    if idx >= acc {
        for _ in 0..SCAN_DAYS {
            let times = w.times_on(d);
            let m = i64::from(crate::minutes_of(&times));
            if acc + m > idx {
                return in_day(d, &times, idx - acc, start_side);
            }
            acc += m;
            d = d.add_days(1)?;
        }
    } else {
        for _ in 0..SCAN_DAYS {
            d = d.add_days(-1)?;
            let times = w.times_on(d);
            acc -= i64::from(crate::minutes_of(&times));
            if acc <= idx {
                return in_day(d, &times, idx - acc, start_side);
            }
        }
    }
    moment_near(w, r, k, start, if idx >= acc { d } else { r })
}

/// The moment of working minute `into` of `date`'s working `times`: its start, or its end when not `start`.
fn in_day(date: Date, times: &[WorkingTime], mut into: i64, start: bool) -> Option<Moment> {
    for t in times {
        let len = i64::from(t.minutes());
        if into < len {
            let minute = i64::from(t.from) + into + i64::from(!start);
            return Some(Moment { date, minute: u16::try_from(minute).ok()? });
        }
        into -= len;
    }
    None
}

/// Working minutes on `w` from the start of `r` to the start of `d`, and to the moment `m` on it.
pub(crate) fn counts_to(w: &dyn Work, r: Date, m: Moment) -> (i64, i64) {
    let day = if m.date >= r { w.minutes_between(r, m.date) } else { -w.minutes_between(m.date, r) };
    (day, day + minutes_before(&w.times_on(m.date), m.minute))
}

/// The moment of working minute `k` (counted from the start of `r`) on day `r + n`, which holds
/// it: its start, or its end when not `start`.
fn on_day(w: &dyn Work, r: Date, n: i64, k: i64, start: bool) -> Option<Moment> {
    let date = r.add_days(n)?;
    in_day(date, &w.times_on(date), k - w.minutes_between(r, date), start)
}

#[cfg(test)]
impl Calendar {
    /// Working minutes from the start of `r` up to the moment `m` ([`count_to`]).
    pub(crate) fn count_to(&self, r: Date, m: Moment) -> i64 {
        count_to(self, r, m)
    }

    /// The moment `k` working minutes after the start of `r` ([`moment_at`]).
    pub(crate) fn moment_at(&self, r: Date, k: i64, start: bool) -> Option<Moment> {
        moment_at(self, r, k, start)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parse_working_time;

    fn d(s: &str) -> Date {
        s.parse().unwrap()
    }

    fn at(s: &str, h: u16, m: u16) -> Moment {
        Moment { date: d(s), minute: h * 60 + m }
    }

    #[test]
    fn a_resource_calendar_is_the_project_calendar_with_its_own_days_on_top() {
        let mut p = Project::new("t", d("2026-01-05"));
        let ann = p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
        let steel = p.add_resource(Some("Steel"), ResourceType::Material, None).unwrap();
        assert_eq!(p.resource_calendar(ann).unwrap(), p.calendar, "nothing of its own");
        // Part time: Fridays off, Wednesday mornings; a week's leave; a holiday on the project.
        p.set_resource_weekday_times(ann, 4, Some(vec![])).unwrap();
        p.set_resource_weekday_times(ann, 2, Some(vec![parse_working_time("8:00-12:00").unwrap()])).unwrap();
        for day in 12..=16 {
            p.set_resource_exception(ann, Date::from_ymd(2026, 1, day).unwrap(), vec![]).unwrap();
        }
        p.calendar.add_exception(d("2026-01-06"), false).unwrap();
        let c = p.resource_calendar(ann).unwrap();
        assert!(!c.is_workday(d("2026-01-09")) && !c.is_workday(d("2026-01-13")) && !c.is_workday(d("2026-01-06")));
        assert_eq!(c.minutes_on(d("2026-01-07")), 240);
        assert_eq!(c.minutes_on(d("2026-01-05")), 480, "Monday follows the project");
        // Back to the project's times; an exception removed; a material has no calendar.
        p.set_resource_weekday_times(ann, 4, None).unwrap();
        p.remove_resource_exception(ann, d("2026-01-12")).unwrap();
        assert!(p.remove_resource_exception(ann, d("2026-01-12")).is_err());
        let c = p.resource_calendar(ann).unwrap();
        assert!(c.is_workday(d("2026-01-09")) && c.is_workday(d("2026-01-12")));
        let e = p.set_resource_exception(steel, d("2026-01-12"), vec![]).unwrap_err().to_string();
        assert!(e.contains("only work resources have calendars"), "{e}");
        assert_eq!(p.resource_calendar(steel).unwrap(), p.calendar);
        // A week with no working day is refused and changes nothing.
        let before = p.clone();
        assert!(p.set_resource_working(ann, [false; 7]).is_err());
        assert_eq!(p, before);
        // Saturday ticked: her own, with the default times, worked though the project doesn't.
        p.set_resource_working(ann, [true, true, true, true, false, true, false]).unwrap();
        let r = &p.resource(ann).unwrap().calendar;
        assert_eq!(r.weekday_times(&p.calendar, 5), &DEFAULT_WORKING_TIMES);
        assert!(!r.within(&p.calendar));
        let c = p.resource_calendar(ann).unwrap();
        assert_eq!(c.working, [true, true, true, true, false, true, false]);
        assert_eq!(c.minutes_on(d("2026-01-10")), 480);
        assert_eq!(c.minutes_on(d("2026-01-07")), 240, "a weekday left working keeps its own times");
        // Ticking a day back to what the project does follows the project again, its hours included.
        p.calendar.set_weekday_times(0, vec![parse_working_time("7:00-15:00").unwrap()]).unwrap();
        p.set_resource_working(ann, [false, true, true, true, false, false, false]).unwrap();
        p.set_resource_working(ann, [true, true, true, true, true, false, false]).unwrap();
        let r = &p.resource(ann).unwrap().calendar;
        assert_eq!((r.week[0].clone(), r.week[4].clone(), r.week[5].clone()), (None, None, None));
        assert_eq!(p.resource_calendar(ann).unwrap().weekday_times(0), &[parse_working_time("7:00-15:00").unwrap()]);
        // Her own Wednesday mornings are hers whatever the project works then.
        p.set_resource_working(ann, [false, true, true, true, true, false, false]).unwrap();
        p.calendar.set_working([true, false, false, false, false, false, false]).unwrap();
        p.check_resource_weeks().unwrap();
        assert_eq!(p.resource_calendar(ann).unwrap().minutes_on(d("2026-01-07")), 240);
        // The project calendar can't leave her with no working weekday: refused, naming her.
        p.set_resource_weekday_times(ann, 2, None).unwrap_err();
        p.calendar.set_working([true, true, true, true, true, false, false]).unwrap();
        p.set_resource_weekday_times(ann, 2, None).unwrap();
        p.calendar.set_working([true, false, false, false, false, false, false]).unwrap();
        let e = p.check_resource_weeks().unwrap_err().to_string();
        assert!(e.contains("Ann would have no working weekday"), "{e}");
    }

    /// Walking by working minutes: a moment's count and the moment of a count agree, across
    /// lunch, nights, weekends and days off.
    #[test]
    fn counting_and_finding_moments_agree() {
        let mut c = Calendar::default();
        c.add_exception(d("2026-01-07"), false).unwrap();
        let r = d("2026-01-05");
        assert_eq!(c.moment_at(r, 0, true), Some(at("2026-01-05", 8, 0)));
        assert_eq!(c.moment_at(r, 240, false), Some(at("2026-01-05", 12, 0)));
        assert_eq!(c.moment_at(r, 240, true), Some(at("2026-01-05", 13, 0)));
        assert_eq!(c.moment_at(r, 960, false), Some(at("2026-01-06", 17, 0)));
        assert_eq!(c.moment_at(r, 960, true), Some(at("2026-01-08", 8, 0)), "Wednesday is off");
        assert_eq!(c.moment_at(r, -5, false), Some(at("2026-01-05", 8, 0)));
        for k in [0, 1, 239, 240, 241, 479, 480, 960, 2400, 100_000] {
            for start in [true, false] {
                let m = c.moment_at(r, k, start).unwrap();
                assert_eq!(c.count_to(r, m), k, "{k} {start}");
            }
        }
        // Inside non-working time, a count stays put: Saturday noon is after all of Friday.
        assert_eq!(c.count_to(r, at("2026-01-10", 12, 0)), 4 * 480);
        assert_eq!(c.count_to(r, at("2026-01-02", 12, 0)), -240, "before the start: Friday afternoon");
        // Past year 9999, or a calendar of a day's work, runs out.
        assert_eq!(c.moment_at(r, i64::MAX / 4, true), None);
        let mut one = Calendar { working: [false; 7], ..Calendar::default() };
        one.add_exception(d("2026-01-14"), true).unwrap();
        assert_eq!(one.moment_at(r, 479, true), Some(at("2026-01-14", 16, 59)));
        assert_eq!(one.moment_at(r, 480, true), None);
        assert_eq!(one.moment_at(r, 480, false), Some(at("2026-01-14", 17, 0)));
    }

    /// The quick counts ([`Counted`], [`ResView`]) agree with a merged copy of the calendar, day
    /// by day and over any range: weekdays of its own, its own exceptions (on project exceptions
    /// or plain days), times outside the project's (not worked), all together.
    fn agrees(base: &Calendar, own: &ResourceCalendar) {
        let merged = own.on(base);
        let counted = Counted::new(base);
        let view = ResView::new(&counted, own);
        let on = OnProject { base: &counted, view: &view };
        let start = d("2025-12-29");
        let days: Vec<Date> = (0..70).map(|k| start.add_days(k).unwrap()).collect();
        for (k, a) in days.iter().enumerate() {
            assert_eq!(Work::times_on(&on, *a).to_vec(), merged.times_on(*a).to_vec(), "{a}");
            assert_eq!(Work::times_on(&counted, *a).to_vec(), base.times_on(*a).to_vec(), "{a}");
            for b in days.iter().skip(k) {
                assert_eq!(Work::minutes_between(&counted, *a, *b), base.minutes_between(*a, *b), "{a}..{b}");
                assert_eq!(Work::minutes_between(&on, *a, *b), merged.minutes_between(*a, *b), "{a}..{b}");
            }
        }
        assert_eq!(view.per_week(), Work::per_week(&merged));
        // The plan's working time holds every minute either works; none needed when the
        // resource works only within the project's.
        let plan = plan_time(base, &[own]);
        for a in &days {
            let line = plan.as_ref().map_or(base.times_on(*a), |c| c.times_on(*a));
            assert!(inside(base.times_on(*a), line) && inside(merged.times_on(*a), line), "{a}");
        }
    }

    #[test]
    fn quick_counts_agree_with_the_merged_calendar() {
        let t = |s: &str| parse_working_time(s).unwrap();
        let mut base = Calendar::default();
        base.set_weekday_times(4, vec![t("8:00-12:00")]).unwrap();
        base.add_exception(d("2026-01-07"), false).unwrap();
        base.set_exception(d("2026-01-10"), vec![t("9:00-13:00")]).unwrap();
        base.add_exception(d("2026-01-19"), true).unwrap();
        let mut own = ResourceCalendar::default();
        agrees(&base, &own);
        own.week[0] = Some(vec![t("7:00-10:00"), t("14:00-18:00")]);
        own.week[2] = Some(vec![]);
        own.week[5] = Some(vec![t("8:00-17:00")]);
        for (day, times) in [
            ("2026-01-07", vec![t("8:00-17:00")]),
            ("2026-01-08", vec![]),
            ("2026-01-12", vec![t("10:00-11:00")]),
            ("2026-01-10", vec![t("10:00-20:00")]),
            ("2026-01-14", vec![]),
        ] {
            own.exceptions.insert(d(day), times);
        }
        agrees(&base, &own);
        // Time of its own outside the project's is worked: Saturday, the holiday, before 8:00.
        let merged = own.on(&base);
        assert_eq!(merged.times_on(d("2026-01-17")), &[t("8:00-17:00")]);
        assert_eq!(merged.times_on(d("2026-01-07")), &[t("8:00-17:00")], "the project's holiday");
        assert_eq!(merged.times_on(d("2026-01-05")), &[t("7:00-10:00"), t("14:00-18:00")]);
        assert_eq!(merged.times_on(d("2026-01-10")), &[t("10:00-20:00")], "past the project's worked Saturday");
        assert!(!own.within(&base));
        // The plan's working time holds every minute of both.
        let plan = plan_time(&base, &[&own]).unwrap();
        for k in 0..70 {
            let day = d("2025-12-29").add_days(k).unwrap();
            assert!(inside(base.times_on(day), plan.times_on(day)) && inside(merged.times_on(day), plan.times_on(day)), "{day}");
        }
        assert_eq!(plan.times_on(d("2026-01-17")), &[t("8:00-17:00")]);
        // Within the project's time, the project calendar is the plan's.
        let part_time = ResourceCalendar { week: [Some(vec![t("8:00-12:00")]), None, Some(vec![]), None, None, None, None], ..Default::default() };
        assert!(part_time.within(&base));
        assert_eq!(plan_time(&base, &[&part_time]), None);
        assert_eq!(union(&[t("8:00-12:00")], &[t("12:00-13:00"), t("7:00-9:00")]), vec![t("7:00-13:00")]);
    }

    proptest::proptest! {
        #[test]
        fn quick_counts_agree_on_any_calendar(
            week in proptest::collection::vec(proptest::option::of(0u8..4), 7),
            base_days in proptest::collection::vec((0i64..70, 0u8..4), 0..12),
            own_days in proptest::collection::vec((0i64..70, 0u8..4), 0..12),
        ) {
            let t = |s: &str| parse_working_time(s).unwrap();
            let pick = |k: u8| [vec![], vec![t("8:00-12:00")], vec![t("10:00-16:00")], DEFAULT_WORKING_TIMES.to_vec()][usize::from(k)].clone();
            let start = d("2025-12-29");
            let mut base = Calendar::default();
            for (day, k) in base_days {
                let _ = base.set_exception(start.add_days(day).unwrap(), pick(k));
            }
            let mut own = ResourceCalendar::default();
            for (i, w) in week.into_iter().enumerate() {
                own.week[i] = w.map(pick);
            }
            for (day, k) in own_days {
                own.exceptions.insert(start.add_days(day).unwrap(), pick(k));
            }
            agrees(&base, &own);
        }

        #[test]
        fn moments_and_counts_round_trip(k in 0i64..2_000_000, start: bool, off in proptest::collection::vec(0i64..400, 0..20)) {
            let mut c = Calendar::default();
            for o in off {
                let _ = c.add_exception(d("2026-01-05").add_days(o).unwrap(), o % 3 == 0);
            }
            let m = c.moment_at(d("2026-01-05"), k, start).unwrap();
            proptest::prop_assert_eq!(c.count_to(d("2026-01-05"), m), k);
            // From any nearby day with its count known, forward or back, the same moment.
            for near in [m.date, m.date.add_days(-3).unwrap(), m.date.add_days(5).unwrap(), m.date.add_days(-200).unwrap()] {
                let (before, _) = counts_to(&c, d("2026-01-05"), Moment { date: near, minute: 0 });
                proptest::prop_assert_eq!(moment_from(&c, d("2026-01-05"), near, before, k, start), Some(m), "from {:?}", near);
            }
        }
    }
}
