# ProjectCraft

**An open-source project scheduling program, written from scratch in Rust.**

ProjectCraft is a clean-room reimplementation of the project planner people already know how to use: the task sheet with its Gantt Chart, links between tasks, working calendars, the critical path, and in time resources, baselines and tracking. It is a native, GPU-rendered app (no Electron, no web view), it is built to run in a browser tab through WebAssembly too, and every action in it is a command that scripts and AI agents can run.

> **Status: pre-alpha (0.0.1).** The foundations are in place: the scheduler, the command engine and a first Gantt Chart view. See [ROADMAP.md](ROADMAP.md).

ProjectCraft follows the architecture and conventions of [MailCraft](https://github.com/UCS-Care-Group/mailcraft), [WordCraft](https://github.com/storytold/wordcraft) and the other Craft apps: the same layered crates, the same "everything is a command" engine, the same egui front end, tooling and quality gates.

## What works today
- **Tasks:** add, rename, change duration, delete; milestones (duration 0).
- **Links:** finish-to-start, start-to-start, finish-to-finish, start-to-finish, with lag or lead, typed in the Predecessors column (`3, 5SS+2, 7FF-1`). Loops are refused.
- **Scheduling:** forward and backward pass over working days (Monday to Friday, plus holidays in the file); total slack; critical tasks in red.
- **Gantt Chart:** the task sheet with bars and milestone diamonds; undo and redo for every change.
- **Files:** `.projectcraft` (JSON).
- **Command line:** every command, and the schedule printed as a table.

## Build and run
```
git clone https://github.com/UCS-Care-Group/projectcraft
cd projectcraft
cargo run --release -p projectcraft -- --sample
```
`--sample` opens an invented project and keeps nothing. `projectcraft plan.projectcraft` opens or starts a file; Ctrl+S saves it.

**Command line:**
```
projectcraft-cli schedule --sample
projectcraft-cli run --sample --cmd 'task.add={"name":"Snag list","duration":3}' --cmd 'task.setPredecessors={"id":9,"text":"7"}' --save demo.projectcraft
projectcraft-cli commands
```

**Everything CI runs:** `cargo xtask ci` (formatting, clippy with warnings as errors, the tests, asset attribution, crate layering, the wasm build, and `cargo deny` for advisories, licences and sources once `cargo install cargo-deny --locked` has run).

## How it is put together
| Layer | Crates | Job |
|---|---|---|
| L0 | `time` | Civil dates and ISO date text |
| L1 | `model` | Tasks, links, calendars, the critical-path scheduler |
| L4 | `engine` | Session, undo, the command registry, the sample, file I/O |
| L6 | `ui-egui` | The Gantt Chart front end (swappable) |
| Apps | `projectcraft`, `projectcraft-cli` | Desktop, command line |

A crate may only depend on lower layers, and nothing below L6 knows about the UI toolkit (`cargo xtask layers` enforces both). More in [docs/architecture.md](docs/architecture.md).

## Principles
- **Never crash.** No panics in non-test code; every input treated as hostile; a failed command leaves the project exactly as it was.
- **The schedule is computed, never typed.** Same plan, same dates, on every machine.
- **Clean room.** Built from public documentation of how project scheduling works and from watching how planners behave, never from anyone's code or assets.
- **Everything is a command.** One registry behind the interface, the CLI and (soon) agents.

## Security
Report a vulnerability privately: see [SECURITY.md](SECURITY.md). What ProjectCraft protects and how is in [docs/threat-model.md](docs/threat-model.md).

## Contributing
Read [CONTRIBUTING.md](CONTRIBUTING.md) and [AGENTS.md](AGENTS.md) first.

## License
ProjectCraft is dual-licensed under the [MIT License](LICENSE-MIT) or the [Apache License, Version 2.0](LICENSE-APACHE), at your option. See [NOTICE](NOTICE). ProjectCraft is not affiliated with Microsoft; Microsoft Project is a trademark of Microsoft Corporation, named here only to say what ProjectCraft is an alternative to.
