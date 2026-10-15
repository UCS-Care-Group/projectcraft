# PlanCraft: domain language

A glossary of the words the project uses with one meaning. Not a spec: how things are built lives in `docs/`.

## People
- **Owner**: the person whose project PlanCraft holds. The docs and the interface say "owner", never "user", so it is never confused with a resource.
- **Resource**: a person, piece of equipment or material assigned to tasks. Not an owner. (Not modelled yet.)

## Plans
- **Project**: one plan: a name, a start date, a calendar and its tasks. One `.plancraft` file.
- **Task**: a piece of work with a **duration** in working days. A task of duration 0 is a **milestone**.
- **Task ID**: the number in the ID column. Stable for the life of the project and never reused, so links survive deletes and moves.
- **Link**: a predecessor of a task, with a **kind** (FS finish-to-start, SS, FF, SF) and a **lag** in working days (negative lag is **lead**). Written as in the Predecessors column: `3, 5SS+2, 7FF-1`.
- **Calendar**: which weekdays are worked, and holidays. Only the project calendar exists so far.
- **Working day**: a day the calendar works. Durations, lags and slack count working days only.

## Scheduling
- **Schedule**: the dates the scheduler computes from the project. Never typed in; the UI shows them.
- **Early start / finish**: the soonest a task can start or finish (forward pass). Shown as Start and Finish.
- **Late start / finish**: the latest it can without moving the project's finish (backward pass).
- **Finish**: the last working day of a task (its start, for a milestone), as Microsoft Project shows it.
- **Total slack**: late start minus early start, in working days.
- **Critical**: a task with no total slack. The **critical path** is the chain of critical tasks.

## Progress
- **Milestone (roadmap)**: a named stage with a pass/fail checklist, reached when every item passes. Not to be confused with a milestone task; say **roadmap milestone** where both could be meant.
