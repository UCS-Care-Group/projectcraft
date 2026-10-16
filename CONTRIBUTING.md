# Contributing to ProjectCraft

> **Status:** current · **Written:** 2026-10-16 · **Change:** minor (planning docs private; review wording)

## Before you build anything
1. **Read [AGENTS.md](AGENTS.md).** It is for people as much as for agents: never crash, the schedule is computed, no vendor assets, clean room. None of it is negotiable.
2. **Read the roadmap and the issue.** [`ROADMAP.md`](ROADMAP.md) shows the direction and each issue says what to build. Specs and plans are kept privately by each contributor, for clean-room work; they are never committed.
3. **Claim an issue** before you build it. Something not in the issues: open one and wait for a yes.

## The pull request
- **One issue, one pull request**, based on current `main`. Say `Closes #<issue>`.
- **`cargo xtask ci` passes** before every commit. Commits carry the task id (`M1.2: task calendars`).
- **No AI attribution**: no `Co-Authored-By` trailer and no "Generated with" line in commits, pull requests, issues or comments.
- **Every crash fix and every scheduling fix comes with a regression test** (a worked example with its expected dates).
- **Changes to the scheduler and the file format** (`crates/model`) are read by a maintainer before they land.

## Decisions that are the owners', not a contributor's
Ask in an issue first: a change to `AGENTS.md` or to an agreed decision; a new crate; a new dependency with a licence other than MIT, Apache-2.0, BSD or ISC; a new asset; anything that sends something off the computer.

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
| 2026-10-16 | minor | Specs, plans and decisions kept private; review promised as read by a maintainer; no AI attribution |
