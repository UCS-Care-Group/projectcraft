//! MSPDI task fields: constraints, deadlines, task types and effort-driven imported then
//! scheduled; manual tasks made auto-scheduled; tracking and other fields ProjectCraft doesn't
//! hold dropped with a report line each. Hand-written files built from Microsoft's published
//! schema docs (<https://learn.microsoft.com/office-project/xml-data-interchange/task-element>).
//! The plans are invented.

use projectcraft_interchange::model::{self, Constraint, Date, Driver, DurationUnit, MAX_DAYS, ResourceType, TaskId, TaskType};
use projectcraft_interchange::{Format, Import, Outcome, ReportLine, import};

fn mspdi(text: &str) -> Import {
    import(Format::Mspdi, text.as_bytes()).unwrap()
}

fn d(y: i32, m: u32, day: u32) -> Date {
    Date::from_ymd(y, m, day).unwrap()
}

fn lines<'a>(i: &'a Import, feature: &str) -> Vec<&'a ReportLine> {
    i.report.lines.iter().filter(|l| l.feature == feature).collect()
}

/// The report's lines but the one every file without a calendar has.
fn others(i: &Import) -> Vec<&ReportLine> {
    i.report.lines.iter().filter(|l| l.feature != "Project calendar").collect()
}

fn find<'a>(i: &'a Import, feature: &str) -> &'a ReportLine {
    let found = lines(i, feature);
    assert_eq!(found.len(), 1, "one `{feature}` line in {:#?}", i.report.lines);
    found[0]
}

/// A project starting Monday 2026-03-02 on the default calendar (Monday to Friday, 8:00–12:00 and
/// 13:00–17:00) and `extra` project fields, with tasks of `(uid, name, hours, inside)`, `inside`
/// being the task's other fields as the file writes them.
fn plan_with(extra: &str, tasks: &[(i64, &str, u32, &str)]) -> String {
    let mut s = format!(
        "<Project xmlns=\"http://schemas.microsoft.com/project\"><StartDate>2026-03-02T08:00:00</StartDate><MinutesPerDay>480</MinutesPerDay>{extra}<Tasks>"
    );
    for (uid, name, hours, inside) in tasks {
        s.push_str(&format!(
            "<Task><UID>{uid}</UID><ID>{uid}</ID><Name>{name}</Name><OutlineLevel>1</OutlineLevel><Duration>PT{hours}H0M0S</Duration>\
<DurationFormat>7</DurationFormat>{inside}</Task>"
        ));
    }
    s.push_str("</Tasks></Project>");
    s
}

fn plan(tasks: &[(i64, &str, u32, &str)]) -> String {
    plan_with("", tasks)
}

fn constraint(code: u8, date: &str) -> String {
    format!("<ConstraintType>{code}</ConstraintType><ConstraintDate>{date}</ConstraintDate>")
}

/// (start, finish, late start, late finish) of a task.
fn dates(s: &model::Schedule, uid: u32) -> (Date, Date, Date, Date) {
    let t = &s.tasks[&TaskId(uid)];
    (t.start, t.finish, t.late_start, t.late_finish)
}

