# Roadmap

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change list:** minor (M1.3 hard constraints; M1.5 Open/Save dialogs; planning docs made private; roadmap format as MailCraft) · **Target:** Microsoft Project (desktop)

ProjectCraft aims for parity with Microsoft Project's scheduling, as its documentation describes it. Each roadmap milestone has a pass/fail checklist; it is reached when every item passes.

## Headline numbers (2026-10-16)

| Number | Value | Kind |
|---|---|---|
| M1 tickets done | **8 / 16** | measured (GitHub issues) |
| Commands / crates | 10 commands · 4 crates, 2 apps, `xtask` | measured (`projectcraft-cli commands`, source) |
| Tests | 67 `#[test]`s; property tests on the file reader, the duration and predecessor parsers | measured (source) |
| Builds verified | Windows, macOS and Linux (CI on each), the browser build (wasm) | measured |
| Releases | none yet | measured |

## M0 — Foundations ✅ (2026-10-15)
- [x] Workspace, layering, `cargo xtask ci`, CI on three systems
- [x] Tasks, milestones, FS/SS/FF/SF links with lag, project calendar
- [x] Critical-path scheduler with slack, refusing loops
- [x] Command registry, undo/redo, failed commands change nothing, hostile-params test
- [x] Gantt Chart view (task sheet + bars), `.projectcraft` JSON, CLI

## M1 — A plan you can run a real project on
- [x] Command param schemas and effects; `format_version`
- [ ] Summary tasks (outline levels, indent/outdent, rolled-up dates, summary links)
- [ ] ID as row number, Unique ID stable; insert, move and delete rows; selection and keyboard
- [ ] Calendar exceptions and the timescale (weeks/months, zoom)
- [ ] Constraints (Start No Earlier Than, Must Finish On…) and deadlines; negative slack warning
- [ ] Atomic save, Open/Save dialogs, autosave (on by default), recovery copy, recent files
- [ ] About 15 worked examples citing Project's documentation; 200 tasks under 10 ms
- [x] Theme tokens (light/dark), `tl!` translation, look checked in screenshots (`ui_shot`)

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
| 2026-10-15 | M1 scope agreed: parity decisions, file format, groundwork for agents |
| 2026-10-16 | M1.0: every command has a params JSON Schema and an effect; unknown params refused; `commands` lists both (#4) |
| 2026-10-16 | M1.0: `format_version` in the file (refuse newer, migrate M0), validation on load, atomic save with `.bak` (#6) |
| 2026-10-16 | M1.4: duration entry takes `d`, `w` (×5) and `mo` (×20), clamped to `MAX_DAYS`; `h`, `m` and elapsed units refused with a clear message (#16) |
| 2026-10-16 | M1.2: the ID column is the row number; Predecessors shows and reads row IDs, links stay by Unique ID; commands documented as taking Unique IDs (#9) |
| 2026-10-16 | M1.3: Start/Finish No Earlier Than constraints (`task.setConstraint`); typing a Start or Finish sets SNET or FNET; worked examples (#11) |
| 2026-10-16 | M1.6: theme tokens (light/dark), `tl!` for UI text, `ui_shot` headless screenshots (#5) |
| 2026-10-16 | M1.5: File menu with Open, Open Recent, Save and Save As through native dialogs (`rfd`, desktop app only); recent files (10, missing ones pruned) kept in the settings folder; Save / Don't Save / Cancel prompt on close or open with unsaved changes; Ctrl+O, Ctrl+S, Ctrl+Shift+S (#17) |
| 2026-10-16 | M1.3: hard constraints (SNLT, FNLT, MSO, MFO) win over links; negative slack propagates to predecessors; per-task constraint-conflict warning from every schedule run (sheet indicator, CLI `!`); start drivers (project start, link, constraint) recorded; worked examples (#12) |
| 2026-10-16 | Specs, plans, decisions and glossary moved out of the repository (kept privately); no AI attribution in commits or pull requests |

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
| 2026-10-16 | minor | M1 progress; status line, headline numbers and revision history as MailCraft; links to private planning docs removed |
