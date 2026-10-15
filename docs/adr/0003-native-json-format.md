# 0003. The native file stays versioned JSON; MSPDI is for exchange

> **Status:** accepted · **Date:** 2026-10-15 · **Deciders:** Martin Ball

## Context
`.projectcraft` files are serde JSON of `Project`. Microsoft Project's XML (MSPDI) was considered as the native format, so we could open and save Project's files directly. MPXJ's documentation and other planners' issue trackers show what that costs:
- MSPDI can't hold views or settings.
- Project recomputes summaries and dates on import.
- The calendar exception format changed in Project 2003.
- Exports from GanttProject, Bryntum and OmniPlan have repeatedly failed to round-trip.

## Decision
The native format stays JSON. Every file carries a `format_version`:
- a file with a newer major version is refused with a clear message;
- older files are migrated on load.

Saves are atomic: temp file in the same folder, fsync, rename, one rolling `.bak`. MSPDI (and CSV, XER) are import/export formats only, in M4.

## Consequences
- We can add fields (outline level, constraints, exceptions) without waiting on MSPDI's shape.
- Opening a Project file is always an import that can report what it dropped. It's never a silent load.
- Migration code lives forever for every released format version.