#[test]
fn every_constraint_type_is_imported_and_scheduled() {
    // A ten-day task sets the project finish (Friday 2026-03-13); each other task is two days.
    let i = mspdi(&plan(&[
        (1, "Lay the patio", 80, ""),
        (2, "Clear the pond", 16, &constraint(0, "2026-03-09T08:00:00")),
        (3, "Fit the gate", 16, &constraint(2, "2026-03-10T08:00:00")),
        (4, "Paint the fence", 16, &constraint(3, "2026-03-13T17:00:00")),
        (5, "Plant the beds", 16, &constraint(4, "2026-03-05T08:00:00")),
        (6, "Order turf", 16, &constraint(5, "2026-03-05T08:00:00")),
        (7, "Hang the lights", 16, &constraint(6, "2026-03-11T17:00:00")),
        (8, "Book a skip", 16, &constraint(7, "2026-03-04T17:00:00")),
    ]));
    let c: Vec<Constraint> = i.project.tasks.iter().map(|t| t.constraint).collect();
    assert_eq!(
        c,
        [
            Constraint::Asap,
            Constraint::Asap,
            Constraint::MustStartOn(d(2026, 3, 10)),
            Constraint::MustFinishOn(d(2026, 3, 13)),
            Constraint::StartNoEarlierThan(d(2026, 3, 5)),
            Constraint::StartNoLaterThan(d(2026, 3, 5)),
            Constraint::FinishNoEarlierThan(d(2026, 3, 11)),
            Constraint::FinishNoLaterThan(d(2026, 3, 4)),
        ]
    );
    assert!(others(&i).is_empty(), "every constraint is imported as it is: {:#?}", others(&i));
    let s = model::schedule(&i.project).unwrap();
    assert_eq!(s.finish, d(2026, 3, 13));
    // As Soon As Possible: the project start; its date is ignored.
    assert_eq!(dates(&s, 2), (d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 12), d(2026, 3, 13)));
    // Must Start On and Must Finish On fix the task to their dates, early and late.
    assert_eq!(dates(&s, 3), (d(2026, 3, 10), d(2026, 3, 11), d(2026, 3, 10), d(2026, 3, 11)));
    assert_eq!(dates(&s, 4), (d(2026, 3, 12), d(2026, 3, 13), d(2026, 3, 12), d(2026, 3, 13)));
    // Start No Earlier Than moves the start; Finish No Earlier Than the finish.
    assert_eq!(dates(&s, 5).0, d(2026, 3, 5));
    assert_eq!(dates(&s, 7).1, d(2026, 3, 11));
    // Start and Finish No Later Than leave the task at the start but cap its late dates.
    assert_eq!(dates(&s, 6), (d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 5), d(2026, 3, 6)));
    assert_eq!(dates(&s, 8), (d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 3), d(2026, 3, 4)));
    for uid in [3, 4, 5, 7] {
        assert_eq!(s.tasks[&TaskId(uid)].driver, Driver::Constraint, "task {uid}");
    }
}

#[test]
fn as_late_as_possible_is_imported_and_scheduled() {
    // Sow the lawn (2 days) sets the finish, Tuesday 3 March; Rake (1 day) As Late As Possible
    // takes its late dates, the Tuesday. A date given with it is ignored, as Project does.
    let i = mspdi(&plan(&[(1, "Sow the lawn", 16, "<ConstraintType>1</ConstraintType>"), (2, "Rake", 8, &constraint(1, "2026-03-20T08:00:00"))]));
    assert!(i.project.tasks.iter().all(|t| t.constraint == Constraint::AsLateAsPossible));
    assert!(others(&i).is_empty(), "imported as it is: {:#?}", others(&i));
    let s = model::schedule(&i.project).unwrap();
    assert_eq!(dates(&s, 1), (d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 2), d(2026, 3, 3)));
    assert_eq!(dates(&s, 2), (d(2026, 3, 3), d(2026, 3, 3), d(2026, 3, 3), d(2026, 3, 3)));
    assert_eq!(s.tasks[&TaskId(2)].driver, Driver::Constraint);
}

#[test]
fn deadlines_are_imported_and_scheduled() {
    let i = mspdi(&plan(&[
        (1, "Lay the patio", 80, ""),
        (2, "Seal the deck", 16, "<Deadline>2026-03-04T17:00:00</Deadline>"),
        (3, "Fix the shed roof", 24, "<Deadline>2026-03-03T17:00:00</Deadline>"),
    ]));
    let deadlines: Vec<Option<Date>> = i.project.tasks.iter().map(|t| t.deadline).collect();
    assert_eq!(deadlines, [None, Some(d(2026, 3, 4)), Some(d(2026, 3, 3))]);
    assert!(others(&i).is_empty(), "{:#?}", others(&i));
    let s = model::schedule(&i.project).unwrap();
    // A deadline never moves the task; it caps its late finish.
    let met = &s.tasks[&TaskId(2)];
    assert_eq!((met.start, met.finish, met.late_finish, met.slack, met.deadline_missed), (d(2026, 3, 2), d(2026, 3, 3), d(2026, 3, 4), 480, false));
    // Three days can't finish by the second day: missed, with a day of negative slack.
    let missed = &s.tasks[&TaskId(3)];
    assert_eq!((missed.start, missed.finish, missed.slack, missed.deadline_missed), (d(2026, 3, 2), d(2026, 3, 4), -480, true));
}

