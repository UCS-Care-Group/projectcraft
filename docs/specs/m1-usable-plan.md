# M1 — A plan you can run a real project on

> **Status:** draft (to be grilled) · **Written:** 2026-10-15 · **Change:** major (first version)

## Problem
After M0 an owner can list tasks and links and see the critical path, but cannot structure a plan (no summary tasks), reorder it, change the calendar, pin a date, or open and save files without the command line. Nobody would run a real project on it.

## Goal
An owner can build and maintain a 50–200 task plan for a real project entirely in the desktop app, and the dates match what Microsoft Project computes for the same plan.

## In scope
1. **Summary tasks**: outline level per task; indent/outdent commands; a summary's start/finish/duration rolled up from its children; links to and from summaries.
2. **Row editing**: insert above, move up/down, multi-select, delete selection; keyboard navigation in the sheet (arrows, Enter, F2, Insert, Delete).
3. **Calendar**: edit working weekdays and holidays in a dialog; the Gantt timescale in days, weeks or months with non-working days shaded.
4. **Constraints and deadlines**: As Soon As Possible (default), Start/Finish No Earlier/Later Than, Must Start/Finish On; a deadline marker; a warning when a constraint makes slack negative.
5. **Files**: Open/Save/Save As dialogs (`rfd`), autosave, recent files, unsaved-changes prompt.
6. **Look**: theme tokens with light and dark, text through `tl!`, screenshots checked.

## Out of scope
Resources, baselines, tracking, MSPDI import (later milestones); part-day durations; multiple calendars.

## Acceptance (pass/fail)
- [ ] Five reference plans (worked examples with expected dates written by hand from Project's documented rules) schedule to the expected dates in tests.
- [ ] A 200-task sample plan reschedules in under 10 ms in release.
- [ ] Every new action is a command, listed by `plancraft-cli commands`, covered by `hostile_params_never_panic`.
- [ ] `cargo xtask ci` green; light and dark screenshots of the Gantt Chart reviewed.

## Open questions
- File format: stay on JSON, or adopt MSPDI XML as the native format early?
- Outline model: parent pointers vs. outline level per row (Project uses levels).
- Do summaries take links in M1, or is that deferred?
