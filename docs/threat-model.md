# Threat model

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change:** major (first version) · **Target:** Microsoft Project

What ProjectCraft protects, from whom, and how, with the code that does it. Rules for working in
the code are in [`AGENTS.md`](../AGENTS.md); how to report a vulnerability is in
[`SECURITY.md`](../SECURITY.md). A claim here that the code doesn't keep is a bug: report it like
one.

## Assets

| Asset | Where it lives |
|---|---|
| The owner's plans: tasks, links, dates, calendars, and in time resources, costs and baselines | the `.projectcraft` file the owner chose, its `.bak` beside it, and memory while open |
| The owner's work in progress | memory (the session and its undo stack) until it is saved |
| Which plans the owner has opened | the recent-files list (`recent-files.txt` in the settings folder: `apps/projectcraft/src/settings.rs`) |
| The owner's trust that the dates are right | the scheduler (`crates/model/src/schedule.rs`), deterministic: the same plan gives the same dates |

## What ProjectCraft does not do

- **No network.** ProjectCraft makes no connections: no update check, no telemetry, no fonts or
  pictures from the web, no cloud storage. There is no HTTP or TLS library in the dependency tree.
- **No code in a plan runs.** A project file is data (JSON); there are no macros, scripts,
  formulas that call out, or embedded objects.
- **No listening socket.** Nothing listens on a port today (see "The control channel and MCP").

## Adversaries and what ProjectCraft does about them

### A hostile project file

A `.projectcraft` file from somewhere else: an email attachment, a shared drive, a download. It
may be malformed, enormous, or crafted to make the scheduler loop or overflow.

- **Parsers never crash.** No `unwrap`, `expect`, `panic!` or `unsafe` in production code
  (`unsafe_code = "forbid"` in the workspace `Cargo.toml`, the clippy denials at every crate root).
  Reading a file (`crates/engine/src/file.rs`) is capped at 128 MB before parsing, must be UTF-8
  JSON, refuses a newer `format_version`, and is validated after: at most `MAX_TASKS` tasks, ids
  non-zero and unique, durations and lags within `MAX_DAYS`, links only to tasks that exist.
  A property test feeds arbitrary bytes to the loader.
- **No runaway scheduling.** Durations and lags are clamped to `MAX_DAYS` again in the scheduler,
  arithmetic is checked or saturating, and a loop of links is found by the ordering pass
  (`order` in `crates/model/src/schedule.rs`) and reported as an error, never followed.
- **A bad file is an error, not a crash:** the app prints why and exits, or the Open dialog
  reports it and keeps the project that was open.

### Imported files

Microsoft Project XML (MSPDI), Primavera XER and CSV, once they can be imported. Each is read by a
parser of its own under the rules above: everything in the file is hostile, sizes and counts are
capped, and every parser has property tests that feed it arbitrary input. An import becomes a
project through the same validation as a `.projectcraft` file, so the scheduler never sees
anything the editing commands couldn't have produced.

### Hostile command parameters

Whatever drives the commands: the command line (`projectcraft-cli run --cmd …`), and later the
control channel and MCP, where the caller may be a script or an AI agent.

- **One door.** Every action is a command in one registry (`COMMANDS`, `crates/engine/src/lib.rs`);
  nothing edits a project any other way. `Session::run` checks the parameters against the
  command's schema first (unknown or mistyped fields are refused), strips control characters from
  names and caps them at 255 characters.
- **All or nothing.** A command that returns an error, or panics, leaves the project exactly as it
  was: `Session::run` restores it and turns an escaped panic into an error.
- `hostile_params_never_panic` (`crates/engine/src/lib.rs`) runs every command with junk
  parameters and checks both.

### Losing the owner's work

Not an attacker, but the reason the program exists: a crash, a full disk or a power cut while
saving.

- **Atomic saves.** A save writes `<file>.tmp` beside the file, flushes it to disk, copies the
  previous save to `<file>.bak`, then renames the new one into place (`replace_file` in
  `crates/engine/src/file.rs`). The file on disk is always the previous save or this one, never
  half of either; on any error the original is untouched and the temp file removed.
- **Never crash** (above) is the main defence: an error is reported and the project stays open.
- **No autosave or crash recovery yet.** Unsaved changes live only in memory. When autosave and
  recovery arrive, their files go beside the project or in the settings folder, are written with
  the same atomic replace, are read back through the same validation as any project file (a
  recovery file is hostile input too), and are removed once the project is saved.

### The control channel and MCP

ProjectCraft has neither yet; the command registry is built for them. When they arrive they follow
MailCraft's: off unless the owner starts the app with them; listening on `127.0.0.1` only, never
another interface; every request carrying a key of 32 random bytes from the operating system,
kept in a key file only the owner can read and compared in constant time; and every request going
through `Session::run` like any other command, with the same schema checks. Destructive and file
commands are marked as such in the registry, so an agent can be refused them.

### Other programs and users on the same computer

- Project files are written where the owner chooses, with the operating system's default
  permissions for a new file (the owner's umask on Unix, the folder's access list on Windows),
  like any document. The settings folder is in the owner's profile.
- The recent-files list holds paths only, read back capped at 64 KB, and entries that no longer
  exist are dropped.
- Error messages name the file that failed (on standard error, or in the app). They never carry
  a plan's contents. There is no log file.

### A supply-chain compromise

A malicious or vulnerable dependency, GitHub Action or build step.

- **Dependencies:** `cargo deny check` ([`deny.toml`](../deny.toml); `cargo xtask deny`, a step of
  `cargo xtask ci` and a CI job that `ci-ok` requires) fails on a RustSec vulnerability, allows
  only OSI-permissive licences (no GPL, LGPL or AGPL) plus one font licence for one crate, and only
  crates.io as a source. `Cargo.lock` is committed.
- **Workflows:** every third-party action is pinned to a full commit SHA, kept current by
  Dependabot with a week's cooldown (Cargo updates too); checkouts don't keep credentials;
  workflows get read-only permissions unless a job needs more.
  [zizmor](https://docs.zizmor.sh/) checks the workflows on every change to them, and
  [OpenSSF Scorecard](https://scorecard.dev/) scores the repository weekly.
- **Releases:** there is no release workflow yet; the rules it must follow are in
  [`releasing.md`](releasing.md).
- Rust only, no handwritten JavaScript.

## Out of scope and known limits

- **Malware running as the owner.** A program with the owner's rights can read and change every
  plan. ProjectCraft doesn't defend against its own user's account being compromised.
- **Plans aren't encrypted at rest.** ProjectCraft relies on the operating system's disk
  encryption (BitLocker, FileVault, LUKS, GELI).
- **The `.bak` file** keeps the previous save, so something deleted from a plan is still on disk
  there until the next save.
- **Unsigned builds.** There are no signed builds yet; build from source or check a download
  against its published checksums once releases exist.
- **The web build** runs in the browser's sandbox and inherits the browser's security.

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | major | First version: assets, what the program doesn't do, adversaries and mechanisms with their files, known limits |
