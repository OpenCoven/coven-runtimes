# Contributing to coven-runtimes

Thanks for helping grow the Coven's runtime ecosystem. This repo is the SDK,
conformance toolkit, and registry for integrating a new **runtime** (an agent
CLI the Coven drives) — without editing `coven` core.

There are two kinds of contribution, and they have different bars:

1. **Adding / updating a runtime adapter** — a manifest describing a runtime.
2. **Changing the SDK itself** — the `coven-runtime-*` crates.

---

## 1. Adding a runtime adapter

This is the common case, and the tooling is built for it. For the narrative
start-to-finish walkthrough see [`docs/authoring.md`](docs/authoring.md); when
something fails, [`docs/troubleshooting.md`](docs/troubleshooting.md) is
symptom-indexed. The condensed version:

```sh
# Scaffold a manifest (minimal one-shot, or streaming + sandbox)
conjure new my-runtime --flavor minimal
conjure new my-runtime --flavor streaming

# Validate it against the shared spec
conjure validate my-runtime.json --verbose

# Conformance-test against the real binary on your PATH
conjure test my-runtime.json
conjure test my-runtime.json --skip-binary   # static rules only (no binary installed)

# Accept it into the canonical registry: copies to registry/runtimes/<id>/<version>.json
# and recompiles the committed index. (Requires a `version` in the manifest.)
conjure registry add my-runtime.json
```

Getting the manifest **merged under `registry/runtimes/`** is what makes the
runtime *accepted*. The acceptance bar lives in [`GOVERNANCE.md`](GOVERNANCE.md);
how the list is stored and rebuilt is in [`docs/registry.md`](docs/registry.md).

**Requirements for a runtime-adapter PR:**

- `conjure validate <manifest> --verbose` passes with **zero** problems.
- Every declared capability is real:
  - `capabilities.stream` requires a `stream_args` block with non-empty
    `prefix_args`.
  - `capabilities.preassigned_session_id` requires `stream_args.session_id_flag`.
  - a `sandbox` block requires a non-empty `flag`, `full`, and `read_only`.
- The `id` is lowercase `[a-z0-9._-]+` and does not collide with a built-in
  (`codex`, `claude`).
- `install_hint` tells a user exactly how to get the binary.
- The source is at `registry/runtimes/<id>/<version>.json` (one adapter,
  `version` = filename stem) and you ran `conjure registry build` so the
  committed index is in sync — CI's `registry check` drift guard enforces this.
- Released versions are immutable: to change a runtime, add a **new** version
  file rather than editing a published one.

See [`docs/conformance.md`](docs/conformance.md) for the full field reference and
every validation rule, and [`examples/`](examples) for dogfooded manifests
(`hermes.json`, `claude.json`, `copilot.json`).

**Do not** declare a capability the runtime can't actually honor. The point of
the manifest is that `coven` trusts it; a false `stream: true` will hang a real
session.

---

## 2. Changing the SDK crates

The workspace is three crates:

| Crate | Responsibility |
|-------|----------------|
| `coven-runtime-spec` | Manifest schema, capability model, sandbox mapping, pure validation. No I/O. The crate `coven` core depends on. |
| `coven-runtime-cli` (`conjure`) | Authoring toolkit: `new`, `validate`, `test`, `package`, and `registry` (build/check/add/list/yank). |
| `coven-runtime-registry` | Index format + semver resolver, and the embedded canonical accepted list (`RegistryIndex::canonical()`). |

**Before you open a PR, every one of these must pass** (it's exactly what CI runs):

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
cargo deny check advisories licenses bans sources   # if cargo-deny is installed
```

**Ground rules:**

- **`coven-runtime-spec` stays pure** — no async, no network, no process
  spawning, no filesystem. It's a dependency of `coven` core; keep it light.
- **Keep the manifest a backward-compatible superset** of coven's
  `ExternalHarnessAdapterSpec`. Adding fields is fine; renaming or removing ones
  coven reads is not. New fields need snake_case + camelCase serde aliases and a
  matching entry in [`schema/adapter-manifest.schema.json`](schema/adapter-manifest.schema.json)
  (both alias spellings).
- **If you touch the manifest shape, update the JSON Schema in the same PR.** The
  `schema_examples` test asserts the schema accepts the examples and everything
  `RuntimeAdapter` serializes — it will fail if they drift.
- **Watch dependency weight.** Prefer `default-features = false` and dev-deps for
  test-only tooling (see how `jsonschema` is pulled in). New runtime deps go
  through `cargo deny` on every CI run.
- **Bump `SCHEMA_VERSION`** in `coven-runtime-spec` on any backward-incompatible
  manifest change.

---

## Commits & PRs

- Small, focused PRs. One concern each.
- Conventional-ish commit subjects (`feat:`, `fix:`, `docs:`, `chore:`).
- Fill in the PR template; the checklist is the merge bar.
- CI must be green. No exceptions for `-D warnings`.

## The bigger picture

This SDK only becomes load-bearing once `coven` core reads `capabilities`
instead of hardcoded `harness_id == "claude"` checks. That's a coordinated
follow-up; the plan lives in [`docs/integration.md`](docs/integration.md). If
your change assumes core already consumes a field, check that doc first — it
probably doesn't yet.

## License

By contributing, you agree your contributions are licensed under the repo's
[MIT License](LICENSE).
