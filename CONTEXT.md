# ProjectCraft: domain language

A glossary of the words the project uses with one meaning. Not a spec: how things are built lives in `docs/`.

## People
- **Owner**: the person whose project ProjectCraft holds. The docs and the interface say "owner", never "user", so it is never confused with a resource.
- **Resource**: a person, piece of equipment or material assigned to tasks. Not an owner. (Not modelled yet.)

## Plans
- **Project**: one plan: a name, a start date, a calendar and its tasks. One `.projectcraft` file.
- **Task**: an auto-scheduled piece of work with a **duration** in working days. A task of duration 0 is a **milestone**.
- **ID**: the row number shown in the ID column. Renumbers when rows are inserted, moved or deleted, as in Microsoft Project. The Predecessors column shows and accepts IDs.
- **Unique ID**: a task's permanent number. Never changes or gets reused, so links survive deletes and moves. Links are stored by Unique ID.
- **Outline level**: a task's depth in the outline; the first row is level 1, and no row is more than one level deeper than the row above it.
- **Summary task**: a task whose next row is one level deeper. Its **subtasks** are the rows below it at deeper levels, up to the next row at its own level or shallower. Its start, finish and duration are rolled up from its subtasks, never set. Not a stored kind: indenting or outdenting a row makes or unmakes summaries.
- **Link**: a predecessor of a task, with a **kind** (FS finish-to-start, SS, FF, SF) and a **lag** in working days (negative lag is **lead**). Written as in the Predecessors column: `3, 5SS+2, 7FF-1`.
- **Calendar**: which weekdays are worked, and its exceptions. Only the project calendar exists so far.
- **Exception**: a whole day the calendar treats differently from its weekday: non-working (a holiday) or working (say, an overtime Saturday). Microsoft Project's term; we don't say "holiday" for the general case.
- **Constraint**: a date rule on a task's start or finish: As Soon As Possible (none, the default), Start/Finish No Earlier Than, Start/Finish No Later Than, Must Start/Finish On. Constraints win over links; when a link cannot be honoured, the task gets negative slack and a warning.
- **Deadline**: a target finish date shown as a marker. Not a constraint: it never moves the task, but it caps the late finish, so a missed deadline shows as negative slack.
- **Working day**: a day the calendar works. Durations, lags and slack count working days only.

## Scheduling
- **Schedule**: the dates the scheduler computes from the project. Never typed in; the UI shows them.
- **Early start / finish**: the soonest a task can start or finish (forward pass). Shown as Start and Finish.
- **Late start / finish**: the latest it can without moving the project's finish (backward pass).
- **Finish**: the last working day of a task (its start, for a milestone), as Microsoft Project shows it.
- **Total slack**: late start minus early start, in working days.
- **Critical**: a task with no total slack. The **critical path** is the chain of critical tasks.

## Files
- **Autosave**: saving the owner's file every few minutes while it has unsaved changes. On by default (Microsoft Project ships it off).
- **Recovery copy**: a separate file written beside the owner's file while there are unsaved changes, offered back after a crash, deleted on a clean save or close. Never replaces the owner's file.

## Automation
- **Agent**: an AI or script driving ProjectCraft through commands (CLI, control channel, MCP). It touches only the open project; every change it makes is one labelled undo step.
- **Destructive change**: deleting tasks, or one command changing more than 10 tasks. An agent needs the owner's confirmation for it.

## Progress
- **Milestone (roadmap)**: a named stage with a pass/fail checklist, reached when every item passes. Not to be confused with a milestone task; say **roadmap milestone** where both could be meant.
