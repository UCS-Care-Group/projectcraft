//! MSPDI resources, assignments and costs: worked examples imported, scheduled and costed to the
//! penny; resource calendars with part-time weeks and leave; rate tables B–E and contours other
//! than flat reported; hostile values clamped or dropped, a report line each. Hand-written files
//! built from Microsoft's published schema docs
//! (<https://learn.microsoft.com/office-project/xml-data-interchange/resource-element>,
//! <https://learn.microsoft.com/office-project/xml-data-interchange/assignment-element>). The plans
//! are invented.

use projectcraft_interchange::model::{
    self, AccrueAt, Date, FULL_TIME, MAX_MINUTES, MAX_MONEY, MAX_RESOURCES, MAX_UNITS, ResourceId, ResourceType, Units, WorkingTime,
};
use projectcraft_interchange::{Format, Import, Outcome, ReportLine, import};

const PATIO: &str = include_str!("fixtures/patio-costs.xml");

fn mspdi(text: &str) -> Import {
    import(Format::Mspdi, text.as_bytes()).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> Date {
    Date::from_ymd(y, m, day).unwrap()
}

fn lines<'a>(i: &'a Import, feature: &str) -> Vec<&'a ReportLine> {
    i.report.lines.iter().filter(|l| l.feature == feature).collect()
}

fn find<'a>(i: &'a Import, feature: &str) -> &'a ReportLine {
    let found = lines(i, feature);
    assert_eq!(found.len(), 1, "one `{feature}` line in {:#?}", i.report.lines);
    found[0]
}

fn t(from: u16, to: u16) -> WorkingTime {
    WorkingTime::new(from, to).unwrap()
}

/// A project starting Monday 2026-03-02 on the default calendar, with `extra` project fields,
/// tasks of `(uid, name, hours)`, and the resources and assignments as the file writes them.
fn plan(extra: &str, tasks: &[(i64, &str, u32)], resources: &str, assignments: &str) -> String {
    let mut s = format!(
        "<Project xmlns=\"http://schemas.microsoft.com/project\"><StartDate>2026-03-02T08:00:00</StartDate><MinutesPerDay>480</MinutesPerDay>{extra}<Tasks>"
    );
    for (uid, name, hours) in tasks {
        s.push_str(&format!(
            "<Task><UID>{uid}</UID><ID>{uid}</ID><Name>{name}</Name><OutlineLevel>1</OutlineLevel><Duration>PT{hours}H0M0S</Duration><DurationFormat>7</DurationFormat></Task>"
        ));
    }
    s.push_str(&format!("</Tasks><Resources>{resources}</Resources><Assignments>{assignments}</Assignments></Project>"));
    s
}

fn resource(uid: i64, name: &str, inside: &str) -> String {
    format!("<Resource><UID>{uid}</UID><ID>{uid}</ID><Name>{name}</Name>{inside}</Resource>")
}

fn assignment(task: i64, resource: i64, inside: &str) -> String {
    format!("<Assignment><TaskUID>{task}</TaskUID><ResourceUID>{resource}</ResourceUID>{inside}</Assignment>")
}

