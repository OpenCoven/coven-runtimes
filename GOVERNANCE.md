# Governance: accepting a runtime

This repo maintains the canonical list of runtimes the Coven ecosystem has
**accepted**. This document defines what "accepted" means, the bar a runtime
must clear, and how the list changes over time.

- How the list is stored and rebuilt → [`docs/registry.md`](docs/registry.md)
- How downstream repos consume the list → [`docs/adoption.md`](docs/adoption.md)

## What "accepted" means

**A runtime is accepted when its manifest is merged into `registry/runtimes/`.**
There is no separate status field or promotion step — the merge *is* the
approval. This keeps the model git-native and auditable: the history of the
canonical list is the commit history of that directory.

The approval gate is enforced by two mechanisms:

1. **`.github/CODEOWNERS`** requires a maintainer review on `registry/**` (and
   the compiled index) before a change to the accepted set can merge.
2. **Branch protection** on `main` with *Require review from Code Owners* and
   *Require status checks* (CI, including the drift-guard) enabled.

> Repo admins: both must be configured for the gate to hold. CODEOWNERS ships
> with a placeholder team — replace it with the real maintainer handle.

## Acceptance criteria

A runtime-adapter PR is accepted only when all of these hold (they are also the
[PR checklist](.github/PULL_REQUEST_TEMPLATE.md)):

- **Valid.** `conjure validate <manifest> --verbose` passes with zero problems.
- **Honest capabilities.** Every declared capability is one the runtime actually
  honors — `stream` has working `stream_args`, `preassigned_session_id` has a
  `session_id_flag`, `sandbox` maps a real permission flag. A false capability
  will break live sessions, so this is non-negotiable.
- **Conformance-checked.** `conjure test <manifest>` was run against the real
  binary (or the PR notes why it was skipped).
- **Well-formed id.** Lowercase `[a-z0-9._-]+`, not colliding with a built-in
  (`codex`, `claude`).
- **Clear install path.** `install_hint` tells a user exactly how to obtain the
  binary.
- **Placed and compiled correctly.** The source lives at
  `registry/runtimes/<id>/<version>.json` (one adapter, `version` = filename),
  and the committed index was regenerated with `conjure registry build` — CI's
  drift guard enforces this.

## Changing an accepted runtime

- **New version:** add a new `registry/runtimes/<id>/<newversion>.json` and
  rebuild. Released versions are immutable — the generator rejects an in-place
  content change to an existing version.
- **Deprecate a version:** `conjure registry yank <id> <version>`. It stays
  resolvable by exact pin but drops out of "latest".
- **Retire a runtime:** yank all its versions; optionally remove its source
  directory and rebuild.

## Releasing the accepted set

Tagging `v*` runs the [release workflow](.github/workflows/release.yml): it
verifies the index is in sync, then publishes `registry-index.json` +
`.sha256` as release assets. The same bytes are what
`coven-runtime-registry` embeds, so the crate and the asset always agree.
Downstream repos adopt newly accepted runtimes by bumping their pin.

## Roles

- **Maintainers** (CODEOWNERS) review and merge changes to the accepted set,
  cut releases, and steward the spec/schema.
- **Contributors** propose runtimes via PR (preferred) or a
  [New runtime adapter](.github/ISSUE_TEMPLATE/new-runtime.yml) issue for
  discussion first.
