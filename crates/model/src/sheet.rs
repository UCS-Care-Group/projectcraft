//! The task sheet's text: each cell as the Gantt Chart's sheet shows it, in English. The sheet
//! draws these and a CSV export writes them, so the two never differ. Worked out for the whole
//! project at once (rows, summaries, rolled-up work and cost, each task's assignments), so a
//! sheet of any size is read in one pass rather than a search per cell.

use std::collections::{BTreeMap, HashMap};

use crate::{Assignment, Column, Date, DurationUnit, Outline, Project, Schedule, Task, TaskId, TaskType, Units, duration_text};

/// The task sheet's cells for one project and its schedule.
pub struct SheetText<'a> {
    project: &'a Project,
    schedule: Option<&'a Schedule>,
    outline: Outline,
    /// Each task's 1-based row.
    rows: HashMap<TaskId, usize>,
    /// Each task's assignments, in the project's order.
    assignments: HashMap<TaskId, Vec<&'a Assignment>>,
    /// A summary's Work ([`Project::work_by_task`]).
    work: BTreeMap<TaskId, i64>,
    costs: BTreeMap<TaskId, i64>,
}

impl<'a> SheetText<'a> {
    /// The sheet of `project`; Start and Finish are empty without a `schedule`.
    pub fn new(project: &'a Project, schedule: Option<&'a Schedule>) -> Self {
        let rows = project.tasks.iter().enumerate().map(|(i, t)| (t.id, i.saturating_add(1))).collect();
        let mut assignments: HashMap<TaskId, Vec<&Assignment>> = HashMap::new();
        for a in &project.assignments {
            assignments.entry(a.task).or_default().push(a);
        }
        SheetText { project, schedule, outline: Outline::of(project), rows, assignments, work: project.work_by_task(), costs: project.cost_by_task() }
    }

    /// Rows in the sheet.
    pub fn len(&self) -> usize {
        self.project.tasks.len()
    }

    pub fn is_empty(&self) -> bool {
        self.project.tasks.is_empty()
    }

    /// A row's (0-based) outline level, kept to the outline rule.
    pub fn level(&self, row: usize) -> u32 {
        self.outline.level(row)
    }

    /// Whether a row (0-based) is a summary task.
    pub fn is_summary(&self, row: usize) -> bool {
        self.outline.is_summary(row)
    }

    /// The Task Name cell of a row (0-based).
    pub fn name(&self, row: usize) -> &str {
        self.project.tasks.get(row).map_or("", |t| t.name.as_str())
    }

    /// Whether a cell can be typed in: never the Cost, which is worked out, nor a summary's
    /// Duration, Work, Type, Effort Driven or Leveling Delay, which follow its subtasks.
    pub fn editable(&self, row: usize, column: Column) -> bool {
        match column {
            Column::Cost => false,
            Column::Duration | Column::Work | Column::Type | Column::EffortDriven | Column::LevelingDelay => !self.is_summary(row),
            _ => true,
        }
    }

    /// A cell of a row (0-based) as the sheet shows it; empty past the last row.
    pub fn cell(&self, row: usize, column: Column) -> String {
        let Some(t) = self.project.tasks.get(row) else { return String::new() };
        let p = self.project;
        let summary = self.is_summary(row);
        let date = |d: Option<Date>| d.map_or_else(String::new, |d| d.to_string());
        let dates = self.schedule.and_then(|s| s.tasks.get(&t.id));
        match column {
            // A summary's duration is rolled up, and shown in days.
            Column::Duration if summary => duration_text(t.duration, DurationUnit::Days, &p.options),
            Column::Duration => duration_text(t.duration, t.duration_unit, &p.options),
            Column::Work if summary => work_text(self.work.get(&t.id).copied().unwrap_or(0), p),
            Column::Work => work_text(self.own_work(t), p),
            Column::Start => date(dates.map(|d| d.start)),
            Column::Finish => date(dates.map(|d| d.finish)),
            Column::Predecessors => p.preds_text_by(&t.preds, |id| self.rows.get(&id).copied()),
            Column::ResourceNames => p.names_text(self.assignments.get(&t.id).into_iter().flatten().copied()),
            Column::Cost => p.currency.format(self.costs.get(&t.id).copied().unwrap_or(0)),
            Column::FixedCost => p.currency.format(t.fixed_cost),
            Column::Deadline => date(t.deadline),
            // A summary is scheduled as Fixed Duration and is never effort driven.
            Column::Type if summary => TaskType::FixedDuration.name().to_owned(),
            Column::Type => t.task_type.name().to_owned(),
            Column::EffortDriven => yes_no(!summary && (t.effort_driven || t.task_type == TaskType::FixedWork)).to_owned(),
            Column::Priority => t.priority.to_string(),
            Column::LevelingDelay if summary => String::new(),
            Column::LevelingDelay => duration_text(t.leveling_delay, DurationUnit::Days, &p.options),
        }
    }

