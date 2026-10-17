//! MSPDI calendars: the project calendar's working week and exceptions, imported and then
//! scheduled, against hand-written files built from Microsoft's published schema docs
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/calendar-element>). The
//! plans are invented.

use std::time::{Duration, Instant};

use projectcraft_interchange::model::{self, Date, MAX_EXCEPTIONS, Moment, TaskId, WorkingTime};
use projectcraft_interchange::{Format, Import, Outcome, ReportLine, import};

fn mspdi(text: &str) -> Import {
    import(Format::Mspdi, text.as_bytes()).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> Date {
    Date::from_ymd(y, m, day).unwrap()
}

fn t(from: u16, to: u16) -> WorkingTime {
    WorkingTime::new(from, to).unwrap()
}

fn lines<'a>(i: &'a Import, feature: &str) -> Vec<&'a ReportLine> {
    i.report.lines.iter().filter(|l| l.feature == feature).collect()
}

const DAY: &str = "<WorkingTimes><WorkingTime><FromTime>08:00:00</FromTime><ToTime>12:00:00</ToTime></WorkingTime>\
<WorkingTime><FromTime>13:00:00</FromTime><ToTime>17:00:00</ToTime></WorkingTime></WorkingTimes>";
const MORNING: &str = "<WorkingTimes><WorkingTime><FromTime>08:00:00</FromTime><ToTime>12:00:00</ToTime></WorkingTime></WorkingTimes>";

/// The Standard week (Monday to Friday, 8:00–12:00 and 13:00–17:00) as `WeekDay`s, with
/// `friday` as Friday's working times.
fn week(friday: &str) -> String {
    let mut s = String::from("<WeekDays><WeekDay><DayType>1</DayType><DayWorking>0</DayWorking></WeekDay>");
    for day in 2..=5 {
        s.push_str(&format!("<WeekDay><DayType>{day}</DayType><DayWorking>1</DayWorking>{DAY}</WeekDay>"));
    }
    s.push_str(&format!("<WeekDay><DayType>6</DayType><DayWorking>1</DayWorking>{friday}</WeekDay>"));
    s.push_str("<WeekDay><DayType>7</DayType><DayWorking>0</DayWorking></WeekDay></WeekDays>");
    s
}

/// A project starting Monday 2026-03-02 on calendar UID 1 (`calendar` is its inside after the
/// UID), with other `calendars`, and one task of `hours` working hours.
fn plan(calendar: &str, calendars: &str, hours: u32) -> String {
    format!(
        "<Project xmlns=\"http://schemas.microsoft.com/project\"><StartDate>2026-03-02T08:00:00</StartDate><CalendarUID>1</CalendarUID>\
<MinutesPerDay>480</MinutesPerDay><Calendars><Calendar><UID>1</UID><Name>Standard</Name><IsBaseCalendar>1</IsBaseCalendar>{calendar}</Calendar>{calendars}</Calendars>\
<Tasks><Task><UID>1</UID><ID>1</ID><Name>Build the shed</Name><Duration>PT{hours}H0M0S</Duration><DurationFormat>7</DurationFormat></Task></Tasks></Project>"
    )
}

/// One `Exception` of the given inside, wrapped in `Exceptions`.
fn exceptions(each: &[&str]) -> String {
    let mut s = String::from("<Exceptions>");
    for e in each {
        s.push_str(&format!("<Exception>{e}</Exception>"));
    }
    s.push_str("</Exceptions>");
    s
}

fn period(from: &str, to: &str) -> String {
    format!("<TimePeriod><FromDate>{from}T00:00:00</FromDate><ToDate>{to}T23:59:00</ToDate></TimePeriod>")
}

/// When the one task finishes, as the scheduler works it out on the imported calendar.
fn finish(i: &Import) -> Moment {
    model::schedule(&i.project).unwrap().tasks[&TaskId(1)].finish_at
}