#[test]
fn work_material_and_cost_resources_are_imported_as_the_file_has_them() {
    let i = mspdi(PATIO);
    let p = &i.project;
    let rows: Vec<(u32, &str, ResourceType, u32, i64, i64, i64)> =
        p.resources.iter().map(|r| (r.id.0, r.name.as_str(), r.kind, r.max_units, r.standard_rate, r.overtime_rate, r.cost_per_use)).collect();
    assert_eq!(
        rows,
        [
            // Overtime 63.555 is rounded half to even, to the penny.
            (1, "Ann Lee", ResourceType::Work, FULL_TIME, 4237, 6356, 1500),
            (2, "Gravel", ResourceType::Material, FULL_TIME, 3820, 0, 0),
            (3, "Skip hire", ResourceType::Cost, FULL_TIME, 0, 0, 0),
            (4, "Ben Ortiz", ResourceType::Work, FULL_TIME / 2, 3000, 0, 0),
        ],
        "Project's placeholder resource (UID 0) is not imported"
    );
    let ann = p.resource(ResourceId(1)).unwrap();
    assert_eq!((ann.initials.as_str(), ann.group.as_str(), ann.accrue_at), ("AL", "Groundworks", AccrueAt::Prorated));
    assert_eq!(p.resource(ResourceId(2)).unwrap().material_label, "tons");
    assert_eq!(p.next_resource_id, 5);
    assert_eq!((p.currency.code.as_str(), p.currency.symbol.as_str(), p.currency.digits), ("GBP", "£", 2));
    let a: Vec<(u32, u32, Units, i64, i64)> = p.assignments.iter().map(|a| (a.task.0, a.resource.0, a.units, a.work, a.duration)).collect();
    assert_eq!(
        a,
        [
            (1, 1, Units::Work(FULL_TIME), 24 * 60, 24 * 60),
            (2, 2, Units::Fixed(650), 0, 0),
            (2, 1, Units::Work(FULL_TIME / 2), 4 * 60, 8 * 60),
            (3, 3, Units::Cost(18599), 0, 0),
            (4, 4, Units::Work(FULL_TIME), 16 * 60, 16 * 60),
            (5, 1, Units::Work(FULL_TIME), 450, 450),
        ],
        "the task without resources (-65535) has no assignment"
    );
    assert_eq!(p.tasks[0].fixed_cost, 12025);
}

#[test]
fn costs_match_to_the_penny_and_dates_follow_the_resources() {
    let i = mspdi(PATIO);
    let p = &i.project;
    let cost = p.cost_by_task();
    let by: Vec<(u32, i64)> = p.tasks.iter().map(|t| (t.id.0, cost[&t.id])).collect();
    assert_eq!(
        by,
        [
            // 24 h × £42.37 + £15 per use + £120.25 fixed.
            (1, 101688 + 1500 + 12025),
            // 6.5 tons × £38.20, and 4 h × £42.37 + £15.
            (2, 24830 + 16948 + 1500),
            (3, 18599),
            // 16 h × £30 on Ben's part-time calendar.
            (4, 48000),
            // 7.5 h × £42.37 is £317.775: half to even, £317.78; + £15.
            (5, 31778 + 1500),
        ]
    );
    assert_eq!(p.total_cost(), 258368);
    let s = model::schedule(p).unwrap();
    let dates: Vec<(u32, Date, Date)> = p.tasks.iter().map(|t| (t.id.0, s.tasks[&t.id].start, s.tasks[&t.id].finish)).collect();
    assert_eq!(
        dates,
        [
            (1, d(2026, 3, 2), d(2026, 3, 4)),
            (2, d(2026, 3, 5), d(2026, 3, 5)),
            (3, d(2026, 3, 6), d(2026, 3, 6)),
            // Mornings Monday to Wednesday, on leave Wednesday 4th: 4 + 4 + 4 + 4 hours.
            (4, d(2026, 3, 2), d(2026, 3, 10)),
            (5, d(2026, 3, 9), d(2026, 3, 9)),
        ]
    );
    assert!(lines(&i, "Start and finish").is_empty(), "the file's dates are ProjectCraft's: {:#?}", i.report.lines);
}

#[test]
fn a_resource_calendar_with_a_part_time_week_and_leave_is_imported() {
    let p = mspdi(PATIO).project;
    let ben = &p.resource(ResourceId(4)).unwrap().calendar;
    let mornings = Some(vec![t(480, 720)]);
    assert_eq!(ben.week, [mornings.clone(), mornings.clone(), mornings, Some(vec![]), Some(vec![]), None, None]);
    assert_eq!(ben.exceptions.iter().collect::<Vec<_>>(), [(&d(2026, 3, 4), &vec![])], "a day's leave");
    assert!(p.resource(ResourceId(1)).unwrap().calendar.is_empty(), "Ann's calendar is the project calendar as it is");
}

