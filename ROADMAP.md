# Roadmap

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change list:** minor (Assign Resources on the selection) · **Target:** Microsoft Project (desktop)

ProjectCraft aims for parity with Microsoft Project's scheduling, as its documentation describes it. Each roadmap milestone has a pass/fail checklist; it is reached when every item passes.

## Headline numbers (2026-10-16)

| Number | Value | Kind |
|---|---|---|
| M1 tickets done | **16 / 16**, plus follow-ups #26, #28, #37, #39 | measured (GitHub issues) |
| M2 tickets done | **7 / 11** (#51–#56, #58), plus #65 Project Options | measured (GitHub issues) |
| Commands / crates | 32 commands · 4 crates, 2 apps, `xtask` | measured (`projectcraft-cli commands`, source) |
| Tests | 286 `#[test]`s; property tests in 11 modules: the file reader, the duration, predecessor and Resource Names parsers, outline changes, working times, resources, task types, costs and the scheduler | measured (source) |
| Builds verified | Windows, macOS and Linux (CI on each), the browser build (wasm) | measured |
| Reschedule time | 200 tasks: ~0.1–0.3 ms (gate 10 ms); 2,000 tasks: ~2–5 ms; working time counted in minutes since M2 | measured (`cargo xtask perf`, release, CI runners) |
| Resource Names edit | 1,000 names on 5,000 tasks and 20,000 assignments: ~40–60 ms (gate 100 ms) | measured (`cargo xtask perf`, release, CI runners) |
| Releases | none yet | measured |

## M0 — Foundations ✅ (2026-10-15)
- [x] Workspace, layering, `cargo xtask ci`, CI on three systems
- [x] Tasks, milestones, FS/SS/FF/SF links with lag, project calendar
- [x] Critical-path scheduler with slack, refusing loops
- [x] Command registry, undo/redo, failed commands change nothing, hostile-params test
- [x] Gantt Chart view (task sheet + bars), `.projectcraft` JSON, CLI

## M1 — A plan you can run a real project on ✅ (2026-10-16)
- [x] Command param schemas and effects; `format_version`
- [x] Summary tasks (outline levels, indent/outdent, rolled-up dates; summary links and constraints hold their subtasks; circular outline changes refused)
- [x] ID as row number, Unique ID stable; insert, move and delete rows; selection and keyboard
- [x] Calendar exceptions and the timescale (weeks/months, zoom)
- [x] Constraints (Start No Earlier Than, Must Finish On…) and deadlines; negative slack warning
- [x] Atomic save, Open/Save dialogs, autosave (on by default), recovery copy, recent files
- [x] About 15 worked examples citing Project's documentation; 200 tasks under 10 ms
- [x] Theme tokens (light/dark), `tl!` translation, look checked in screenshots (`ui_shot`)

## M2 — Resources and assignments
- [x] Working time in hours and minutes: part-day durations, working hours in calendars, files from M1 open with the same dates
- [x] Resource Sheet: work, material and cost resources
- [x] Assignments: Resource Names column, Assign Resources dialog, material quantities
- [x] Task types (Fixed Units, Fixed Duration, Fixed Work) and effort-driven scheduling, as Microsoft Project documents them
- [ ] Resource calendars (leave and part-time working)
- [x] Costs: rates, cost per use, fixed costs, cost resources, rolled up to the penny
- [ ] Overallocation shown in red; Resource Usage view
- [ ] Stretch: manual resource levelling (delays only)
- [ ] 200 tasks with 30 resources reschedule under 10 ms

## Later
- M3 — Baselines, % complete, actuals, tracking Gantt
- M4 — Microsoft Project XML (MSPDI) and CSV in and out. Imports open as a new project, dates recomputed by ProjectCraft's scheduler, with an import report of what was converted or dropped and which tasks' dates differ from the file's. Gaps on import, until later work closes them:
  - Converted: elapsed durations and lags become working time; percent lags become working time from the predecessor's duration; manually scheduled tasks become auto-scheduled; contours other than flat become flat
  - Dropped: base calendars other than the project calendar, task calendars, resource calendars (until resource calendars land), rate tables B–E
  - Dropped until tracking (M3): baselines, actuals, % complete
  - Dropped for now: notes, hyperlinks, custom fields, priority, task splits, recurring tasks
  - Not yet: appending or merging into the open project; a CSV column-mapping wizard
- XER (Primavera) import — moved after M4
- M5 — Control channel and MCP server; web build (`apps/projectcraft-web`)
- M6 — Packaging and releases (as MailCraft: msi, AppImage, deb, rpm, flatpak, dmg)
- Gap: currency exchange. Changing a project's currency keeps each amount's number (£12.50 becomes $12.50; rounded half to even when the new currency has fewer decimal places); converting amounts at an exchange rate is not done yet

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
| 2026-10-16 | SEC: cargo-deny (`deny.toml`, `cargo xtask deny`, a CI job); every action pinned to a commit SHA; zizmor and OpenSSF Scorecard workflows; Dependabot with a week's cooldown; CodeQL config; `SECURITY.md`, threat model and release rules |
| 2026-10-16 | M1.1: summary tasks: `outline_level`, `task.indent`/`task.outdent` (one undo step, subtree moves), roll-up of start, finish and duration, summary bar, Alt+Shift+→/←; worked example 13; links and constraints on summaries wait for #8 (#7) |
| 2026-10-16 | M1.5: autosave on by default (every 2 minutes while dirty, File > Autosave to turn off; saving never clears undo); recovery copy `<name>.projectcraft.recovery` (untitled: in the settings folder) at most every 30 s while dirty, deleted on clean save or close, offered back with Restore / Discard when newer than the file; `--sample` keeps nothing (#18) |
| 2026-10-16 | M1.4: calendar exceptions (non-working and working days); `calendar.setWorking`, `calendar.addException`, `calendar.removeException`; a week with no working weekday refused; old `holidays` load as exceptions; Project ▸ Change Working Time dialog; worked example 7 (#14) |
| 2026-10-16 | M1.1: summaries take links (as predecessor and successor) and every constraint type; a summary's link or constraint holds all its subtasks, through nesting; `ParentSummary` start driver; a link between a summary and its own subtask, or an indent or outdent that would make one, is refused as a circular relationship; worked examples 1, 2, 9, 10 (#8) |
| 2026-10-16 | SEC: on Unix every file ProjectCraft writes (project file via its atomic-save temp, `.bak`, recovery copy, settings files) is created 0600, and new settings and recovery folders 0700; no-op on Windows and the web (#37) |
| 2026-10-16 | M1.3: deadlines: `task.setDeadline` (no date clears); a deadline never moves a task but caps its late finish, so finishing after it gives negative slack back through the predecessors; missed deadlines flagged (sheet indicator, CLI `>`); deadline marker on the Gantt; Deadline column; worked examples (#13) |
| 2026-10-16 | M1.4: Gantt timescale in two tiers (weeks over days, months over weeks, years over months) from View ▸ Timescale or the toolbar; Zoom In / Zoom Out (Ctrl+= / Ctrl+-) in steps, clamped; zoom is view state (not saved, not undone); weekends and non-working exceptions shaded behind the bars, working exceptions not; only visible columns drawn (#15) |
| 2026-10-16 | M1.2: `task.insert` (above a task, at its outline level), `task.move` and `task.delete` on several tasks at once, each one undo step; a summary moves or deletes with its subtasks; Unique IDs kept and never reused, links survive reorder. Sheet: click, Shift-click and Ctrl-click select rows; arrows, Enter, F2, typing to edit, Escape, Insert, Delete, Alt+Shift+→/← on the selection (#10) |
| 2026-10-16 | M1.7: worked examples for SS, FF and SF links with lag moving a successor later and earlier, lead time, a refused A→B→A loop, and a loop in a loaded file reported (a 50,000-task ring included); a deterministic plan generator (summaries three levels deep, all link types with lag and lead, every constraint type, deadlines, holidays and working Saturdays); `cargo xtask perf` (part of `cargo xtask ci`) gates the median 200-task reschedule under 10 ms in release (about 0.1 ms) and reports 2,000 tasks (about 0.9 ms) (#19) |
| 2026-10-16 | M1.5: built-in file browser for Open and Save As when no system dialog can be shown (Linux with no file chooser portal and no zenity; `PROJECTCRAFT_FILE_DIALOG=builtin` forces it): folders and `.projectcraft` files, Up and Home, a typed folder or file name, `.projectcraft` added on Save As, asks before replacing; listing capped at 5,000 entries (#28) |
| 2026-10-16 | M1.3: a milestone constrained to start on a date (SNET, SNLT held, MSO) shows that date as Start and Finish, early and late, and its summary and the project finish roll it up; one after a finish-to-start link still shows the predecessor's finish; worked examples (#26) |
| 2026-10-16 | M1.1: finish-type holds from a summary are exact: an FF or SF link into a summary, or Finish No Earlier Than, Finish No Later Than or Must Finish On on it, puts the summary's finish on the date even when its subtasks' own links and constraints make its span depend on the hold (a bounded search, the same dates every run); the search is bounded so a hostile full-size plan (100,000 tasks of nested Finish No Earlier Than summaries) schedules in about 175 ms in release, gated under 250 ms by `cargo xtask perf`; the perf plan now has groups with Finish No Earlier Than (200 tasks about 0.1 ms, 2,000 about 1 ms) (#39) |
| 2026-10-16 | M2.1: Resource Sheet (View ▸ Resource Sheet): work (default), material and cost resources with name, type, material label, initials, group, Max Units, standard and overtime rates, cost/use and Accrue At; row ID plus permanent Unique ID never reused; money in whole pence shown with £, Max Units in hundredths of a percent (no floats); `resource.add`, `resource.rename`, `resource.set`, `resource.move`, `resource.delete` (destructive); names unique and capped, no `[ ]` or commas; resources validated on load; the sample has five invented resources; the threat model notes that project files hold people's names (#54) |
| 2026-10-16 | M2.0: calendars hold working times per weekday (default Mon–Fri 8:00–12:00 and 13:00–17:00; a day with none is non-working) and per exception (a worked Saturday 9:00–13:00); `calendar.setWorkingTimes`, `calendar.addException` takes `times`; project options hours per day / per week / days per month (8 / 40 / 20) via `project.setOptions`; Change Working Time edits weekday times and exception hours; additive file fields, validated on load; scheduling still in whole working days (#51) |
| 2026-10-16 | M2.0: Project › Options… dialog shows hours per day, hours per week and days per month; Apply sends the changed ones as one `project.setOptions` (one undo step); a value out of range or not a number says why and changes nothing; Cancel changes nothing (#65) |
| 2026-10-16 | M2.0: the scheduler counts working time in minutes on the calendar's working times; durations, lags and slack are working minutes, starts and finishes moments (date and minute); file format 2 (version-1 files migrate on open, day counts × the file's minutes per day); every M1 worked example keeps its dates (#52) |
| 2026-10-16 | SEC: the project file, `.bak`, recovery copy and settings files are saved through `craft-atomic` (storytold/craft-libs, pinned by `rev`): staging beside the file, full flush on macOS, Windows rename retry, owner-only from the first byte; ProjectCraft's own temp/rename/fsync code removed; native only (#67) |
| 2026-10-16 | SEC: on macOS quitting (window close, Cmd+Q from a new app menu whose Quit asks first) goes through `-[NSApplication terminate:]` via `craft-quit` from storytold/craft-libs, after the Save / Don't Save / Cancel prompt, so eframe never closes the window under AppKit; the close check runs in eframe's `logic`, so a close while minimized also asks first (the window comes back for the prompt); other platforms unchanged. `cargo xtask layers` keeps `craft-quit` to the UI and app layers (#66) |
| 2026-10-16 | M2.0: part-day durations and lags: `30m`, `4h`, `1.5d`, `2w`, `1mo` (decimals allowed), converted with the project's hours per day / per week / days per month and rounded to the nearest minute; the unit typed is kept for display (`4h` stays `4h`, days to two decimals), in the Duration and Predecessors columns (`3FS+4h`) and the CLI; elapsed units and percent lags refused with the reason; commands take the text or whole days as before; Gantt bars start and end part-way through a day, in proportion to the day's working time; part-day worked examples (#53) |
| 2026-10-16 | M2: a finish-to-finish successor (task or summary) never finishes before a milestone predecessor at the start of a day: it finishes at that moment (Monday 8:00, not Friday 17:00), the same working time, so slack is unchanged; an intermittent property-test failure fixed (#74) |
| 2026-10-16 | SEC: in the desktop app, opening or saving a project (and writing its recovery copy) removes the `.<name>.<pid>-<n>.craft_tmp` staging files a killed save left beside it (or beside its `.bak`) once they are an hour old; only regular files named exactly for that file, never a symbolic link, at most 10,000 folder entries looked at; a file that can't be removed stays and the status bar says so; native only (#72) |
| 2026-10-16 | M2.2: assignments: a resource on a task with units (work: percent of working time, default 100% or the resource's Max Units if lower; material: a fixed quantity `5 tons` or a rate `2 tons/d` that scales with the duration; cost: an amount per assignment); `assignment.add`, `assignment.setUnits`, `assignment.remove` and `task.setResourceNames` by Unique ID; Resource Names column typed as Project writes it (`Ann[50%], Bob, Concrete[2 tons/d]`), an unknown name making a work resource in the same undo step with a status message; read-only Work column (duration × units, summaries roll up their own and their subtasks'), each assignment's work in its tooltip; Assign Resources dialog (toolbar, Alt+F10); resource names beside the Gantt bars and in the CLI; summaries take resources; deleting a task or resource removes its assignments, and changing an assigned resource's type asks first and drops them; durations stay as entered; additive `assignments` file field, validated on load (#55) |
| 2026-10-16 | M2.5: costs: one currency per project (ISO code, default GBP, its symbol and decimal places; JPY with none and BHD with three handled), set with `project.setOptions` (`currency`, `currencySymbol`, `currencyDigits`) and in Project › Options…; changing it keeps each amount's value (no exchange rates), rounding half to even when there are fewer places; money stays whole minor units with integer maths; an assignment costs its work × the standard rate (a material its quantity × the rate) plus cost/use, rounded half to even once; a cost resource its amount; a task its assignments plus its fixed cost (`task.setFixedCost`); summaries and the project roll up; Cost (read-only, each assignment's cost in its tooltip) and Fixed Cost columns, the project total in the status bar and the CLI; overtime rate and Accrue At stored for tracking; hostile amounts saturate, never overflow; worked example from Microsoft's "Enter costs for resources"; additive file fields, validated on load (#58) |
| 2026-10-16 | M2.3: task types and effort-driven scheduling: each task is Fixed Units (the default), Fixed Duration or Fixed Work, and each work assignment keeps its own units, work and duration (the task's is the longest); editing units, duration or work recalculates as Microsoft's table says (worked examples for all nine cells, with its own numbers); effort-driven tasks keep their work as resources are added or removed (shared in proportion to units; Fixed Units and Fixed Work change duration, Fixed Duration changes units), resources assigned together first each work the whole duration, Fixed Work is always effort-driven and summaries never are (scheduled as Fixed Duration); `task.setType` and `task.setWork` (hours, or text with a unit); `project.setOptions` takes the default task type and whether new tasks are effort-driven (off by default), shown under New tasks in Project › Options…; Type and Effort Driven columns, and the Work column can be typed into; costs follow each assignment's work (an effort-driven task's cost stays as resources are added); additive file fields (assignment `work` and `duration`, worked out on load for older files; a task whose duration isn't its longest assignment's is repaired on open, never refused); an assignment at 0% doesn't hold the duration; `cargo xtask perf` gates Resource Names with 1,000 names on a 5,000-task, 20,000-assignment plan under 100 ms (#56) |
| 2026-10-16 | M2: `projectcraft-cli` removes the staging files a killed save left beside a project after opening it and after `--save`, as the desktop app does; one it can't remove stays, with one line on stderr, and the command still succeeds (#87) |
| 2026-10-16 | M2: a duration typed on a task whose work assignments are of different lengths sets every one to the new duration (Fixed Units and Fixed Duration recalculate each one's work, Fixed Work each one's units); Microsoft's published pages give the rule for one assignment but not this case, so the choice is documented in the code and pinned by a worked example for all three task types (#88) |
| 2026-10-16 | M2: column chooser for the task sheet: View › Columns… ticks each column to show or hide it (Duration, Work, Start, Finish, Predecessors, Resource Names, Cost, Fixed Cost, Deadline, Type, Effort Driven), Show All puts every column back; ID and Task Name always show; `view.setColumns` takes the headings left to right, one undo step; the arrows skip hidden columns; the choice is saved with the project (an additive `columns` field: older files open with every column, a column name this version doesn't know is dropped) (#86) |
| 2026-10-16 | M2: Assign Resources acts on every selected task, as Microsoft Project's: Assign adds the resource to the tasks without it (the others keep their units), new units and Remove go to those with it, each one command and one undo step; a tick where all selected tasks have the resource, a dash where only some do; units and work shown are the focused task's. `assignment.add`, `assignment.setUnits` and `assignment.remove` take `tasks` (a list) in place of `task` (#82) |

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
| 2026-10-16 | minor | M1 progress; status line, headline numbers and revision history as MailCraft; links to private planning docs removed |
| 2026-10-16 | minor | Security hardening in the progress log |
| 2026-10-16 | minor | M1.1 summary tasks |
| 2026-10-16 | minor | M1.5 autosave and recovery copy |
| 2026-10-16 | minor | M1.4 calendar exceptions |
| 2026-10-16 | minor | M1.1 summary links |
| 2026-10-16 | minor | All M1 tickets landed; headline numbers refreshed; reschedule timings added |
| 2026-10-16 | minor | M1 reached |
| 2026-10-16 | minor | M2 checklist added |
| 2026-10-16 | minor | FF after a milestone at the start of a day (#74) |
| 2026-10-16 | minor | M2.2 assignments |
| 2026-10-16 | minor | M2.5 costs |
| 2026-10-16 | minor | Currency exchange listed as a gap; working time and Resource Sheet ticked |
| 2026-10-16 | minor | M2.3 task types |
| 2026-10-16 | minor | Headline numbers refreshed for M2 |
| 2026-10-16 | minor | CLI sweeps stale save temp files (#87) |
| 2026-10-16 | minor | Duration edit on assignments of different lengths (#88) |
| 2026-10-16 | minor | M4 scope and import gaps listed; XER moved after M4 |
| 2026-10-16 | minor | Task sheet column chooser (#86) |
| 2026-10-16 | minor | Assign Resources on every selected task (#82) |
