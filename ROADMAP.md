# Roadmap

> **Status:** current · **Written:** 2026-10-15 · **Change:** major (first version)

Each roadmap milestone has a pass/fail checklist; it is reached when every item passes.

## M0 — Foundations ✅ (2026-10-15)
- [x] Workspace, layering, `cargo xtask ci`, CI on three systems
- [x] Tasks, milestones, FS/SS/FF/SF links with lag, project calendar
- [x] Critical-path scheduler with slack, refusing loops
- [x] Command registry, undo/redo, failed commands change nothing, hostile-params test
- [x] Gantt Chart view (task sheet + bars), `.plancraft` JSON, CLI

## M1 — A plan you can run a real project on (draft: [spec](docs/specs/m1-usable-plan.md), [plan](docs/plans/m1-usable-plan.md))
- [ ] Summary tasks (outline levels, indent/outdent, rolled-up dates)
- [ ] Insert and move rows; task sheet selection and keyboard
- [ ] Calendar editing (working days, holidays) and the timescale (weeks/months)
- [ ] Constraints (Start No Earlier Than, Must Finish On…) and deadlines
- [ ] Open/Save dialogs, autosave, recent files
- [ ] Theme tokens (light/dark), `tl!` translation, look checked in screenshots

## Later
- M2 — Resources and assignments (work, units, effort-driven), resource sheet, overallocation
- M3 — Baselines, % complete, actuals, tracking Gantt
- M4 — Microsoft Project XML (MSPDI) and CSV in and out; XER
- M5 — Control channel and MCP server; web build (`apps/plancraft-web`)
- M6 — Packaging and releases (as MailCraft: msi, AppImage, deb, rpm, flatpak, dmg)

## Progress log
| Date | What landed |
|---|---|
| 2026-10-15 | M0: repository set up; scheduler, engine, Gantt view, CLI |