#[test]
fn dropped_rate_tables_flattened_contours_and_rounding_are_report_lines() {
    let i = mspdi(PATIO);
    let b = find(&i, "Cost rate table B");
    assert_eq!((b.outcome, b.place.as_str()), (Outcome::Dropped, "Ann Lee"));
    assert!(lines(&i, "Cost rate table C").is_empty(), "a table of nothing but zeros is no loss");
    let bell = find(&i, "Work contour: Bell");
    assert_eq!((bell.outcome, bell.place.as_str()), (Outcome::Converted, "Ann Lee on ID 1 Dig the footings"));
    assert!(bell.why.contains("flat"), "{}", bell.why);
    let rounded = find(&i, "Amounts finer than the currency");
    assert_eq!((rounded.outcome, rounded.place.as_str()), (Outcome::Converted, "Ann Lee"));
    assert!(i.report.lines.iter().all(|l| l.outcome != Outcome::NotYetImported), "{:#?}", i.report.lines);
    assert_eq!(i.report.lines.len(), 4, "{:#?}", i.report.lines);
    assert!(lines(&i, "Project summary task").len() == 1);
}

#[test]
fn every_contour_but_flat_and_every_table_but_a_has_its_own_line() {
    let rates = |table: u8| {
        format!(
            "<Rate><RatesFrom>1984-01-01T00:00:00</RatesFrom><RatesTo>2049-12-31T23:59:00</RatesTo><RateTable>{table}</RateTable><StandardRate>1</StandardRate></Rate>"
        )
    };
    let all: String = (0..=4).map(rates).collect();
    let resources = resource(1, "Ann", &format!("<Rates>{all}</Rates>")) + &resource(2, "Bob", &format!("<Rates>{}</Rates>", rates(3)));
    let tasks: Vec<(i64, &str, u32)> = (1..=9).map(|n| (n, "T", 8)).collect();
    let assignments: String = (0..=8).map(|c| assignment(c + 1, 1, &format!("<WorkContour>{c}</WorkContour>"))).collect();
    let i = mspdi(&plan("", &tasks, &resources, &assignments));
    for name in ["Back Loaded", "Front Loaded", "Double Peak", "Early Peak", "Late Peak", "Bell", "Turtle", "Contoured"] {
        assert_eq!(find(&i, &format!("Work contour: {name}")).outcome, Outcome::Converted, "{name}");
    }
    assert!(lines(&i, "Work contour: Flat").is_empty());
    for (table, place) in [("B", "Ann"), ("C", "Ann"), ("D", "Ann, Bob"), ("E", "Ann")] {
        let l = find(&i, &format!("Cost rate table {table}"));
        assert_eq!((l.outcome, l.place.as_str()), (Outcome::Dropped, place), "{table}");
    }
    assert!(lines(&i, "Cost rate table A").is_empty());
}

#[test]
fn an_assignment_on_another_rate_table_is_costed_at_table_a() {
    let resources = resource(1, "Ann", "<StandardRate>10</StandardRate>");
    let i = mspdi(&plan("", &[(1, "Dig", 8)], &resources, &assignment(1, 1, "<CostRateTable>2</CostRateTable>")));
    assert_eq!(find(&i, "Cost rate table on assignments").outcome, Outcome::Converted);
    assert_eq!(i.project.total_cost(), 8000);
}