    /// A task's own work ([`Project::task_work`]) from its assignments.
    fn own_work(&self, t: &Task) -> i64 {
        let list = self.assignments.get(&t.id).map_or(&[][..], Vec::as_slice);
        list.iter().filter(|a| matches!(a.units, Units::Work(_))).fold(0i64, |sum, a| sum.saturating_add(a.work))
    }
}

/// Work as the Work column shows it: hours (`16h`).
pub fn work_text(minutes: i64, project: &Project) -> String {
    duration_text(minutes, DurationUnit::Hours, &project.options)
}

/// The Effort Driven column's text.
pub fn yes_no(b: bool) -> &'static str {
    if b { "Yes" } else { "No" }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Link, LinkKind, ResourceType, schedule};

    /// A summary over two tasks, a milestone after them, resources and a fixed cost.
    fn plan() -> Project {
        let mut p = Project::new("Sheet", Date::from_ymd(2026, 3, 2).unwrap_or(Date::EPOCH));
        let sum = p.add_task("Build", 0).unwrap();
        let a = p.add_task("Dig", 960).unwrap();
        let b = p.add_task("Pour", 240).unwrap();
        let m = p.add_task("Done", 0).unwrap();
        for id in [a, b] {
            p.task_mut(id).unwrap().outline_level = 2;
        }
        p.task_mut(b).unwrap().duration_unit = DurationUnit::Hours;
        p.set_preds(b, vec![Link { pred: a, kind: LinkKind::SS, lag: 240, lag_unit: DurationUnit::Hours }]).unwrap();
        p.set_preds(m, vec![Link { pred: sum, kind: LinkKind::FS, lag: 0, lag_unit: DurationUnit::Days }]).unwrap();
        p.set_resource_names(a, "Ann[50%], Bob").unwrap();
        p.task_mut(a).unwrap().fixed_cost = 12_345;
        p.task_mut(a).unwrap().deadline = Date::from_ymd(2026, 3, 20);
        p.set_task_type(b, Some(TaskType::FixedWork), None).unwrap();
        let _ = p.add_resource(Some("Crane"), ResourceType::Cost, None);
        p
    }

    #[test]
    fn every_cell_is_what_the_project_s_own_text_functions_give() {
        let p = plan();
        let s = schedule(&p).unwrap();
        let sheet = SheetText::new(&p, Some(&s));
        assert_eq!(sheet.len(), 4);
        for (row, t) in p.tasks.iter().enumerate() {
            assert_eq!(sheet.name(row), t.name);
            assert_eq!(sheet.cell(row, Column::Predecessors), p.preds_text_rows(&t.preds));
            assert_eq!(sheet.cell(row, Column::ResourceNames), p.resource_names(t.id));
            assert_eq!(sheet.cell(row, Column::Type), p.task_type_of(t.id).name());
            assert_eq!(sheet.cell(row, Column::EffortDriven), yes_no(p.is_effort_driven(t.id)));
            if !sheet.is_summary(row) {
                assert_eq!(sheet.cell(row, Column::Work), work_text(p.task_work(t.id), &p));
            }
            let d = s.tasks.get(&t.id).unwrap();
            assert_eq!(sheet.cell(row, Column::Start), d.start.to_string());
            assert_eq!(sheet.cell(row, Column::Finish), d.finish.to_string());
        }
        assert_eq!(sheet.cell(1, Column::Duration), "2d");
        assert_eq!(sheet.cell(2, Column::Duration), "4h");
        assert_eq!(sheet.cell(2, Column::Predecessors), "2SS+4h");
        assert_eq!(sheet.cell(3, Column::Predecessors), "1");
        assert_eq!(sheet.cell(1, Column::ResourceNames), "Ann[50%], Bob");
        assert_eq!(sheet.cell(1, Column::FixedCost), "£123.45");
        assert_eq!(sheet.cell(1, Column::Deadline), "2026-03-20");
        assert_eq!(sheet.cell(2, Column::Type), "Fixed Work");
        assert_eq!(sheet.cell(2, Column::EffortDriven), "Yes");
        assert_eq!(sheet.cell(1, Column::Priority), "500");
        assert_eq!(sheet.cell(1, Column::LevelingDelay), "0d");
        // The summary's: rolled up, in days; Fixed Duration, not effort driven, no delay.
        assert!(sheet.is_summary(0) && sheet.level(1) == 2);
        assert_eq!(sheet.cell(0, Column::Type), "Fixed Duration");
        assert_eq!(sheet.cell(0, Column::EffortDriven), "No");
        assert_eq!(sheet.cell(0, Column::LevelingDelay), "");
        assert_eq!(sheet.cell(0, Column::Cost), p.currency.format(p.cost_by_task()[&p.tasks[0].id]));
        assert!(!sheet.editable(0, Column::Duration) && sheet.editable(1, Column::Duration) && !sheet.editable(1, Column::Cost));
        assert_eq!(sheet.cell(9, Column::Duration), "", "past the last row");
    }

    #[test]
    fn without_a_schedule_dates_are_empty() {
        let p = plan();
        let sheet = SheetText::new(&p, None);
        assert_eq!(sheet.cell(1, Column::Start), "");
        assert_eq!(sheet.cell(1, Column::Deadline), "2026-03-20");
    }
}