#[test]
fn task_types_and_effort_driven_are_imported() {
    let i = mspdi(&plan(&[
        (1, "Units", 16, "<Type>0</Type><EffortDriven>0</EffortDriven>"),
        (2, "Duration", 16, "<Type>1</Type><EffortDriven>1</EffortDriven>"),
        (3, "Work", 16, "<Type>2</Type><EffortDriven>1</EffortDriven>"),
        (4, "Units, effort-driven", 16, "<Type>0</Type><EffortDriven>1</EffortDriven>"),
        (5, "No type", 16, ""),
    ]));
    let got: Vec<(TaskType, bool)> = i.project.tasks.iter().map(|t| (t.task_type, t.effort_driven)).collect();
    assert_eq!(
        got,
        [
            (TaskType::FixedUnits, false),
            (TaskType::FixedDuration, true),
            (TaskType::FixedWork, true),
            (TaskType::FixedUnits, true),
            (TaskType::FixedUnits, false),
        ]
    );
    assert!(others(&i).is_empty(), "{:#?}", others(&i));
}

/// The imported project with Ann and Ben as work resources and Ann on `uid` at 100%.
fn staffed(i: Import, uid: u32) -> (model::Project, model::ResourceId, model::ResourceId) {
    let mut p = i.project;
    let ann = p.add_resource(Some("Ann"), ResourceType::Work, None).unwrap();
    let ben = p.add_resource(Some("Ben"), ResourceType::Work, None).unwrap();
    p.assign(TaskId(uid), ann, None).unwrap();
    (p, ann, ben)
}

#[test]
fn an_imported_task_type_decides_what_a_change_keeps() {
    // Fixed Work: four days at 100% is 32 hours of work; a new duration keeps the work.
    let fixed_work = mspdi(&plan(&[(1, "Paint the studio", 32, "<Type>2</Type><EffortDriven>1</EffortDriven>")]));
    let (mut p, ..) = staffed(fixed_work, 1);
    assert_eq!(p.task_work(TaskId(1)), 32 * 60);
    p.set_duration(TaskId(1), 16 * 60, DurationUnit::Days).unwrap();
    assert_eq!(p.task_work(TaskId(1)), 32 * 60, "Fixed Work keeps the work");
    assert_eq!(model::schedule(&p).unwrap().tasks[&TaskId(1)].finish, d(2026, 3, 3));
    // Fixed Units: the same change keeps the units and halves the work.
    let fixed_units = mspdi(&plan(&[(1, "Paint the studio", 32, "<Type>0</Type><EffortDriven>0</EffortDriven>")]));
    let (mut p, ..) = staffed(fixed_units, 1);
    p.set_duration(TaskId(1), 16 * 60, DurationUnit::Days).unwrap();
    assert_eq!(p.task_work(TaskId(1)), 16 * 60, "Fixed Units recalculates the work");
    // Fixed Duration: a second resource adds work and keeps the four days.
    let fixed_duration = mspdi(&plan(&[(1, "Paint the studio", 32, "<Type>1</Type><EffortDriven>0</EffortDriven>")]));
    let (mut p, _, ben) = staffed(fixed_duration, 1);
    p.assign(TaskId(1), ben, None).unwrap();
    assert_eq!((p.task(TaskId(1)).unwrap().duration, p.task_work(TaskId(1))), (32 * 60, 64 * 60));
    assert_eq!(model::schedule(&p).unwrap().tasks[&TaskId(1)].finish, d(2026, 3, 5));
}

#[test]
fn an_imported_effort_driven_task_shares_its_work() {
    // Effort-driven: Ben joins Ann and the four days of work are done in two.
    let i = mspdi(&plan(&[(1, "Dig the pond", 32, "<Type>0</Type><EffortDriven>1</EffortDriven>")]));
    let (mut p, _, ben) = staffed(i, 1);
    p.assign(TaskId(1), ben, None).unwrap();
    assert_eq!((p.task(TaskId(1)).unwrap().duration, p.task_work(TaskId(1))), (16 * 60, 32 * 60));
    assert_eq!(model::schedule(&p).unwrap().tasks[&TaskId(1)].finish, d(2026, 3, 3));
    // Not effort-driven: Ben adds his own work and the task still takes four days.
    let i = mspdi(&plan(&[(1, "Dig the pond", 32, "<Type>0</Type><EffortDriven>0</EffortDriven>")]));
    let (mut p, _, ben) = staffed(i, 1);
    p.assign(TaskId(1), ben, None).unwrap();
    assert_eq!((p.task(TaskId(1)).unwrap().duration, p.task_work(TaskId(1))), (32 * 60, 64 * 60));
    assert_eq!(model::schedule(&p).unwrap().tasks[&TaskId(1)].finish, d(2026, 3, 5));
}