#[test]
fn the_currency_comes_in_with_its_decimal_places() {
    let extra = "<CurrencySymbol>¥</CurrencySymbol><CurrencyDigits>0</CurrencyDigits><CurrencyCode>JPY</CurrencyCode>";
    let resources = resource(1, "Kenji", "<StandardRate>1500</StandardRate><CostPerUse>250.5</CostPerUse>");
    let i = mspdi(
        &plan(extra, &[(1, "Survey", 8)], &resources, &assignment(1, 1, ""))
            .replace("<Name>Survey</Name>", "<Name>Survey</Name><FixedCost>99.4</FixedCost>"),
    );
    let p = &i.project;
    assert_eq!((p.currency.code.as_str(), p.currency.symbol.as_str(), p.currency.digits), ("JPY", "¥", 0));
    // 8 h × ¥1,500, ¥250.5 per use is ¥250 (half to even), ¥99.4 fixed is ¥99.
    assert_eq!(p.total_cost(), 12_000 + 250 + 99);
    let rounded: Vec<&str> = lines(&i, "Amounts finer than the currency").iter().map(|l| l.place.as_str()).collect();
    assert_eq!(rounded, ["ID 1 Survey", "Kenji"], "the fixed cost and the resource's cost per use");

    let bahrain = mspdi(&plan(
        "<CurrencyCode>BHD</CurrencyCode><CurrencyDigits>3</CurrencyDigits>",
        &[(1, "Survey", 8)],
        &resource(1, "Ali", "<StandardRate>1.125</StandardRate>"),
        &assignment(1, 1, ""),
    ));
    assert_eq!(bahrain.project.currency.digits, 3);
    assert_eq!(bahrain.project.total_cost(), 9_000, "8 h × 1.125 dinars, in fils");
    assert!(bahrain.report.lines.iter().all(|l| l.feature != "Currency"), "{:#?}", bahrain.report.lines);

    for (bad, why) in
        [("<CurrencyCode>POUNDS</CurrencyCode>", "code"), ("<CurrencyCode>EUR</CurrencyCode><CurrencySymbol>12</CurrencySymbol>", "symbol")]
    {
        let i = mspdi(&plan(bad, &[(1, "Survey", 8)], "", ""));
        let l = find(&i, "Currency");
        assert_eq!(l.outcome, Outcome::Converted, "{why}");
        if why == "symbol" {
            assert_eq!((i.project.currency.code.as_str(), i.project.currency.symbol.as_str()), ("EUR", "€"));
        } else {
            assert_eq!(i.project.currency, model::Currency::default());
        }
    }
}

#[test]
fn a_resource_left_on_another_base_calendar_or_one_not_in_the_file_is_reported() {
    let calendars = "<Calendars><Calendar><UID>1</UID><Name>Standard</Name><IsBaseCalendar>1</IsBaseCalendar></Calendar>\
<Calendar><UID>7</UID><Name>Night shift</Name><IsBaseCalendar>1</IsBaseCalendar><WeekDays><WeekDay><DayType>7</DayType><DayWorking>1</DayWorking>\
<WorkingTimes><WorkingTime><FromTime>08:00:00</FromTime><ToTime>12:00:00</ToTime></WorkingTime></WorkingTimes></WeekDay></WeekDays></Calendar>\
<Calendar><UID>8</UID><Name>Cara</Name><IsBaseCalendar>0</IsBaseCalendar><BaseCalendarUID>7</BaseCalendarUID></Calendar>\
<Calendar><UID>9</UID><Name>Nobody</Name><IsBaseCalendar>0</IsBaseCalendar><BaseCalendarUID>1</BaseCalendarUID></Calendar></Calendars>";
    let resources = resource(1, "Cara", "<CalendarUID>8</CalendarUID>") + &resource(2, "Dev", "<CalendarUID>44</CalendarUID>");
    let i = mspdi(&plan(&format!("<CalendarUID>1</CalendarUID>{calendars}"), &[(1, "Dig", 8)], &resources, ""));
    let cara = &i.project.resource(ResourceId(1)).unwrap().calendar;
    assert_eq!(cara.week[5], Some(vec![t(480, 720)]), "the base calendar's own Saturday is copied");
    let based = find(&i, "Resource calendar base");
    assert_eq!((based.outcome, based.place.as_str()), (Outcome::Converted, "Cara"));
    let missing = find(&i, "Resource calendar");
    assert_eq!((missing.outcome, missing.place.as_str()), (Outcome::Converted, "Dev"));
    let unused = find(&i, "Resource calendars");
    assert_eq!((unused.outcome, unused.place.as_str()), (Outcome::Dropped, "Nobody (UID 9)"));
}

