# AGENTS.md — coven-runtimes

Guidance for **AI agents** (Codex, Claude Code, Hermes, and any Coven familiar)
opening pull requests against this repo. Humans: this is useful too, but your
canonical guide is [`CONTRIBUTING.md`](CONTRIBUTING.md).

> **Read first:** [`README.md`](README.md) for what this repo *is*, and
> [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full contribution bar. This file
> is the agent-specific layer on top of those.

---

## What this repo is (one line)

The runtime SDK, conformance toolkit (`conjure`), and registry for teaching
Coven about a new **runtime** (an agent CLI it drives) *without editing `coven`
core*. Two contribution types with different bars: **(1)** adding/updating a
runtime *adapter manifest*, **(2)** changing the *SDK crates*. See
[`CONTRIBUTING.md`](CONTRIBUTING.md) §1 and §2.

## Branch & PR workflow (all agents)

- **Never push to `main`.** Every change lands via a PR with green CI. Branch
  from current `origin/main`.
- Use a **fresh branch per task**; if multiple sessions may touch this repo,
  work in a git worktree so operations don't race:
  ```sh
  git fetch origin main
  git worktree add -b <branch> /tmp/<repo>-<branch> origin/main
  ```
- Keep the diff **scoped to one concern**. No drive-by refactors in a feature PR.
- Conventional commit subjects: `feat:`, `fix:`, `docs:`, `chore:`, `refactor:`.
- Fill in [`.github/PULL_REQUEST_TEMPLATE.md`](.github/PULL_REQUEST_TEMPLATE.md);
  its checklist is the merge bar.
- After merge: delete the remote branch and remove your local worktree/branch.

## CI gates — run locally before you open the PR

CI (`.github/workflows/ci.yml`) will reject on any of these. Run them first:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo deny check advisories licenses bans sources   # if cargo-deny installed
```

For an **adapter manifest** PR, also:

```sh
conjure validate <manifest>.json --verbose   # must pass with ZERO problems
conjure test <manifest>.json                 # conformance vs the real binary
```

There are **no exceptions for `-D warnings`**. Do not declare a capability the
runtime can't honor — a false `stream: true` hangs a real session.

## Repo-specific invariants (don't break these)

- **`coven-runtime-spec` stays pure** — no async, no network, no process
  spawning, no filesystem. `coven` core depends on it.
- **Manifest is a backward-compatible superset** of coven's
  `ExternalHarnessAdapterSpec`. Add fields, don't rename/remove ones coven reads.
  New fields need snake_case + camelCase serde aliases **and** a matching entry
  in [`schema/adapter-manifest.schema.json`](schema/adapter-manifest.schema.json).
- **Touch the manifest shape → update the JSON Schema in the same PR.** The
  `schema_examples` test fails if they drift.
- **Bump `SCHEMA_VERSION`** on any backward-incompatible manifest change.

## Attribution — credit external contributors correctly

If you re-land or build on someone else's work (a fork PR, an issue author's
proposal, a co-author), **credit them with a working GitHub-linked trailer** so
they appear in the contributors graph and on their profile:

```
Co-authored-by: Full Name <ID+username@users.noreply.github.com>
```

- Use the **numeric-id no-reply form** (`ID+username@users.noreply.github.com`).
  Get the id with `gh api users/<login> --jq .id`.
- **Never** use a machine/`.local` email (e.g. `name@Someones-Mac.local`) in a
  co-author trailer — it links to no account and gives **zero** credit.
- When a squash-merge collapses a contributor's PR into an internal branch,
  preserve their `Co-authored-by:` line in the squash commit message, and record
  substantial contributions in `CONTRIBUTORS.md` if the repo has one.

## Secrets & safety

- Never commit secrets, tokens, or private emails. Use `*.noreply.github.com`
  for attribution.
- Don't weaken CI gates or branch protection to land a change. If it can't go
  through a green PR, surface the blocker instead of working around it.

## Claude Code

`CLAUDE.md` points here — this file is the source of truth for both. Anything
Claude-specific is noted there.
