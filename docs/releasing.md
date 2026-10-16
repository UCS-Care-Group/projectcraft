# Releasing ProjectCraft

> **Last reviewed:** 2026-10-16 · **Last updated:** 2026-10-16 · **Change:** major (first version) · **Status:** no release workflow exists yet; nothing has been released.

ProjectCraft has no release workflow, no packaging and no signing yet. There are no published
builds: build from source (`README.md`). This file holds the rules the release workflow must
follow when it is written, so that it starts out safe rather than being made safe later. A
release workflow that breaks one of them is a security bug.

The general recipe for a Craft app is [craftrules `release/playbook.md`](https://github.com/storytold/craftrules/blob/main/release/playbook.md).

## Rules for the release workflow

1. **Signing secrets live only in a protected `release` environment.** Code-signing and
   notarization secrets (Apple, Windows/Azure) are environment secrets of an environment named
   `release`, never repository or organisation secrets. The environment has a required reviewer,
   and may deploy only from tags matching `v*`, which are protected so that only maintainers can
   create them. Only the job that signs uses the environment.
2. **Secrets are passed by name.** A called workflow gets the secrets it needs listed one by one,
   never `secrets: inherit`.
3. **A release is published as a draft.** The workflow creates the GitHub release as a draft with
   every file and a `SHA256SUMS.txt`; a maintainer checks it and publishes it by hand.
4. **A dry run gets no secrets.** Run by hand (`workflow_dispatch`) on a branch, the workflow
   builds everything unsigned and keeps the files as workflow artifacts. It never enters the
   `release` environment and never creates a release.
5. **The tag is the version.** The workflow refuses a tag that isn't the version in `Cargo.toml`
   (`[workspace.package] version`).
6. **Third-party tools are pinned.** Every action is pinned to a full commit SHA (as in every
   workflow here). Every tool downloaded during the build (packagers, signing tools, linters) is
   fetched at an exact version and checked against a sha256 written in the workflow before it runs;
   no `curl | sh`, no `latest`.
7. **Least privilege.** The workflow's default permissions are read-only; only the job that
   creates the draft release gets `contents: write`. Checkouts don't keep credentials
   (`persist-credentials: false`).
8. **Checked like every workflow.** zizmor runs on it with no unexplained ignores, and
   `cargo xtask ci` (including `cargo deny`) passes on the tagged commit.

## Before the first release (repository settings)

These are settings, not files, so the workflow can't set them:

- Create the `release` environment with a required reviewer and a deployment rule for `v*` tags.
- Protect `v*` tags (a tag ruleset) so only maintainers can create, move or delete them.
- Keep private vulnerability reporting on (`SECURITY.md` relies on it).

## Revision history

| Date | Change | Summary |
|---|---|---|
| 2026-10-16 | major | First version: no release workflow yet; the rules it must follow |