#[test]
fn hostile_resources_are_clamped_renamed_or_dropped_with_a_line_each() {
    let resources = [
        resource(1, "Ann, the [boss]", "<MaxUnits>1e9</MaxUnits><StandardRate>-5</StandardRate><CostPerUse>99999999999999999999</CostPerUse>"),
        resource(2, "ann, the (boss)", "<Type>7</Type><AccrueAt>9</AccrueAt>"),
        resource(3, "", "<MaxUnits>abc</MaxUnits>"),
        resource(4, "Null", "<IsNull>1</IsNull>"),
        resource(5, "Mud", "<Type>0</Type><MaterialLabel>buckets</MaterialLabel>"),
    ]
    .concat();
    let assignments = [
        assignment(1, 1, "<Units>1e30</Units><Work>PT99999999999H</Work>"),
        assignment(1, 1, ""),
        assignment(1, 99, ""),
        assignment(99, 1, ""),
        assignment(0, 1, ""),
        assignment(2, 2, "<Units>-3</Units><Work>junk</Work>"),
        assignment(2, 5, "<Units>2.5</Units><FixedMaterial>0</FixedMaterial>"),
    ]
    .concat();
    let i = mspdi(&plan("", &[(1, "Dig", 8), (2, "Fill", 8)], &resources, &assignments));
    let p = &i.project;
    let names: Vec<&str> = p.resources.iter().map(|r| r.name.as_str()).collect();
    assert_eq!(names, ["Ann; the (boss)", "ann; the (boss) (2)", "Resource 3", "Mud"]);
    let ann = &p.resources[0];
    assert_eq!((ann.max_units, ann.standard_rate, ann.cost_per_use), (MAX_UNITS, 0, MAX_MONEY));
    assert_eq!((p.resources[1].kind, p.resources[1].accrue_at), (ResourceType::Work, AccrueAt::Prorated));
    assert_eq!(p.resources[2].max_units, FULL_TIME);
    let a: Vec<(u32, u32, Units)> = p.assignments.iter().map(|a| (a.task.0, a.resource.0, a.units)).collect();
    assert_eq!(a, [(1, 1, Units::Work(MAX_UNITS)), (2, 2, Units::Work(0)), (2, 5, Units::Fixed(250))]);
    assert!(p.assignments.iter().all(|a| (0..=model::MAX_WORK).contains(&a.work) && (0..=MAX_MINUTES).contains(&a.duration)));
    assert!(p.tasks.iter().all(|t| (0..=MAX_MINUTES).contains(&t.duration)));
    for (feature, outcome) in [
        ("Resource names", Outcome::Converted),
        ("Resource type", Outcome::Converted),
        ("Max Units", Outcome::Converted),
        ("Rates and costs", Outcome::Converted),
        ("Accrue At", Outcome::Converted),
        ("Blank resource rows", Outcome::Dropped),
        ("Assignment units", Outcome::Converted),
        ("Assignment work", Outcome::Converted),
        ("Variable material consumption", Outcome::Converted),
    ] {
        assert_eq!(find(&i, feature).outcome, outcome, "{feature}");
    }
    let dropped: Vec<(&str, &str)> = lines(&i, "Assignments").iter().map(|l| (l.place.as_str(), l.why.as_str())).collect();
    assert_eq!(dropped.len(), 4, "{dropped:#?}");
    assert!(model::schedule(p).is_ok());
}

#[test]
fn resource_and_assignment_counts_are_capped() {
    let resources: String = (1..=MAX_RESOURCES as i64 + 2).map(|n| format!("<Resource><UID>{n}</UID><Name>R{n}</Name></Resource>")).collect();
    let i = mspdi(&plan("", &[(1, "Dig", 8)], &resources, ""));
    assert_eq!(i.project.resources.len(), MAX_RESOURCES);
    let l = find(&i, "Resources");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Dropped, "2 resources"));
    let resources: String = (1..=1_002).map(|n| format!("<Resource><UID>{n}</UID><Name>R{n}</Name></Resource>")).collect();
    let assignments: String = (1..=1_002).map(|n| assignment(1, n, "")).collect();
    let i = mspdi(&plan("", &[(1, "Dig", 8)], &resources, &assignments));
    assert_eq!(i.project.assignments.len(), model::MAX_TASK_ASSIGNMENTS);
    assert!(lines(&i, "Assignments").iter().any(|l| l.place == "2 assignments"), "{:#?}", lines(&i, "Assignments"));
}