#[test]
fn the_project_s_default_task_type_and_effort_driven_are_imported() {
    let i = mspdi(&plan_with("<DefaultTaskType>2</DefaultTaskType><NewTasksEffortDriven>1</NewTasksEffortDriven>", &[(1, "Mow", 8, "")]));
    assert_eq!((i.project.options.task_type, i.project.options.effort_driven), (TaskType::FixedWork, true));
    assert!(others(&i).is_empty(), "{:#?}", others(&i));
    let i = mspdi(&plan_with("<DefaultTaskType>1</DefaultTaskType><NewTasksEffortDriven>0</NewTasksEffortDriven>", &[(1, "Mow", 8, "")]));
    assert_eq!((i.project.options.task_type, i.project.options.effort_driven), (TaskType::FixedDuration, false));
    let i = mspdi(&plan_with("<DefaultTaskType>9</DefaultTaskType>", &[(1, "Mow", 8, "")]));
    assert_eq!(i.project.options.task_type, TaskType::FixedUnits);
    assert_eq!(find(&i, "Default task type").outcome, Outcome::Converted);
}

#[test]
fn hostile_constraints_deadlines_and_types_are_reported_never_an_error() {
    let i = mspdi(&plan(&[
        (1, "Unknown code", 8, &constraint(9, "2026-03-05T08:00:00")),
        (2, "No date", 8, "<ConstraintType>4</ConstraintType>"),
        (3, "Unreadable date", 8, &constraint(7, "next Tuesday")),
        (4, "Far future", 8, &constraint(2, "9999-12-31T08:00:00")),
        (5, "Far past", 8, &constraint(3, "0001-01-01T17:00:00")),
        (6, "Bad deadline", 8, "<Deadline>soon</Deadline>"),
        (7, "Far deadline", 8, "<Deadline>9999-12-31T17:00:00</Deadline>"),
        (8, "Odd type", 8, "<Type>7</Type>"),
        (9, "Lazy work", 8, "<Type>2</Type><EffortDriven>0</EffortDriven>"),
        (10, "Negative code", 8, &constraint(255, "2026-03-05T08:00:00")),
        (11, "Words", 8, "<ConstraintType>MSO</ConstraintType><Type>fixed</Type>"),
    ]));
    let t = |uid: u32| i.project.task(TaskId(uid)).unwrap();
    let start = d(2026, 3, 2);
    assert!([1, 2, 3, 10, 11].iter().all(|&u| t(u).constraint.is_asap()));
    assert_eq!(t(4).constraint, Constraint::MustStartOn(start.add_days(MAX_DAYS).unwrap()));
    assert_eq!(t(5).constraint, Constraint::MustFinishOn(start.add_days(-MAX_DAYS).unwrap()));
    assert_eq!(t(6).deadline, None);
    assert_eq!(t(7).deadline, start.add_days(MAX_DAYS));
    assert_eq!((t(8).task_type, t(11).task_type), (TaskType::FixedUnits, TaskType::FixedUnits));
    assert_eq!((t(9).task_type, t(9).effort_driven), (TaskType::FixedWork, true));
    let expect = [
        ("Constraint", Outcome::Converted, "ID 1 Unknown code, ID 10 Negative code, ID 11 Words"),
        ("Constraint date", Outcome::Converted, "ID 2 No date, ID 3 Unreadable date"),
        ("Deadline", Outcome::Dropped, "ID 6 Bad deadline"),
        ("Task type", Outcome::Converted, "ID 8 Odd type, ID 11 Words"),
        ("Effort-driven", Outcome::Converted, "ID 9 Lazy work"),
    ];
    for (feature, outcome, place) in expect {
        let l = lines(&i, feature);
        assert!(l.iter().any(|l| (l.outcome, l.place.as_str()) == (outcome, place)), "{feature}: {l:#?}");
    }
    let far: Vec<&str> = i.report.lines.iter().filter(|l| l.why.contains("100,000 days")).map(|l| l.place.as_str()).collect();
    assert_eq!(far, ["ID 4 Far future, ID 5 Far past", "ID 7 Far deadline"]);
    // Scheduling the result never panics; a Must Start On 100,000 days out is honoured or refused.
    let _ = model::schedule(&i.project);
}

