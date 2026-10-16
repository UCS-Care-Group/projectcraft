# Roadmap

> **Status:** current · **Written:** 2026-10-16 · **Change:** minor (M1.0 command schemas)

Each roadmap milestone has a pass/fail checklist; it is reached when every item passes.

## M0 — Foundations ✅ (2026-10-15)
- [x] Workspace, layering, `cargo xtask ci`, CI on three systems
- [x] Tasks, milestones, FS/SS/FF/SF links with lag, project calendar
- [x] Critical-path scheduler with slack, refusing loops
- [x] Command registry, undo/redo, failed commands change nothing, hostile-params test
- [x] Gantt Chart view (task sheet + bars), `.projectcraft` JSON, CLI

## M1 — A plan you can run a real project on (agreed: [spec](docs/specs/m1-usable-plan.md), [plan](docs/plans/m1-usable-plan.md))
- [ ] Command param schemas and effects; `format_version` ([ADR 0003](docs/adr/0003-native-json-format.md), [ADR 0004](docs/adr/0004-ai-access-model.md))
- [ ] Summary tasks (outline levels, indent/outdent, rolled-up dates, summary links)
- [ ] ID as row number, Unique ID stable; insert, move and delete rows; selection and keyboard
- [ ] Calendar exceptions and the timescale (weeks/months, zoom)
- [ ] Constraints (Start No Earlier Than, Must Finish On…) and deadlines; negative slack warning
- [ ] Atomic save, Open/Save dialogs, autosave (on by default), recovery copy, recent files
- [ ] About 15 worked examples citing Project's documentation; 200 tasks under 10 ms
- [ ] Theme tokens (light/dark), `tl!` translation, look checked in screenshots

## Later
- M2 — Resources and assignments (work, units, effort-driven), resource sheet, overallocation
- M3 — Baselines, % complete, actuals, tracking Gantt
- M4 — Microsoft Project XML (MSPDI) and CSV in and out; XER
- M5 — Control channel and MCP server; web build (`apps/projectcraft-web`)
- M6 — Packaging and releases (as MailCraft: msi, AppImage, deb, rpm, flatpak, dmg)

## Progress log
| Date | What landed |
|---|---|
| 2026-10-15 | M0: repository set up; scheduler, engine, Gantt view, CLI |
| 2026-10-15 | M1 spec and plan grilled and agreed; ADRs 0003 (native JSON) and 0004 (agent access) |
| 2026-10-16 | M1.0: every command has a params JSON Schema and an effect; unknown params refused; `commands` lists both (#4). `format_version` still to do |
