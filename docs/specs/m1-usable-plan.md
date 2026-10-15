# M1 — A plan you can run a real project on

> **Status:** agreed (grilled 2026-10-15) · **Written:** 2026-10-15 · **Change:** major (grilled: parity decisions, files, AI-access groundwork)

## Revision history
| Date | Change |
|---|---|
| 2026-10-15 | First draft. |
| 2026-10-15 | Grilled with the owner: open questions answered, parity notes with sources, survey of other planners, AI-access groundwork ([ADR 0004](../adr/0004-ai-access-model.md)). |

## Problem
After M0 an owner can list tasks and links and see the critical path. They can't structure a plan (no summary tasks), reorder it, change the calendar, pin a date, or open and save files without the command line. Nobody would run a real project on it.

## Goal
An owner can build and maintain a 50–200 task plan for a real project entirely in the desktop app. **The aim is parity**: the dates and behaviour match what Microsoft Project does with the same plan, as its documentation describes. Words follow [`CONTEXT.md`](../../CONTEXT.md).

## Decisions

### IDs
- The **ID** column is the row number. It renumbers on insert, move and delete, as in Project. The Predecessors column shows and accepts IDs (`3, 5SS+2, 7FF-1`; `,` or `;` separate).
- Each task keeps a permanent **Unique ID** (today's `TaskId`): never changed, never reused. Links are stored by Unique ID, so they survive reordering.
- Commands (CLI, control channel, later MCP) take Unique IDs. The task sheet translates row IDs.

### Summary tasks
- Each task has an **outline level**. The first row is level 1, and no row is more than one level deeper than the row above it. A **summary task** is any row followed by a deeper row. That's worked out from row order, never stored.
- `task.indent` / `task.outdent` take a list of Unique IDs and are one undo step.
- A summary's start and finish are the earliest start and latest finish of its subtasks. Its duration is the working days between them. While a task is a summary, its own duration isn't used and can't be edited. When it stops being a summary (outdent, or its last subtask deleted or moved out), it keeps its last rolled-up duration.
- Deleting or moving a summary deletes or moves its whole subtree ("When you move or delete a summary task, Project moves or deletes all of its subtasks").
- Summaries take links. A summary's predecessor holds all its subtasks: no subtask starts before the date that link allows.
- A link between a summary and any of its own subtasks is refused. So is an indent or outdent that would create one, including indenting a task under its own linked predecessor. Message: "This outline change would create a circular relationship."

### Constraints and deadlines
- Types: As Soon As Possible (default), Start No Earlier Than, Finish No Earlier Than, Start No Later Than, Finish No Later Than, Must Start On, Must Finish On. A constraint date on a non-working day moves to the next working day for "start" types and the previous one for "finish" types.
- **Constraints win over links**, Project's default "Tasks will always honor their constraint dates". If a link can't be honoured, the task keeps its constraint date. Total slack goes negative on it and on its predecessors, and the task shows a warning. The warning comes from every schedule run, not only from the edit that caused it, so a predecessor slipping into a constraint is caught too. There's no modal Planning Wizard. The change applies and is one undo step.
- All types are allowed on summary tasks (Project allows them, with a warning when linked).
- Typing a Start in the sheet issues `task.setConstraint` Start No Earlier Than. Typing a Finish issues Finish No Earlier Than. The date shown still comes from the scheduler.
- Setting As Soon As Possible clears the constraint, and the task goes back to link-driven dates.
- A **deadline** never moves a task. It caps the late finish, so a finish after the deadline gives negative slack. It's drawn as a marker on the Gantt, with an indicator when missed.
- The scheduler records what drove each task's start: a link (which one), a constraint, the project start, or a parent summary. This is used by tests now and later by a "why this date?" view and the agent `explain` tool. No UI panel in M1.

### Calendar and timescale
- The project calendar has working weekdays plus **exceptions**: whole days made non-working (a holiday) or working (an overtime Saturday). Commands: `calendar.setWorking`, `calendar.addException`, `calendar.removeException`. A calendar with no working days is refused.
- A Change Working Time dialog edits them. Work weeks (date-ranged weekday patterns) come later.
- Timescale: two tiers, weeks over days by default, presets days / weeks / months, zoom in and out. Non-working days are shaded.

### Durations
- Whole working days. Entry accepts `d`, `w` (× days per week, default 5) and `mo` (× days per month, default 20). Typing `h`, `m` or an elapsed unit is refused with a clear error until part-day durations exist.

### Files
- Native format stays JSON with a `format_version` field. A newer major version is refused cleanly, and older ones are migrated on load ([ADR 0003](../adr/0003-native-json-format.md)). MSPDI is for exchange only, in M4.
- **Atomic save**: write a temp file in the same folder, fsync, rename. One rolling `.bak` of the previous save. Validate on load, never panic.
- Open / Save / Save As dialogs through `rfd` (new dependency, desktop app only). Recent files and an unsaved-changes prompt.
- **Autosave** is on by default (Project ships it off). It saves the owner's file every 2 minutes while there are unsaved changes. **Saving never clears undo**, manual or auto. That's the flaw that makes Project users turn autosave off.
- **Recovery copy**: always on, written beside the owner's file while there are unsaved changes, deleted on a clean save or close. On open, a recovery copy newer than the file is offered back. An untitled project's recovery copy lives in the app data folder until its first Save As. `--sample` keeps nothing.

### Command metadata (groundwork for agents)
Each `CommandSpec` gains a JSON Schema for its params and an **effect**: `read`, `edit`, `destructive` or `file`. Unknown params are refused, not ignored, so `durration` is an error. The CLI, `projectcraft-cli commands`, `hostile_params_never_panic` and (in M5) MCP all read the same metadata. The full model is in [ADR 0004](../adr/0004-ai-access-model.md).

### Look
Theme tokens with light and dark. Text goes through `tl!`. A headless screenshot example (`ui_shot`, as MailCraft). Light and dark screenshots are reviewed.

## Out of scope (known parity gaps)
- Manually scheduled tasks (Project's default for new tasks since 2010). All tasks are auto-scheduled.
- Part-day and elapsed durations.
- Work weeks, multiple calendars, task calendars.
- Project summary task (row 0).
- Three-tier timescale and custom formats.
- The "honor constraint dates" switch and a constraint-conflict resolution dialog.
- The "why this date?" panel (scheduler side only).
- Resources, baselines, tracking, MSPDI (later milestones). MCP itself (M5).

## Acceptance (pass/fail)
- [ ] Named worked examples (about 15) schedule to their expected dates. Each cites the Microsoft documentation rule it checks. List under *Worked examples*.
- [ ] Unique IDs survive insert, move, delete and save/load, and a deleted Unique ID is never reused.
- [ ] A save interrupted midway leaves the previous file or the new one intact, never a broken one.
- [ ] A 200-task plan reschedules in under 10 ms in release (gate). A 2,000-task timing is reported, not gated.
- [ ] Every new action is a command with a param schema and an effect, listed by `projectcraft-cli commands` and covered by `hostile_params_never_panic`. Unknown params are refused.
- [ ] `cargo xtask ci` green. Light and dark screenshots of the Gantt Chart reviewed.

## Worked examples
1. A summary as successor moves **earlier** when its predecessor shortens (GanttProject #2002 got this wrong).
2. A subtask of a linked or SNET summary starts no earlier than the summary's driven start.
3. MSO / FNLT on a successor holds when the predecessor slips. Negative slack on it and its predecessors.
4. A deadline before late finish: slack is measured from the deadline, dates are unmoved.
5. SNET later than the predecessor's finish: the task starts on the SNET date.
6. SS, FF and SF links with lag move a successor both later and earlier.
7. A 2-day task starting Friday skips the weekend. A non-working exception inside the span; a lag spanning an exception; a working exception on a Saturday.
8. A→B then B→A is refused with a message. A loop in a loaded file is reported and scheduling ends.
9. A link between a summary and its own subtask is refused.
10. Indenting a task under its linked predecessor is refused (circular relationship).
11. Clearing a constraint to ASAP returns to link-driven dates.
12. Delete, move and insert keep every Unique ID; links survive a reorder.
13. A summary that loses its last subtask becomes a normal task with its last rolled-up duration.
14. A constraint date on a non-working day moves to a working day.
15. Typing a Start or Finish sets SNET or FNET.

## Parity notes (published behaviour; sources)
- ID renumbers and Unique ID is never reused: [ID fields](https://support.microsoft.com/en-us/project/id-fields), [Unique ID fields](https://support.microsoft.com/en-us/project/unique-id-fields), [Predecessors field](https://support.microsoft.com/en-us/project/predecessors-task-field).
- Constraint definitions: [Definition of project constraints](https://learn.microsoft.com/en-us/office/troubleshoot/project/definition-of-project-constraints). The honour-constraints default: [proj_opt_honor_constraints](https://learn.microsoft.com/dotnet/api/websvcstatusing.statusingdataset.projectsrow.proj_opt_honor_constraints?view=office-project-server). Conflicts: [Planning Wizard](https://support.microsoft.com/en-au/office/planning-wizard-7707d227-d581-4293-a072-1bb08f02aa9e).
- Deadlines and slack: [Deadline field](https://support.microsoft.com/en-us/project/deadline-task-field), [Show slack](https://support.microsoft.com/en-US/project/show-slack-in-your-project-in-project-desktop). LF = min(LF, deadline) is our reading; the docs describe the effect, not the field.
- Typing a date: Start sets SNET ([Constraint Type field](https://support.microsoft.com/en-US/project/constraint-type-task-field)). Finish sets FNET per the [Finish field](https://support.microsoft.com/en-us/project/finish-task-field) page, while the Constraint Type page says FNLT. We follow the more specific page.
- Summaries move and delete with their subtasks: [Subtasks and summary tasks](https://support.microsoft.com/office/create-and-work-with-subtasks-and-summary-tasks-in-project-desktop-b3ff64ce-b121-42cc-905b-cb9b8ce0255f). Circular outline changes are refused: [circular relationship error](https://learn.microsoft.com/en-us/office/troubleshoot/project/circular-relationship-error-when-indenting-task). Summary links are allowed but discouraged: [TechNet wiki](https://learn.microsoft.com/en-us/archive/technet-wiki/29569.ms-project-linking-andor-assigning-resources-to-summary-tasks). Why a successor doesn't follow its predecessor: [TechNet wiki](https://learn.microsoft.com/en-us/archive/technet-wiki/30143.ms-project-troubleshooting-why-a-successor-task-s-start-date-may-not-directly-follow-a-predecessor-task-s-finish-date).
- Calendars: [Set working days and times](https://support.microsoft.com/en-us/project/set-the-general-working-days-and-times-for-a-project). Timescale: [Change the timescale](https://support.microsoft.com/en-us/project/change-the-timescale-in-a-project-view-in-project-desktop). Duration units: [DurationFormat](https://learn.microsoft.com/en-us/office-project/xml-data-interchange/durationformat-element). Task mode: [Task Mode field](https://support.microsoft.com/en-gb/office/task-mode-task-field-3b185518-0e9a-4774-ba51-7b8d191bb84a).
- **Not documented, our choice** (confirm in Project where possible): which constraints summaries accept (we allow all); refusing summary↔own-subtask links; duration kept after outdent; weeks-over-days default tiers.

## Lessons from other planners
From docs, issue trackers and forums only (no source code read):
- **GanttProject:** summary successors not pulling earlier (#2002); no autosave, and recovery files in /tmp that vanish; a corrupted 588 MB file.
- **ProjectLibre:** constraints that revert or won't clear; files that stop opening.
- **Microsoft Project:** autosave clears undo.
- **Bryntum:** a constraint warning only on the conflicting edit, never when a predecessor slips into it.
- **Aspose:** 45 s recalculation on 2,000 tasks.

Each of these is covered above by a decision, a worked example or an acceptance item.