#[test]
fn a_manual_task_becomes_auto_scheduled_reported() {
    let i = mspdi(&plan(&[
        (1, "Lay the patio", 16, ""),
        (2, "Fit the gate", 16, "<Manual>1</Manual><Start>2026-03-20T08:00:00</Start><Finish>2026-03-23T17:00:00</Finish>"),
    ]));
    let l = find(&i, "Manually scheduled");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Converted, "ID 2 Fit the gate"));
    assert!(l.why.contains("auto-scheduled"), "{}", l.why);
    // Its dates come from the scheduler, not the file's manual ones.
    let s = model::schedule(&i.project).unwrap();
    assert_eq!((s.tasks[&TaskId(2)].start, s.tasks[&TaskId(2)].finish), (d(2026, 3, 2), d(2026, 3, 3)));
}

/// The one line `inside` a task makes, and that the task is still imported.
fn dropped(inside: &str, feature: &str) -> ReportLine {
    let i = mspdi(&plan(&[(1, "Lay the patio", 16, ""), (2, "Fit the gate", 16, inside)]));
    assert_eq!(i.project.tasks.len(), 2);
    assert_eq!(others(&i).len(), 1, "{:#?}", others(&i));
    let l = find(&i, feature).clone();
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Dropped, "ID 2 Fit the gate"), "{feature}");
    assert!(!l.why.is_empty());
    l
}

#[test]
fn baselines_are_dropped_reported() {
    dropped("<Baseline><Number>0</Number><Start>2026-03-02T08:00:00</Start><Duration>PT16H0M0S</Duration></Baseline>", "Baselines");
}

#[test]
fn actuals_are_dropped_reported() {
    for actual in [
        "<ActualStart>2026-03-02T08:00:00</ActualStart>",
        "<ActualFinish>2026-03-03T17:00:00</ActualFinish>",
        "<ActualDuration>PT8H0M0S</ActualDuration>",
        "<ActualWork>PT8H0M0S</ActualWork>",
        "<ActualCost>12500</ActualCost>",
    ] {
        dropped(actual, "Progress (actuals, % complete)");
    }
    // Zero actuals, as Project writes for a task not started, aren't progress.
    let i = mspdi(&plan(&[(
        1,
        "Fit the gate",
        16,
        "<ActualDuration>PT0H0M0S</ActualDuration><ActualWork>PT0H0M0S</ActualWork><ActualCost>0</ActualCost>",
    )]));
    assert!(others(&i).is_empty(), "{:#?}", others(&i));
}

#[test]
fn percent_complete_is_dropped_reported() {
    for pc in [
        "<PercentComplete>50</PercentComplete>",
        "<PercentWorkComplete>25</PercentWorkComplete>",
        "<PhysicalPercentComplete>10</PhysicalPercentComplete>",
    ] {
        dropped(pc, "Progress (actuals, % complete)");
    }
}

#[test]
fn notes_are_dropped_reported() {
    dropped("<Notes>Ask Priya about the hinges</Notes>", "Notes");
}

#[test]
fn hyperlinks_are_dropped_reported() {
    dropped("<Hyperlink>Gate catalogue</Hyperlink><HyperlinkAddress>https://example.org/gates</HyperlinkAddress>", "Hyperlink");
}

#[test]
fn custom_fields_are_dropped_reported() {
    dropped("<ExtendedAttribute><FieldID>188743731</FieldID><Value>Garden</Value></ExtendedAttribute>", "Custom fields");
}

#[test]
fn priority_and_leveling_delay_are_imported() {
    // Priority 0 to 1,000; LevelingDelay in tenths of a minute (9,600 is two 8-hour days).
    let i = mspdi(&plan(&[
        (1, "Lay the patio", 16, "<Priority>800</Priority><LevelingDelay>9600</LevelingDelay><LevelingDelayFormat>7</LevelingDelayFormat>"),
        (2, "Fit the gate", 16, "<Priority>500</Priority><LevelingDelay>0</LevelingDelay>"),
    ]));
    assert!(others(&i).is_empty(), "{:#?}", others(&i));
    let got: Vec<(u32, i64)> = i.project.tasks.iter().map(|t| (t.priority, t.leveling_delay)).collect();
    assert_eq!(got, [(800, 960), (500, 0)]);
    // Out of range or unreadable: brought within it, a line each.
    let i = mspdi(&plan(&[
        (1, "Lay the patio", 16, "<Priority>1200</Priority><LevelingDelay>-50</LevelingDelay>"),
        (2, "Fit the gate", 16, "<Priority>high</Priority><LevelingDelay>9600</LevelingDelay><LevelingDelayFormat>8</LevelingDelayFormat>"),
        (3, "Paint the shed", 16, "<LevelingDelay>500</LevelingDelay><LevelingDelayFormat>19</LevelingDelayFormat>"),
    ]));
    let got: Vec<(u32, i64)> = i.project.tasks.iter().map(|t| (t.priority, t.leveling_delay)).collect();
    // 2 elapsed days (9,600 tenths is 16 hours of calendar time, 2/3 of an elapsed day) read as
    // the same number of working days: 320 working minutes.
    assert_eq!(got, [(1000, 0), (500, 320), (500, 0)]);
    let p = find(&i, "Priority");
    assert_eq!((p.outcome, p.place.as_str()), (Outcome::Converted, "ID 1 Lay the patio, ID 2 Fit the gate"));
    let l = find(&i, "Leveling delay");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Converted, "ID 1 Lay the patio, ID 2 Fit the gate, ID 3 Paint the shed"));
}

