# Architecture

> **Status:** current · **Written:** 2026-10-17 · **Change:** minor (resources work outside the project calendar)

ProjectCraft is a Cargo workspace of layered crates, as in [MailCraft](https://github.com/UCS-Care-Group/mailcraft/blob/main/docs/architecture.md).

| Layer | Crate | Depends on | Job |
|---|---|---|---|
| L0 | `projectcraft-time` | — | `Date` (days since 1970-01-01), weekday, ISO text |
| L1 | `projectcraft-model` | time | `Project`, `Task`, `Link`, `Calendar`; `schedule()` (critical path) |
| L2 | *(planned)* `mspdi`, `xer` | model | Microsoft Project XML and Primavera XER import/export |
| L4 | `projectcraft-engine` | model, time | `Session` (project, schedule, undo), `COMMANDS`, sample, JSON file |
| L5 | *(planned)* `mcp` | engine | MCP server |
| L6 | `projectcraft-ui-egui` | engine | Gantt Chart view |
| apps | `projectcraft`, `projectcraft-cli` | any | desktop shell (eframe), command line |

`cargo xtask layers` enforces that a crate depends only on lower layers and that nothing below L6 uses a UI toolkit. `cargo xtask wasm` checks that every crate up to L5 and `ui-egui` builds for `wasm32-unknown-unknown`.

## Data flow
1. The UI or CLI calls `Session::run(id, params)`. Each `CommandSpec` declares a params JSON Schema and an effect (`read`/`edit`/`destructive`/`file`); params are checked against the schema first, so unknown or mistyped fields are refused.
2. The command edits a clone-protected `Project`; on error (or panic) the project is restored.
3. On a change, the old project goes on the undo stack and `model::schedule` recomputes `Session::schedule`.
4. The UI draws the task sheet and bars from `Session::project` and `Session::schedule`.

## Scheduling
Times are offsets in working minutes from the first working minute of the project, counted on the calendar's working times; durations, lags and slack are working minutes, and a day of duration is the project's hours per day. The forward pass gives early start/finish per link kind (FS, SS, FF, SF, with lag); the backward pass from the latest early finish gives late dates; total slack is late start minus early start; slack ≤ 0 is critical. Offsets become moments (a date and a minute of the day) through one walk of the calendar.

A task with a work resource on a calendar of its own (the project calendar with the resource's own weekdays and exceptions on top, worked as they are, inside or outside the project calendar's working time) is worked assignment by assignment: each from the task's start for its own duration on its resource's calendar. The task spans its earliest assignment start to its latest finish; its late start is worked back on the same calendars. Offsets count the plan's working time: the project calendar, or, when a resource on a task works outside it (a Saturday, a project holiday, hours before or after the project's day), the project calendar with that time added, so every minute anyone works has an offset. Then every other task is walked on the project calendar too, lags count project-calendar working time, and slack and summary durations are converted back to it; in most plans nothing of this is needed. Calendars are counted, not copied: the project calendar is indexed once per schedule (its weekly pattern and its exceptions' corrections added up in date order), and each resource adds only its own days, so working minutes between two dates cost a few binary searches. Other tasks, those without resources included, are on the project calendar.

## Resource usage
`model::usage` works out, from a project and its schedule, each work resource's work day by day and the days it is overallocated (work over Max Units × the day's working time on its calendar). Each assignment's work is spread over the working minutes it spans on its resource's calendar (the span the scheduler recorded, or the task's start for the assignment's duration), rounded so the days add up to its work; the walk is bounded in total days. `Session::usage` works it out once per change, on first use; the Resource Sheet, the task sheet's indicator and the Resource Usage view read it. Materials and cost resources are not in it.

## Security
What the program protects, from whom, and which code does it: [threat-model.md](threat-model.md).

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
| 2026-10-16 | minor | Command schemas and effects; links to private planning docs removed |
| 2026-10-16 | minor | Link to the threat model |
| 2026-10-16 | minor | The scheduler counts working minutes |
| 2026-10-16 | minor | Resource calendars |
| 2026-10-17 | minor | Resource usage and overallocation |
| 2026-10-17 | minor | Resources work outside the project calendar's working time (#95) |