#[test]
fn part_day_weekdays_are_worked_as_the_file_has_them() {
    // Monday to Thursday 8 hours, Friday a 4-hour morning: a 5-day (40-hour) task works 32 hours
    // Monday to Thursday, 4 on Friday, and its last 4 on the next Monday morning.
    let i = mspdi(&plan(&week(MORNING), "", 40));
    assert_eq!(i.project.calendar.weekday_times(4), [t(480, 720)]);
    assert_eq!(finish(&i), Moment { date: d(2026, 3, 9), minute: 720 });
}

#[test]
fn a_non_working_exception_is_a_day_off() {
    let holiday =
        format!("{}<Occurrences>1</Occurrences><Name>Site closed</Name><Type>1</Type><DayWorking>0</DayWorking>", period("2026-03-04", "2026-03-04"));
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&holiday])), "", 24));
    assert_eq!(i.project.calendar.exceptions.get(&d(2026, 3, 4)), Some(&false));
    // Monday, Tuesday, (Wednesday off), Thursday.
    assert_eq!(finish(&i), Moment { date: d(2026, 3, 5), minute: 1020 });
    assert!(lines(&i, "Calendar exceptions").is_empty(), "{:#?}", i.report.lines);
}

#[test]
fn an_exception_with_working_times_works_those_times() {
    let saturday = format!(
        "{}<Name>Overtime Saturday</Name><Type>1</Type><DayWorking>1</DayWorking><WorkingTimes><WorkingTime><FromTime>09:00:00</FromTime><ToTime>13:00:00</ToTime></WorkingTime></WorkingTimes>",
        period("2026-03-07", "2026-03-07")
    );
    let short_friday = format!(
        "{}<Name>Short Friday</Name><Type>1</Type><DayWorking>1</DayWorking><WorkingTimes><WorkingTime><FromTime>08:00:00</FromTime><ToTime>10:00:00</ToTime></WorkingTime></WorkingTimes>",
        period("2026-03-06", "2026-03-06")
    );
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&saturday, &short_friday])), "", 48));
    let c = &i.project.calendar;
    assert_eq!(c.times_on(d(2026, 3, 7)), [t(540, 780)]);
    assert_eq!(c.times_on(d(2026, 3, 6)), [t(480, 600)]);
    // 32 hours Monday to Thursday, 2 on Friday, 4 on Saturday, 8 on Monday, the last 2 on Tuesday.
    assert_eq!(finish(&i), Moment { date: d(2026, 3, 10), minute: 600 });
}

#[test]
fn a_working_exception_without_times_works_the_default_day() {
    let e = format!("{}<Name>Catch-up Sunday</Name><Type>1</Type><DayWorking>1</DayWorking>", period("2026-03-08", "2026-03-08"));
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&e])), "", 8));
    assert_eq!(i.project.calendar.times_on(d(2026, 3, 8)), [t(480, 720), t(780, 1020)]);
}

/// The exception days of a calendar holding only `each`.
fn days_of(each: &[&str]) -> Vec<(Date, bool)> {
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(each)), "", 8));
    assert!(lines(&i, "Calendar exceptions").is_empty(), "{:#?}", i.report.lines);
    i.project.calendar.exceptions.iter().map(|(d, w)| (*d, *w)).collect()
}

fn off(dates: &[Date]) -> Vec<(Date, bool)> {
    dates.iter().map(|d| (*d, false)).collect()
}

