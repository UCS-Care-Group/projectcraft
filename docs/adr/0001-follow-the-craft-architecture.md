# 0001. Follow the Craft architecture

> **Status:** proposed · **Date:** 2026-10-15 · **Deciders:** Martin Ball

## Context
ProjectCraft is a new Craft app (a Microsoft Project alternative). MailCraft and WordCraft already prove one architecture: layered crates, an engine where every action is a command, an egui front end, `cargo xtask ci` gates, never-crash rules and a strict asset policy. Contributors and agents move between the Craft repos.

## Decision
ProjectCraft uses the same architecture, tooling and rules: Rust only, egui/eframe for the UI, wasm kept building, the layering checked by `cargo xtask layers`, the never-crash lint set on every production crate, and the asset policy unchanged. The project file is JSON (`.projectcraft`) until a decision says otherwise.

## Alternatives
- **A web stack (TypeScript, a canvas Gantt library):** faster first screens, but breaks with every sibling and the Rust-only rule.
- **Adopt an existing scheduler crate:** none found that is maintained, clean-room and covers Project's link and calendar semantics; the critical path is a small, well-documented algorithm.

## Consequences
- Tooling (`xtask`, CI) is copied from MailCraft and kept close to it.
- The Gantt Chart is painted with egui; there is no web view.
- Microsoft Project interchange (MSPDI XML) is its own L2 crate when it comes.
