# 0004. How agents (AI and scripts) reach a project

> **Status:** accepted (groundwork in M1, MCP in M5) · **Date:** 2026-10-15 · **Deciders:** Martin Ball

## Context
ProjectCraft will be driven by agents through the CLI, a control channel and MCP. People trust it with plans that run for years, and plans imported from elsewhere can carry text written to steer an AI.

MailCraft and WordCraft already share a design, which ADR 0001 says to copy:
- **Control channel:** JSON lines over loopback TCP, with a random 256-bit key in a 0600 file that's new on every start.
- **MCP:** stdio via `<app>-cli mcp`, either headless or `--connect` to the running app, with generic `list_commands`/`execute`/`batch` tools.
- **Path confinement:** `--automation-read-root/--write-root`.

Their known gaps:
- params are free text with no schema, and unknown fields are ignored;
- read/write is marked per tool, not per command;
- prompt-injection defence is wording only;
- no confirmation for destructive changes.

## Decision
Copy the sibling transport, auth and path confinement, and close their gaps:
1. **One registry, typed.** Every `CommandSpec` has a param JSON Schema and an effect: `read`, `edit`, `destructive` or `file`. MCP tools are generated from it, never hand-written. Unknown params are refused. *(Lands in M1.0.)*
2. **Tools split by effect.** `query` runs read commands and is marked `readOnlyHint`. `execute` runs edits. `list_commands` returns schemas and effects. `--read-only` refuses `execute`.
3. **The open project only.** No network listener beyond loopback. `file` commands are refused over MCP unless an automation root is given. A connection whose first line looks like HTTP is dropped, so browser pages can't reach the port.
4. **The owner stays in control.**
   - Every agent change is one undo step labelled "AI: …".
   - A change log of agent edits.
   - A **destructive change** (deleting tasks, or one command changing more than 10 tasks) needs the owner's confirmation when the desktop app is attached. Headless, it's refused unless an explicit flag allows it.
5. **Untrusted text is marked.** Task names and notes go back to the agent as data fields flagged untrusted, not mixed into instructions.
6. **Built to plan well.**
   - `schedule.explain {task}` returns what drove the date, using the scheduler's driver record from M1.
   - `what_if {commands}` runs the commands on a copy and returns the date and critical-path changes.
   - `project.summary` returns the outline and the critical path, sized for a context window.
7. **Bounded.** Size and count caps per call (`MAX_TASKS`, line length), as for every outside input.

## Consequences
- M1 pays for the metadata (schemas, effects, the driver record). M5 is mostly wiring.
- Points 1–3 are worth offering to MailCraft and WordCraft, and to craftrules (private), as suite rules. They aren't changed from here.