#[test]
fn recurring_exceptions_are_expanded_to_their_days() {
    // Type 1 daily, every day of the period and every second day.
    let e = format!("{}<Type>1</Type><DayWorking>0</DayWorking>", period("2026-12-24", "2026-12-26"));
    assert_eq!(days_of(&[&e]), off(&[d(2026, 12, 24), d(2026, 12, 25), d(2026, 12, 26)]));
    let e = format!("{}<Type>1</Type><Period>2</Period><DayWorking>0</DayWorking>", period("2026-03-02", "2026-03-08"));
    assert_eq!(days_of(&[&e]), off(&[d(2026, 3, 2), d(2026, 3, 4), d(2026, 3, 6), d(2026, 3, 8)]));
    // Type 2 yearly on a day of a month (Month 0 is January).
    let e = format!("{}<Type>2</Type><Month>0</Month><MonthDay>1</MonthDay><DayWorking>0</DayWorking>", period("2027-01-01", "2030-01-01"));
    assert_eq!(days_of(&[&e]), off(&[d(2027, 1, 1), d(2028, 1, 1), d(2029, 1, 1), d(2030, 1, 1)]));
    // Type 3 yearly by position: the fourth (3) Thursday (7) of November (10).
    let e = format!(
        "{}<Type>3</Type><Month>10</Month><MonthItem>7</MonthItem><MonthPosition>3</MonthPosition><DayWorking>0</DayWorking>",
        period("2026-11-01", "2027-11-30")
    );
    assert_eq!(days_of(&[&e]), off(&[d(2026, 11, 26), d(2027, 11, 25)]));
    // Type 4 monthly on a day, every second month.
    let e = format!("{}<Type>4</Type><Period>2</Period><MonthDay>15</MonthDay><DayWorking>0</DayWorking>", period("2026-01-15", "2026-06-30"));
    assert_eq!(days_of(&[&e]), off(&[d(2026, 1, 15), d(2026, 3, 15), d(2026, 5, 15)]));
    // Type 5 monthly by position: the last (4) Friday (8); and the first (0) weekday (1).
    let e = format!(
        "{}<Type>5</Type><Period>1</Period><MonthItem>8</MonthItem><MonthPosition>4</MonthPosition><DayWorking>0</DayWorking>",
        period("2026-03-01", "2026-04-30")
    );
    assert_eq!(days_of(&[&e]), off(&[d(2026, 3, 27), d(2026, 4, 24)]));
    let e = format!(
        "{}<Type>5</Type><MonthItem>1</MonthItem><MonthPosition>0</MonthPosition><DayWorking>0</DayWorking>",
        period("2026-08-01", "2026-08-31")
    );
    assert_eq!(days_of(&[&e]), off(&[d(2026, 8, 3)]), "1 August 2026 is a Saturday");
    // Type 6 weekly, Tuesdays and Thursdays (4 + 16) every second week, weeks starting Sunday.
    let e = format!("{}<Type>6</Type><Period>2</Period><DaysOfWeek>20</DaysOfWeek><DayWorking>0</DayWorking>", period("2026-03-02", "2026-03-22"));
    assert_eq!(days_of(&[&e]), off(&[d(2026, 3, 3), d(2026, 3, 5), d(2026, 3, 17), d(2026, 3, 19)]));
}

#[test]
fn an_exception_entered_by_occurrences_stops_after_them() {
    let e = format!(
        "<EnteredByOccurrences>1</EnteredByOccurrences>{}<Occurrences>3</Occurrences><Type>7</Type><DayWorking>0</DayWorking>",
        period("2026-03-02", "2026-03-31")
    );
    assert_eq!(days_of(&[&e]), off(&[d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 4)]));
}

#[test]
fn a_later_exception_wins_over_an_earlier_one_on_the_same_day() {
    let off_day = format!("{}<Type>1</Type><DayWorking>0</DayWorking>", period("2026-03-07", "2026-03-07"));
    let on_day = format!("{}<Type>1</Type><DayWorking>1</DayWorking>{MORNING}", period("2026-03-07", "2026-03-07"));
    assert_eq!(days_of(&[&off_day, &on_day]), [(d(2026, 3, 7), true)]);
}

#[test]
fn older_files_exceptions_as_day_type_0_weekdays_are_imported() {
    // Before Project 2007, an exception was a WeekDay with DayType 0 and a TimePeriod.
    let old = format!(
        "<WeekDays><WeekDay><DayType>0</DayType><DayWorking>0</DayWorking>{}</WeekDay>\
<WeekDay><DayType>0</DayType><DayWorking>1</DayWorking>{}{MORNING}</WeekDay></WeekDays>",
        period("2026-03-04", "2026-03-05"),
        period("2026-03-07", "2026-03-07")
    );
    let i = mspdi(&plan(&old, "", 8));
    let c = &i.project.calendar;
    assert_eq!(
        c.exceptions.iter().map(|(d, w)| (*d, *w)).collect::<Vec<_>>(),
        [(d(2026, 3, 4), false), (d(2026, 3, 5), false), (d(2026, 3, 7), true)]
    );
    assert_eq!(c.times_on(d(2026, 3, 7)), [t(480, 720)]);
    // Newer files write the same exceptions both ways; the Exceptions collection is read then.
    let new = format!("{}<Type>1</Type><DayWorking>0</DayWorking>", period("2026-03-10", "2026-03-10"));
    let both = format!(
        "<WeekDays><WeekDay><DayType>0</DayType><DayWorking>0</DayWorking>{}</WeekDay></WeekDays>{}",
        period("2026-03-04", "2026-03-04"),
        exceptions(&[&new])
    );
    let c = mspdi(&plan(&both, "", 8)).project.calendar;
    assert_eq!(c.exceptions.keys().copied().collect::<Vec<_>>(), [d(2026, 3, 10)]);
}

