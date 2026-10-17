# Threat model

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-17 · **Change:** minor (Microsoft Project XML resources and costs) · **Target:** Microsoft Project

What ProjectCraft protects, from whom, and how, with the code that does it. Rules for working in
the code are in [`AGENTS.md`](../AGENTS.md); how to report a vulnerability is in
[`SECURITY.md`](../SECURITY.md). A claim here that the code doesn't keep is a bug: report it like
one.

## Assets

| Asset | Where it lives |
|---|---|
| The owner's plans: tasks, links, dates, calendars, resources and their rates, and in time costs and baselines | the `.projectcraft` file the owner chose, its `.bak` beside it, and memory while open |
| Personal data: the names (and initials and groups) of the people in a plan's Resource Sheet | the same files; ProjectCraft holds no contact details (no email, phone or address fields) |
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
  non-zero and unique, durations and lags within `MAX_MINUTES` (version-1 day counts within `MAX_DAYS` before they migrate to minutes), links only to tasks that exist; at most `MAX_RESOURCES` resources with non-zero, unique Unique IDs, unique names, and Max Units and amounts of money within their caps; at most `MAX_ASSIGNMENTS` assignments (`MAX_TASK_ASSIGNMENTS` per task), each to a task and resource that exist, once per pair, with units of its resource's type within their caps.
  A property test feeds arbitrary bytes to the loader.
- **No runaway scheduling.** Durations and lags are clamped to `MAX_MINUTES` again in the scheduler, the calendar walk is bounded,
  arithmetic is checked or saturating, and a loop of links is found by the ordering pass
  (`order` in `crates/model/src/schedule.rs`) and reported as an error, never followed.
- **A bad file is an error, not a crash:** the app prints why and exits, or the Open dialog
  reports it and keeps the project that was open.

### Imported files

Microsoft Project XML (MSPDI) and CSV now; Primavera XER later. Each is read by a parser of its
own in `crates/interchange` under the rules above: everything in the file is hostile, sizes and
counts are capped, and every parser has property tests that feed it arbitrary input. An import
becomes a project through the same validation as a `.projectcraft` file (`Session::import`,
`crates/engine/src/import.rs`), so the scheduler never sees anything the editing commands couldn't
have produced.

- **XML** (`crates/interchange/src/xml.rs`, over `quick-xml`): a file over 100 MB is refused
  before parsing, and so is one that isn't UTF-8. A document type definition (`<!DOCTYPE>`) is
  refused, so no entity can be declared (no billion laughs, no external entities, nothing is ever
  fetched); any entity other than XML's five and character references is refused; nesting is
  capped at 64, so a depth bomb is refused at the cap. The file's tasks are capped at
  `MAX_TASKS`, durations clamped to `MAX_MINUTES`, outline levels kept to the outline rule.
  Links are capped at 1,000,000 a file (the rest counted and reported), lags clamped to
  `MAX_MINUTES` with every conversion done in 128-bit arithmetic, and links that would close a
  loop dropped as for CSV; the report lines comparing the file's dates with ProjectCraft's stop
  at 1,000, then one line for the rest. Resources are capped at `MAX_RESOURCES` and assignments
  at `MAX_TASK_ASSIGNMENTS` a task and `MAX_ASSIGNMENTS` a file (the rest counted and reported);
  amounts, units and quantities are read as exact decimals (no floating point, 128-bit
  arithmetic, saturating) and clamped to the model's limits (`MAX_MONEY`, `MAX_UNITS`,
  `MAX_QUANTITY`, `MAX_WORK`); resource calendars' recurring exceptions share one budget of
  steps, and the exception days given to resources one cap, so a calendar shared by many
  resources can't multiply.
- **CSV** (`crates/interchange/src/csv.rs`, a reader of its own, no new dependency): a file over
  100 MB is refused, and so is one that isn't UTF-8 (a BOM is allowed); records are read one at a
  time, so a file of blank lines or separators holds no more than one row; columns are capped at
  1,000 (the rest counted and reported); tasks at `MAX_TASKS`. Cells go through the editing
  commands' own parsers (duration, predecessors, Resource Names) with their clamps; a cell that
  can't be read is a report line, never a failure. Links that would close a loop are dropped with
  a strongly-connected-parts pass and a bounded search, so a loop of any size can't stall the
  import or reach the scheduler.
- **An import never touches the open project:** it makes a new, untitled one, and Save never
  writes over the imported file (it asks for a `.projectcraft` name, and refuses an `.xml` or `.csv` one).
