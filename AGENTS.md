# PlanCraft — instructions for agents

PlanCraft is a clean-room, open-source, Rust-native project scheduling program targeting Microsoft Project parity: tasks, links, working calendars, the critical path, the Gantt Chart, resources, baselines and tracking. It runs natively on macOS, Windows, Linux and BSD, and on the web via WASM. It follows the conventions of its siblings: `../mailcraft` (Outlook), `../wordcraft` (Word), `gridcraft` (Excel) and the rest. Shared rules and learnings: [`storytold/craftrules`](https://github.com/storytold/craftrules) — read its `AGENTS.md`.

## Start every session here
1. Read `plan/STATUS.md` (current milestone, next task) if it exists. `plan/` is gitignored (local only); if it's missing, start from `ROADMAP.md` and the newest spec and plan in `docs/specs/` and `docs/plans/`.
2. Don't stop to ask unless a decision is genuinely the owner's (licensing, publishing, pushing to new remotes, secrets, a new dependency or crate, anything in an ADR).

## Never crash
People trust PlanCraft with plans that run for years; a crash loses their work and their trust. **This outranks feature work.** Standard: [`craftrules/standards/never-crash.md`](https://github.com/storytold/craftrules/blob/main/standards/never-crash.md).
- **No panics in non-test code:** no `unwrap()`, `expect()`, `panic!`, `unreachable!`, `todo!`, `unimplemented!`; no `unsafe` (`unsafe_code = "forbid"`).
- **Errors are `Result<T, E>`** through the crate's error type and `?`. An unfinished feature returns an error or is disabled; it never panics.
- **Everything from outside is hostile:** project files, imported MSPDI/XER/CSV, command and CLI params. `get()` instead of `[i]`, checked/saturating arithmetic, clamp durations and lags (`model::MAX_DAYS`), cap task counts (`model::MAX_TASKS`), bound loops. A loaded file can hold a loop of links: the scheduler reports it, never spins.
- **Last-resort guard:** `Session::run` turns an escaped panic into an error and restores the project; a command that returns an error leaves the project as it was.
- **Prove it:** every crash fix lands with a regression test. `crates/engine/src/lib.rs::hostile_params_never_panic` runs every command with junk params — keep it green. Parsers have property tests that feed arbitrary input.
- Production crate roots carry `#![deny(clippy::unwrap_used, clippy::expect_used, clippy::panic, clippy::unimplemented, clippy::todo, clippy::unreachable)]`; new crates start with it. Tests may `unwrap` (`clippy.toml`).

## The schedule is the product
- **Dates come from the scheduler, never from the UI.** The UI shows `Session::schedule`; nothing writes a start or finish except `model::schedule`.
- **Same plan, same dates.** Scheduling is deterministic: no clock, no hash-order dependence in results. The system clock is read only by the desktop app, for a new project's start date.
- **Parity is checked against the published behaviour of Microsoft Project** (its documentation of task types, link types, calendars and slack), and against worked examples in the tests; never against its code or files we have no right to.

## Assets: absolutely no Microsoft iconography or images
**This rule is non-negotiable.** Breaking it is the worst mistake you can make in this repo.
- Never copy, trace, screenshot-crop or recreate icons, images, templates, themes, sounds, fonts or any other asset from Microsoft Project, Office, or any other vendor's product. Not even "temporarily".
- Every asset must be **original** (drawn in code — prefer this), **public domain / CC0**, **OSI open source**, or **Creative Commons that allows redistribution**.
- **Every asset file has a row in [`ATTRIBUTION.md`](ATTRIBUTION.md)** (author, source, licence). `cargo xtask assets` (part of `cargo xtask ci`) fails otherwise. Add the row in the same commit as the file.
- Sample projects are our own invented tasks and people. Never commit a real organisation's plan.

## Architecture
- **Everything is a command.** One registry (`crates/engine/src/lib.rs`, `COMMANDS`) behind the toolbar, the task sheet, the CLI, and later the control channel and MCP. A new action is a new `CommandSpec`, never UI code that edits the project directly.
- **Layering** is enforced by `cargo xtask layers` (table in `xtask/src/layers.rs`, explained in `docs/architecture.md`). Nothing below L6 depends on egui/eframe/winit/rfd.
- **Rust only** (no handwritten JS/TS). **Never break wasm** (`cargo xtask wasm`).

## Quality gates
Before every commit: `cargo xtask ci` (fmt, clippy -D warnings, tests, assets, layers, wasm). Commit after every arc of work that builds, with a task id in the message (`M1.2: task calendars`). Work follows a spec and an implementation plan in `docs/specs/` and `docs/plans/`; decisions that constrain later work get an ADR in `docs/adr/`.

## Running it
- `cargo run --release -p plancraft -- --sample` (the sample project; keeps nothing).
- `cargo run --release -p plancraft -- plan.plancraft` (opens or creates; Ctrl+S saves).
- `cargo run -p plancraft-cli -- schedule --sample`, `… run --sample --cmd 'task.add={"name":"Snag list","duration":3}' --save out.plancraft`, `… commands`.
- **For UI work, look at the result** (screenshot, read the PNG), in light and dark.
- Parallel agents: separate `CARGO_TARGET_DIR` per agent; edit only the crates you own.

## Roadmap
`ROADMAP.md` holds the milestones and the progress log. When work lands, update it and bump each touched doc's status line and revision history.