#[test]
fn a_task_s_duration_follows_its_assignments_with_a_line() {
    // The file says 1 day, but Ann works 16 hours at 100%.
    let i = mspdi(&plan("", &[(1, "Dig", 8)], &resource(1, "Ann", ""), &assignment(1, 1, "<Units>1</Units><Work>PT16H</Work>")));
    assert_eq!(i.project.tasks[0].duration, 16 * 60);
    let l = find(&i, "Duration from assignments");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Converted, "ID 1 Dig"));
    // No work in the file: the task's duration at the assignment's units.
    let i = mspdi(&plan("", &[(1, "Dig", 8)], &resource(1, "Ann", ""), &assignment(1, 1, "<Units>0.5</Units>")));
    assert_eq!((i.project.assignments[0].work, i.project.tasks[0].duration), (4 * 60, 8 * 60));
    assert!(lines(&i, "Duration from assignments").is_empty());
}

#[test]
fn fixed_costs_are_imported_and_hostile_ones_clamped() {
    let file = plan("", &[(1, "Dig", 8), (2, "Fill", 8), (3, "Level", 8)], "", "")
        .replace("<Name>Dig</Name>", "<Name>Dig</Name><FixedCost>1099.5</FixedCost>")
        .replace("<Name>Fill</Name>", "<Name>Fill</Name><FixedCost>-20</FixedCost><FixedCostAccrual>1</FixedCostAccrual>")
        .replace("<Name>Level</Name>", "<Name>Level</Name><FixedCost>1e40</FixedCost>");
    let i = mspdi(&file);
    let fixed: Vec<i64> = i.project.tasks.iter().map(|t| t.fixed_cost).collect();
    assert_eq!(fixed, [109950, 0, MAX_MONEY]);
    let l = find(&i, "Fixed cost");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Converted, "ID 2 Fill, ID 3 Level"));
    assert_eq!(find(&i, "Fixed cost accrual").place, "ID 2 Fill");
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(512))]

    #[test]
    fn a_damaged_costed_file_never_panics(at in 0usize..PATIO.len(), cut in 0usize..64, junk in "\\PC{0,24}") {
        let mut text = String::new();
        let at = (0..=at).rev().find(|i| PATIO.is_char_boundary(*i)).unwrap_or(0);
        let end = (at..=at.saturating_add(cut).min(PATIO.len())).rev().find(|i| PATIO.is_char_boundary(*i)).unwrap_or(at);
        text.push_str(&PATIO[..at]);
        text.push_str(&junk);
        text.push_str(&PATIO[end..]);
        if let Ok(i) = import(Format::Mspdi, text.as_bytes()) {
            let mut p = i.project.clone();
            proptest::prop_assert!(p.check_resources().is_ok());
            proptest::prop_assert!(p.resources.iter().all(|r| (0..=MAX_MONEY).contains(&r.standard_rate) && r.max_units <= MAX_UNITS));
            proptest::prop_assert!(p.tasks.iter().all(|t| (0..=MAX_MONEY).contains(&t.fixed_cost)));
            let _ = p.total_cost();
            let _ = model::schedule(&p);
        }
    }

    #[test]
    fn arbitrary_numbers_in_money_and_units_fields_never_panic(rate in "[-+0-9.eE]{0,30}", units in "[-+0-9.eE]{0,30}", work in "[-PTHMS0-9.]{0,24}") {
        let resources = resource(1, "Ann", &format!("<StandardRate>{rate}</StandardRate><MaxUnits>{units}</MaxUnits><CostPerUse>{rate}</CostPerUse>"));
        let i = mspdi(&plan("", &[(1, "Dig", 8)], &resources, &assignment(1, 1, &format!("<Units>{units}</Units><Work>{work}</Work>"))));
        let mut p = i.project.clone();
        proptest::prop_assert!(p.check_resources().is_ok());
        let _ = p.total_cost();
        proptest::prop_assert!(model::schedule(&p).is_ok());
    }
}