- Tests: billion laughs and depth bombs refused fast, arbitrary bytes and damaged files never
  panic (`crates/interchange/tests/mspdi.rs`, `crates/interchange/tests/mspdi_resources.rs`,
  `crates/interchange/tests/csv.rs`), and `file.import` is in
  `hostile_params_never_panic`.

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

- **Atomic saves.** The project file, its `.bak`, the recovery copy and the settings files are
  written through `craft-atomic` (storytold/craft-libs): the new content goes into a hidden
  staging file beside the file, is flushed to disk (a full flush on macOS), and is renamed into
  place, then the folder is flushed; on Windows a rename blocked for a moment by a virus scanner
  or indexer is retried. A symbolic link is kept and the file it names is written. Once the new
  content is written, and before it replaces the old, the previous save is copied to
  `<file>.bak` the same way (`replace_file` in `crates/engine/src/file.rs`). The file on disk is
  always the previous save or this one, never half of either; on any error the original is
  untouched and the staging file removed. A crash or power cut mid-save can leave a staging file
  (`.<name>.<pid>-<n>.craft_tmp`, owner-only) beside the file; it is never read. A test injects a
  disk that fails at the flush and at the rename and checks the old file survives whole.
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

A project file holds people's names (its resources), which is personal data. It is protected as
the plan is: owner-only on Unix (below), never sent anywhere (no network), and its owner decides
where it is kept and who it is shared with. Deleting a resource removes its name from the next
save, but the `.bak` keeps the previous save until the one after.

- On Unix (Linux, macOS, the BSDs) every file ProjectCraft writes is owner-only, mode 0600: the
  project file, the `.bak`, the recovery copy, and the settings files (`recent-files.txt`,
  `autosave.txt`). Each is staged with craft-atomic's private permissions, so it is 0600 from its
  first byte and never readable by others for a moment. craft-atomic keeps the mode of a file it
  replaces, so an existing file or `.bak` with wider permissions is first tightened to 0600; one
  owned by another account can't be tightened and keeps its mode. Settings and recovery folders
  ProjectCraft creates are 0700; a folder that already exists, such as the one the owner saves
  into, is left as it is. `replace_file`, `write_private` and `create_private_dir_all` in
  `crates/engine/src/file.rs`, with Unix-only tests there and in `crates/ui-egui/src/recovery.rs`.
- On Windows the files get the folder's access list, like any document; the settings folder is in
  the owner's profile. The web build writes no files.
- The recent-files list holds paths only, read back capped at 64 KB, and entries that no longer
  exist are dropped.
- Error messages name the file that failed (on standard error, or in the app). They never carry
  a plan's contents. There is no log file.

### A supply-chain compromise

A malicious or vulnerable dependency, GitHub Action or build step.

- **Dependencies:** `cargo deny check` ([`deny.toml`](../deny.toml); `cargo xtask deny`, a step of
  `cargo xtask ci` and a CI job that `ci-ok` requires) fails on a RustSec vulnerability, allows
  only OSI-permissive licences (no GPL, LGPL or AGPL) plus one font licence for one crate, and only
  crates.io as a source, plus git from the Craft suite's own `storytold/craft-libs` alone, pinned
  to a commit by `rev`. `Cargo.lock` is committed.
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
| 2026-10-16 | minor | Owner-only permissions on Unix: files written 0600, new settings and recovery folders 0700 (#37) |
| 2026-10-16 | minor | Resources: project files hold people's names (personal data, no contact details); resources validated on load (#54) |
| 2026-10-16 | minor | Durations and lags capped in working minutes; version-1 files migrate with the day cap kept (#52) |
| 2026-10-16 | minor | Saves, `.bak`, recovery copy and settings written through craft-atomic; craft-libs allowed as a pinned git source (#67) |
| 2026-10-16 | minor | Assignments validated on load: counts capped, tasks and resources must exist, units in range (#55) |
| 2026-10-17 | minor | Microsoft Project XML import: DTDs and entities refused, 100 MB and depth 64 caps, imports validated as project files, never written back (#97) |
| 2026-10-17 | minor | CSV import: own RFC 4180 reader, records streamed, 1,000-column cap, loops of links dropped in bounded time (#103) |
| 2026-10-17 | minor | Microsoft Project XML links: 1,000,000-link cap, lags clamped, loops dropped, date-comparison lines bounded (#98) |
| 2026-10-17 | minor | Microsoft Project XML resources and costs: resource and assignment caps, exact decimal money and units clamped to the model, one exception budget for resource calendars (#101) |
