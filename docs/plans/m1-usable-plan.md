# M1 — implementation plan

> **Status:** agreed (follows the grilled [M1 spec](../specs/m1-usable-plan.md)) · **Written:** 2026-10-15 · **Change:** major (grilled)

## Revision history
| Date | Change |
|---|---|
| 2026-10-15 | First draft. |
| 2026-10-15 | Rewritten after grilling: command metadata first, Unique ID / row ID, atomic save and recovery, worked examples. |

Each step is one pull request: tests first, `cargo xtask ci` green, task id in the commit message.

| Step | Work | Crates | Tests |
|---|---|---|---|
| M1.0 | `CommandSpec` gains a param JSON Schema and an `effect` (`read`/`edit`/`destructive`/`file`). Params are checked against the schema, so unknown fields are refused. `commands` lists both. `format_version` in the file, with refuse-newer and a migration hook. | engine, model, cli | Every command has a schema; unknown param refused; hostile params still green; old file loads, newer major refused |
| M1.1 | `Task.outline_level`; `task.indent`/`task.outdent` (lists of Unique IDs). Summary roll-up in `schedule`, plus summary links and the parent hold on subtasks. Circular outline and summary↔own-subtask links refused. The scheduler records the driver of each start. | model, engine | Worked examples 1, 2, 9, 10, 13 |
| M1.2 | The ID column is the row number. Predecessors parse and show row IDs, links stored by Unique ID. `task.insert {before}`, `task.move {ids, to}` (subtree with summary), `task.delete {ids}` (subtree). Multi-select and keyboard in the sheet (arrows, Enter, F2, Insert, Delete, Alt+Shift+←/→ indent). | engine, ui-egui | Worked example 12; undo of a move is one step |
| M1.3 | Constraints and deadline on `Task`. Forward and backward pass with constraints winning, negative slack, deadline capping late finish, warning flag. `task.setConstraint`, `task.setDeadline`. Typing Start or Finish sets SNET or FNET. | model, engine, ui-egui | Worked examples 3, 4, 5, 11, 14, 15 (worked examples written before the code) |
| M1.4 | Calendar exceptions (working and non-working). `calendar.setWorking`, `calendar.addException`, `calendar.removeException`. Change Working Time dialog. Timescale with two tiers, presets and zoom; non-working shading. Duration entry `d`/`w`/`mo`, others refused. | model, engine, ui-egui | Worked examples 6, 7; no-working-days refused; unit parsing |
| M1.5 | Atomic save (temp, fsync, rename, `.bak`). Open/Save/Save As through `rfd`, recent files, unsaved prompt. Autosave on by default every 2 min while dirty, never clearing undo. Recovery copy and restore offer. | apps/projectcraft, engine | Round trip of every field; interrupted save leaves an intact file; save keeps undo; recovery offered when newer |
| M1.6 | Theme tokens light/dark, `tl!`, `ui_shot` headless screenshots | ui-egui | Screenshots reviewed, light and dark |
| M1.7 | Remaining worked examples (8); 200-task gate under 10 ms; 2,000-task timing reported | model | Expected dates; timing |

## Risks
- **Constraints with negative slack (M1.3):** write worked examples 3–5 by hand from the cited formulas before the code. Where Project's docs are silent (see the spec's parity notes), check in Project if available and note the result in the test.
- **The summary hold on subtasks (M1.1)** changes the forward pass. Keep one pass: a subtask's early start is at least its summary's link- or constraint-driven start.
- **Row ID vs Unique ID (M1.2)** touches the Predecessors parser and every command param. M1.0's schema makes the switch checkable.