#[test]
fn splits_are_dropped_reported() {
    dropped("<Stop>2026-03-02T17:00:00</Stop><Resume>2026-03-05T08:00:00</Resume><ResumeValid>1</ResumeValid>", "Split (Stop, Resume)");
    // Stopping and resuming at the same moment is no split.
    let same = "<Stop>2026-03-02T17:00:00</Stop><Resume>2026-03-02T17:00:00</Resume>";
    assert!(others(&mspdi(&plan(&[(1, "Fit the gate", 16, same)]))).is_empty());
}

#[test]
fn recurring_tasks_are_dropped_reported() {
    let l = dropped("<Recurring>1</Recurring>", "Recurring task");
    assert!(l.why.contains("occurrences"), "{}", l.why);
}

proptest::proptest! {
    #![proptest_config(proptest::prelude::ProptestConfig::with_cases(256))]

    /// Any codes and dates import, keep to the model's bounds and schedule or refuse, never panic.
    #[test]
    fn arbitrary_task_fields_never_panic(
        kind in -3i64..12,
        task_type in -3i64..6,
        effort in proptest::bool::ANY,
        year in 1i32..=9999,
        day in 0i64..400,
        junk in "[a-zA-Z0-9 :T-]{0,24}",
    ) {
        let date = Date::from_ymd(year, 1, 1).unwrap().add_days(day).unwrap_or(Date::EPOCH);
        let inside = format!(
            "<ConstraintType>{kind}</ConstraintType><ConstraintDate>{date}T08:00:00</ConstraintDate><Deadline>{junk}</Deadline>\
<Type>{task_type}</Type><EffortDriven>{}</EffortDriven><Stop>{junk}</Stop><Resume>{date}T08:00:00</Resume>",
            u8::from(effort)
        );
        let i = mspdi(&plan(&[(1, "Lay the patio", 16, &inside), (2, "Fit the gate", 8, &format!("<Deadline>{date}T17:00:00</Deadline>"))]));
        let start = i.project.start;
        for t in &i.project.tasks {
            for d in t.constraint.date().into_iter().chain(t.deadline) {
                proptest::prop_assert!(d.days_since(start).abs() <= MAX_DAYS);
            }
            proptest::prop_assert!(t.task_type != TaskType::FixedWork || t.effort_driven);
        }
        let _ = model::schedule(&i.project);
    }
}

/// Regression: a constraint's time of day was left without a word, so a task held to Start No
/// Earlier Than 13:00 finished a day sooner than in the file with nothing in the report to say
/// why. The day is still what ProjectCraft holds; now the report says the time was left.
#[test]
fn a_constraint_time_of_day_that_moves_the_task_is_reported() {
    let i = mspdi(&plan(&[
        (1, "Mulch the rows", 8, &constraint(4, "2026-03-04T13:00:00")),
        (2, "Water the rows", 8, &constraint(7, "2026-03-06T12:00:00")),
        (3, "Rake the paths", 8, &constraint(2, "2026-03-04T08:00:00")),
        (4, "Sweep the yard", 8, &constraint(3, "2026-03-05T17:00:00")),
        (5, "Fill the trough", 8, &constraint(6, "2026-03-05T00:00:00")),
    ]));
    let l = find(&i, "Constraint time of day");
    assert_eq!((l.outcome, l.place.as_str()), (Outcome::Converted, "ID 1 Mulch the rows, ID 2 Water the rows"));
    assert_eq!(i.project.tasks[0].constraint, Constraint::StartNoEarlierThan(d(2026, 3, 4)), "the day is kept");
    let s = model::schedule(&i.project).unwrap();
    assert_eq!((s.tasks[&TaskId(1)].start, s.tasks[&TaskId(1)].finish), (d(2026, 3, 4), d(2026, 3, 4)));
}
