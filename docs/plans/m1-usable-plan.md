# M1 — implementation plan

> **Status:** draft (follows the M1 spec; to be grilled) · **Written:** 2026-10-15

Each step is one pull request, `cargo xtask ci` green, with tests first.

| Step | Work | Crates | Tests |
|---|---|---|---|
| M1.1 | `Task.outline_level`; `task.indent`/`task.outdent`; roll-up of summary dates in `schedule` | model, engine | summary roll-up worked examples; indent past the first row refused |
| M1.2 | `task.insert {before}`, `task.move {id, to}`; selection state in the UI; keyboard in the sheet | engine, ui-egui | IDs stable across moves; undo of a move |
| M1.3 | Constraints on `Task` and in the forward/backward pass; deadline; negative-slack warning | model, engine | one test per constraint type |
| M1.4 | Calendar dialog; `calendar.setWorking`, `calendar.addHoliday`, `calendar.removeHoliday`; timescale tiers; shading | engine, ui-egui | holiday moves dates; no working days refused |
| M1.5 | Open/Save dialogs (`rfd`), autosave, recent files, unsaved prompt | apps/projectcraft | save/load round trip of every field |
| M1.6 | Theme tokens, `tl!`, headless screenshot example (`ui_shot`, as MailCraft) | ui-egui | screenshots light and dark |
| M1.7 | Reference plans and a 200-task benchmark | model | expected dates; timing assertion in release |

Risks: matching Project's handling of constraints with negative slack (M1.3) — write the worked examples before the code.
