# Contributing to PlanCraft

> **Status:** current · **Written:** 2026-10-15 · **Change:** major (first version)

The words here have the meanings in [`CONTEXT.md`](CONTEXT.md).

## Before you build anything
1. **Read [AGENTS.md](AGENTS.md).** It is for people as much as for agents: never crash, the schedule is computed, no vendor assets, clean room. None of it is negotiable.
2. **Read the plan.** Work follows the specs in [`docs/specs/`](docs/specs/), their plans in [`docs/plans/`](docs/plans/), and the decisions in [`docs/adr/`](docs/adr/). Work that goes against an accepted decision won't be merged; propose a change to the decision first.
3. **Claim an issue** before you build it. Something not in the issues: open one and wait for a yes.

## The pull request
- **One issue, one pull request**, based on current `main`. Say `Closes #<issue>`.
- **`cargo xtask ci` passes** before every commit. Commits carry the task id (`M1.2: task calendars`).
- **Every crash fix and every scheduling fix comes with a regression test** (a worked example with its expected dates).
- **The scheduler and the file format** (`crates/model`, see [`CODEOWNERS`](.github/CODEOWNERS)) are reviewed line by line.

## Decisions that are the owners', not a contributor's
Ask in an issue first: a change to `AGENTS.md`, an ADR or a spec; a new crate; a new dependency with a licence other than MIT, Apache-2.0, BSD or ISC; a new asset; anything that sends something off the computer.

## Revision history
| Date | Change | Summary |
|---|---|---|
| 2026-10-15 | major | First version |
