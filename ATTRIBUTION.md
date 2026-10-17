# Asset attribution

Every non-code asset in this repository (fonts, icons, images, example files) is listed here with its author, source and licence. `cargo xtask assets` (part of `cargo xtask ci`) fails if an asset file is missing from this table.

**Policy (mandatory):** ProjectCraft contains **no Microsoft or other vendor's iconography, images, templates, themes, sounds, fonts or other assets**. Every asset is original work by ProjectCraft contributors or third-party material under an open licence (OSI open source, public domain / CC0, or a Creative Commons licence that allows redistribution). Screenshots of other vendors' software are never committed.

Generated-in-code art is original and has no file to list: the Gantt bars and milestone diamonds (`crates/ui-egui/src/lib.rs`) and the sample project (`crates/engine/src/lib.rs`, `sample`).

| Asset | Author | Source | Licence | Notes |
|---|---|---|---|---|
| `crates/engine/tests/fixtures/m1/` | ProjectCraft contributors | saved by ProjectCraft at its last M1 commit from invented plans (test fixtures) | MIT OR Apache-2.0 | invented tasks; no real organisation's plan |
| `crates/interchange/tests/fixtures/garden-studio.xml` | ProjectCraft contributors | Hand-written for the tests from Microsoft's published MSPDI schema documentation (learn.microsoft.com/office-project/xml-data-interchange); no Microsoft file used | MIT OR Apache-2.0 | Invented plan: tasks, people and dates are made up |
