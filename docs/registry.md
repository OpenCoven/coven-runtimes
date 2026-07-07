# The canonical runtime registry

This repo is the **single source of truth** for the runtimes the Coven ecosystem
has *accepted*. Downstream repos don't hand-copy `*.json` adapters; they adopt
the accepted set from here. This document describes how that list is maintained.

For how to *consume* the list from another repo, see
[`adoption.md`](adoption.md). For the acceptance bar and who approves, see
[`../GOVERNANCE.md`](../GOVERNANCE.md).

## Layout

```
registry/runtimes/<id>/<version>.json            ← source manifests (the approval surface)
crates/coven-runtime-registry/canonical/index.json  ← compiled index (generated, committed)
```

- **Source manifests** under `registry/runtimes/` are human-authored, one
  [adapter manifest](conformance.md) per file, one file per published version
  (e.g. `registry/runtimes/hermes/1.0.0.json`). A file must contain exactly one
  adapter whose `id` equals its directory and whose `version` equals its
  filename stem. **These files are the approval surface: a runtime is accepted
  when its manifest is merged here.**
- **The compiled index** is a [`RegistryIndex`](../crates/coven-runtime-registry/src/lib.rs)
  generated from the sources. It lives inside the registry crate so it is both
  embeddable (`include_str!`) and shipped by `cargo publish`, with no second
  copy to drift. It is committed so consumers can fetch it directly and so CI can
  guard it.

## Maintaining it with `conjure registry`

```sh
# Recompile the index from the source manifests (run after any manifest change).
conjure registry build

# Verify the committed index matches the sources — non-zero exit on drift.
# This is what CI runs; run it locally before pushing.
conjure registry build --check          # or: conjure registry check

# Accept a manifest: validate it, copy it under registry/runtimes/, and rebuild.
conjure registry add path/to/my-runtime.json

# Show the accepted runtimes and their latest versions + capabilities.
conjure registry list

# Yank a published version (excluded from "latest" but still resolvable by exact
# pin), or restore it with --unyank. Persists across rebuilds.
conjure registry yank hermes 1.0.0
conjure registry yank hermes 1.0.0 --unyank
```

`build` stamps each entry with the manifest's SHA-256 (the same digest
`conjure package` prints, byte-for-byte) and an ISO-8601 `published_at`.

## Guarantees the generator enforces

- **Deterministic & idempotent.** Rebuilding without a source change produces
  identical bytes: `published_at` is preserved for entries that already exist in
  the committed index. Only a genuinely new `(id, version)` gets a fresh
  timestamp.
- **Version immutability.** Editing a released `(id, version)`'s content without
  bumping the version is a hard error — bump the version instead. This protects
  anyone who pinned that version. (Same rule as crates.io.)
- **Drift guard.** The `committed_index_matches_sources` test (and the
  `registry check` CI step) fail if a source manifest changed without a rebuild,
  so the committed index can never silently lag its sources.
- **Full validation.** Every entry passes the shared spec rules and each entry's
  `adapter.id` matches its runtime key before the index is written.

## Versioning & deprecation

- Versions are semver `major.minor.patch`. A runtime can have several versions
  in the index; `resolve_latest` returns the highest non-yanked one.
- To ship a change, add a new `registry/runtimes/<id>/<newversion>.json` and
  rebuild — don't edit a released file.
- To deprecate a version, `yank` it. To retire a runtime entirely, yank all its
  versions (and, if desired, remove its source directory and rebuild).
- The index format itself is versioned by `INDEX_FORMAT`; the manifest schema by
  `SCHEMA_VERSION`. Bump those only on incompatible changes.