#[test]
fn unreadable_exceptions_are_dropped_and_reported() {
    let no_period = "<Name>No dates</Name><Type>1</Type><DayWorking>0</DayWorking>".to_string();
    let no_days =
        format!("<Name>No weekdays</Name>{}<Type>6</Type><DaysOfWeek>0</DaysOfWeek><DayWorking>0</DayWorking>", period("2026-03-02", "2026-03-31"));
    let bad_times = format!(
        "<Name>Backwards</Name>{}<Type>1</Type><DayWorking>1</DayWorking><WorkingTimes><WorkingTime><FromTime>17:00:00</FromTime><ToTime>08:00:00</ToTime></WorkingTime></WorkingTimes>",
        period("2026-03-07", "2026-03-07")
    );
    let backwards = format!("<Name>Ends first</Name>{}<Type>1</Type><DayWorking>0</DayWorking>", period("2026-03-09", "2026-03-02"));
    let good = format!("<Name>Fine</Name>{}<Type>1</Type><DayWorking>0</DayWorking>", period("2026-03-04", "2026-03-04"));
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&no_period, &no_days, &bad_times, &backwards, &good])), "", 8));
    assert_eq!(i.project.calendar.exceptions.keys().copied().collect::<Vec<_>>(), [d(2026, 3, 4)]);
    let l = lines(&i, "Calendar exceptions");
    assert_eq!(l.len(), 1, "{:#?}", i.report.lines);
    assert_eq!(l[0].outcome, Outcome::Dropped);
    assert_eq!(l[0].place, "No dates, No weekdays, Backwards and 1 more exception");
}

#[test]
fn each_dropped_base_calendar_and_task_calendar_has_a_line() {
    let calendars = "<Calendar><UID>2</UID><Name>Night shift</Name><IsBaseCalendar>1</IsBaseCalendar></Calendar>\
<Calendar><UID>4</UID><Name>Weekend crew</Name><IsBaseCalendar>1</IsBaseCalendar></Calendar>\
<Calendar><UID>5</UID><Name>Winter hours</Name><IsBaseCalendar>1</IsBaseCalendar></Calendar>\
<Calendar><UID>3</UID><Name>Rhiannon Price</Name><IsBaseCalendar>0</IsBaseCalendar><BaseCalendarUID>1</BaseCalendarUID></Calendar>";
    let tasks = "<Tasks><Task><UID>1</UID><ID>1</ID><Name>A</Name><CalendarUID>-1</CalendarUID></Task>\
<Task><UID>2</UID><ID>2</ID><Name>B</Name><CalendarUID>4</CalendarUID></Task>\
<Task><UID>3</UID><ID>3</ID><Name>C</Name><CalendarUID>9</CalendarUID></Task>\
<Task><UID>4</UID><ID>4</ID><Name>D</Name><CalendarUID>4</CalendarUID></Task>\
<Task><UID>5</UID><ID>5</ID><Name>E</Name><CalendarUID>1</CalendarUID></Task></Tasks>";
    let text = plan(&week(DAY), calendars, 8).replace(
        "<Tasks><Task><UID>1</UID><ID>1</ID><Name>Build the shed</Name><Duration>PT8H0M0S</Duration><DurationFormat>7</DurationFormat></Task></Tasks>",
        tasks,
    );
    let i = mspdi(&text);
    let got = |f: &str| lines(&i, f).iter().map(|l| (l.outcome, l.place.clone())).collect::<Vec<_>>();
    assert_eq!(got("Base calendar"), [(Outcome::Dropped, "Night shift (UID 2)".to_string()), (Outcome::Dropped, "Winter hours (UID 5)".to_string())]);
    assert_eq!(
        got("Task calendar"),
        [(Outcome::Dropped, "Weekend crew (UID 4): ID 2 B, ID 4 D".to_string()), (Outcome::Dropped, "UID 9, not in the file: ID 3 C".to_string())],
        "a task on the project calendar (UID 1) or none (-1) has no line"
    );
    assert_eq!(got("Resource calendars"), [(Outcome::NotYetImported, "1 calendar".to_string())]);
    assert!(lines(&i, "Task calendar").iter().all(|l| l.why.contains("project calendar")));
}

