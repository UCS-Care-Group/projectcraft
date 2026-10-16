# Security policy

People trust ProjectCraft with plans that run for years, so a security problem is treated before
feature work. What ProjectCraft protects, from whom, and how is in
[`docs/threat-model.md`](docs/threat-model.md).

## Reporting a vulnerability

**Please don't open a public issue.** Report it privately through GitHub:
[Report a vulnerability](https://github.com/UCS-Care-Group/projectcraft/security/advisories/new)
(the repository's Security tab, "Report a vulnerability"). Only the reporter and the maintainers
see the report; we reply inside it.

Say what you found, the release or commit you ran (`git rev-parse --short HEAD`), the
platform, and how to reproduce it. Use invented projects and people in reproductions; never send
us a real organisation's plan.

We aim to acknowledge a report within 3 working days, to agree a fix and a date for disclosure
with you, and to credit you in the advisory unless you'd rather not be named.

## Supported versions

ProjectCraft is pre-alpha. Fixes go into the latest release only (and `main` until there is one).
Please check that a problem still happens there.

## What counts

Anything that breaks what ProjectCraft promises (see the threat model and the rules in
`AGENTS.md`), for example:

- a crash, hang, or runaway memory or CPU use caused by a project file, an imported file, a
  command's parameters or a command-line argument (a loop of links included);
- a failed command or a failed save that leaves the project, or the file on disk, half changed;
- a project file or import that makes ProjectCraft write outside the file the owner chose, or
  read or run anything else;
- ProjectCraft reaching the network by itself (it makes no connections);
- the control channel or the MCP server, once there, accepting a command without its key, or
  listening anywhere but loopback;
- anything in this repository's build or workflows that would let a pull request or a dependency
  reach secrets or publish a release.

## Known limits (not vulnerabilities)

- Project files aren't encrypted by ProjectCraft; it relies on the operating system's disk
  encryption (BitLocker, FileVault, LUKS).
- Saving keeps the previous save beside the file as `<file>.bak`, so text deleted from a plan is
  still in the `.bak` until the next save.
- Anyone who can run programs as you on your computer can read and change your plans; ProjectCraft
  doesn't protect against that.
