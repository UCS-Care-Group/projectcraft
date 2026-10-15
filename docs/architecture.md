# Architecture

> **Status:** current · **Written:** 2026-10-15 · **Change:** major (first version)

PlanCraft is a Cargo workspace of layered crates, as in [MailCraft](https://github.com/UCS-Care-Group/mailcraft/blob/main/docs/architecture.md) (see [ADR 0001](adr/0001-follow-the-craft-architecture.md)).

| Layer | Crate | Depends on | Job |
|---|---|---|---|
| L0 | `plancraft-time` | — | `Date` (days since 1970-01-01), weekday, ISO text |
| L1 | `plancraft-model` | time | `Project`, `Task`, `Link`, `Calendar`; `schedule()` (critical path) |
| L2 | *(planned)* `mspdi`, `xer` | model | Microsoft Project XML and Primavera XER import/export |
| L4 | `plancraft-engine` | model, time | `Session` (project, schedule, undo), `COMMANDS`, sample, JSON file |
| L5 | *(planned)* `mcp` | engine | MCP server |
| L6 | `plancraft-ui-egui` | engine | Gantt Chart view |
| apps | `plancraft`, `plancraft-cli` | any | desktop shell (eframe), command line |

`cargo xtask layers` enforces that a crate depends only on lower layers and that nothing below L6 uses a UI toolkit. `cargo xtask wasm` checks that every crate up to L5 and `ui-egui` builds for `wasm32-unknown-unknown`.

## Data flow
1. The UI or CLI calls `Session::run(id, params)`.
2. The command edits a clone-protected `Project`; on error (or panic) the project is restored.
3. On a change, the old project goes on the undo stack and `model::schedule` recomputes `Session::schedule`.
4. The UI draws the task sheet and bars from `Session::project` and `Session::schedule`.

## Scheduling
Times are offsets in working days from the project start. The forward pass gives early start/finish per link kind (FS, SS, FF, SF, with lag); the backward pass from the latest early finish gives late dates; total slack is late start minus early start; slack ≤ 0 is critical. Offsets become dates through one walk of the calendar.

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