#[test]
fn a_huge_daily_exception_is_capped_quickly() {
    let all = format!("<Name>Forever</Name>{}<Type>1</Type><DayWorking>0</DayWorking>", period("0001-01-01", "9999-12-31"));
    let start = Instant::now();
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&all])), "", 8));
    assert_eq!(i.project.calendar.exceptions.len(), MAX_EXCEPTIONS);
    let l = lines(&i, "Calendar exceptions");
    assert_eq!(l.len(), 1, "{:#?}", i.report.lines);
    assert!(l[0].why.contains("100,000"), "{}", l[0].why);
    assert!(start.elapsed() < Duration::from_secs(5), "{:?}", start.elapsed());
}

#[test]
fn many_recurring_exceptions_over_long_ranges_never_spin() {
    // The 31st of February never comes: each exception walks its range and finds nothing.
    let never = format!("{}<Type>2</Type><Month>1</Month><MonthDay>31</MonthDay><DayWorking>0</DayWorking>", period("0001-01-01", "9999-12-31"));
    let monthly = format!(
        "{}<Type>5</Type><MonthItem>0</MonthItem><MonthPosition>4</MonthPosition><DayWorking>0</DayWorking>",
        period("0001-01-01", "9999-12-31")
    );
    let each: Vec<&str> = std::iter::repeat_n(never.as_str(), 2_000).chain(std::iter::repeat_n(monthly.as_str(), 200)).collect();
    let start = Instant::now();
    let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&each)), "", 8));
    assert!(i.project.calendar.exceptions.len() <= MAX_EXCEPTIONS);
    assert!(start.elapsed() < Duration::from_secs(10), "{:?}", start.elapsed());
    model::schedule(&i.project).unwrap();
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    #[test]
    fn arbitrary_exceptions_never_panic(
        ty in -2i64..12,
        every in -2i64..400,
        days in -2i64..200,
        month in -2i64..14,
        month_day in -2i64..34,
        item in -2i64..11,
        position in -2i64..6,
        by_count in proptest::bool::ANY,
        count in -2i64..100_000,
        from in (1i32..=9999, 1u32..=12, 1u32..=28),
        span in 0i64..4_000_000,
        working in proptest::bool::ANY,
        junk in "\\PC{0,12}",
    ) {
        let from = Date::from_ymd(from.0, from.1, from.2).unwrap();
        let to = from.add_days(span).unwrap_or(from);
        let e = format!(
            "<EnteredByOccurrences>{}</EnteredByOccurrences>{}<Occurrences>{count}</Occurrences><Name>{}</Name><Type>{ty}</Type><Period>{every}</Period>\
<DaysOfWeek>{days}</DaysOfWeek><Month>{month}</Month><MonthDay>{month_day}</MonthDay><MonthItem>{item}</MonthItem><MonthPosition>{position}</MonthPosition>\
<DayWorking>{}</DayWorking>{MORNING}",
            u8::from(by_count),
            period(&from.to_string(), &to.to_string()),
            junk.replace(['<', '&'], ""),
            u8::from(working),
        );
        let i = mspdi(&plan(&format!("{}{}", week(DAY), exceptions(&[&e, &e])), "", 8));
        proptest::prop_assert!(i.project.calendar.exceptions.len() <= MAX_EXCEPTIONS);
        let _ = model::schedule(&i.project);
    }
}
