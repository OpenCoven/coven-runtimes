# coven-runtimes

**The runtime SDK, conformance toolkit, and registry for the Coven.**

A *runtime* is an agent CLI the Coven drives to do work — today those are Codex,
Claude Code, and Hermes. `coven-runtimes` is how you add a new one **without
editing `coven` core**: declare what the runtime can do in a validated manifest,
conformance-test it against the real binary, and get it accepted into the
**canonical registry** that every downstream repo adopts.

This repo is the single source of truth for the *accepted* runtimes. A runtime
is accepted when its manifest is merged into [`registry/runtimes/`](registry);
that compiles into a checksummed index that Rust repos consume embedded
(`RegistryIndex::canonical()`) and any-language repos consume as a release asset.
See [`docs/registry.md`](docs/registry.md) (how it's maintained),
[`docs/adoption.md`](docs/adoption.md) (how to consume it), and
[`GOVERNANCE.md`](GOVERNANCE.md) (the acceptance bar).

> Status: v0.1 — the spec, CLI, and registry are implemented and tested. The
> `coven` core integration (reading `capabilities` instead of hardcoded
> `harness_id == "claude"` checks) is a coordinated follow-up PR; see
> [`docs/integration.md`](docs/integration.md).

---

## Why this exists

Coven decides a runtime's behavior — stream mode, session pre-assignment,
think/speed toggles, sandbox mapping — with hardcoded `harness_id == "claude"`
string checks in `coven`'s `harness.rs`. External adapters (`*.json` under
`$COVEN_HOME/adapters`) can declare *args* but **not behaviors** or **sandbox
mapping** (coven forces `sandbox: None` for every manifest). So teaching Coven
about a new streaming runtime means editing core Rust.

This repo closes that gap. A runtime *declares* its capabilities; core *reads*
them.

## Workspace layout

| Crate | What it is |
|-------|------------|
| **`coven-runtime-spec`** | The manifest schema, capability model, sandbox mapping, and validation. Pure types + rules, no I/O. This is the crate `coven` core depends on to replace the hardcoded string checks. |
| **`coven-runtime-cli`** (`conjure`) | The authoring toolkit: `new`, `validate`, `test` (conformance probe), `package`. |
| **`coven-runtime-registry`** | A versioned index format + resolver, and the **canonical accepted list** embedded via `RegistryIndex::canonical()`. |

Plus [`registry/`](registry) (the canonical source manifests — the approval
surface), [`schema/`](schema) (JSON Schema for editors/CI),
[`examples/`](examples) (dogfooded reference manifests), and
[`docs/`](docs) (the conformance spec, registry, adoption, and integration guides).

## Quickstart

```sh
# Build the CLI
cargo build --release           # binary at target/release/conjure

# Scaffold a new runtime adapter
conjure new aria                        # minimal one-shot runtime → aria.json
conjure new zephyr --flavor streaming   # streaming + sandbox → zephyr.json

# Validate against the shared spec
conjure validate aria.json --verbose

# Validate a registry index (every entry + id/key match)
conjure validate --registry registry-index.json

# Conformance-test against the real binary (probes PATH + a --version/--help call)
conjure test aria.json
conjure test aria.json --skip-binary    # static rules only (CI without the runtime)

# Package for publishing (canonical JSON + SHA-256)
conjure package aria.json

# Accept a runtime into the canonical registry, then (re)compile the index
conjure registry add aria.json         # copies into registry/runtimes/ + rebuilds
conjure registry build                 # recompile the canonical index
conjure registry check                 # CI drift guard: committed index == sources
conjure registry list                  # the accepted runtimes + capabilities
```

## The manifest, in one glance

A manifest is a backward-compatible **superset** of coven's existing adapter
JSON — every field coven reads today is unchanged; the additions are
`capabilities`, `sandbox`, and `stream_args`.

```jsonc
{
  "adapters": [{
    "id": "aria",
    "label": "Aria",
    "executable": "aria",
    "non_interactive_prompt_prefix_args": ["exec"],
    "install_hint": "Install aria and add it to PATH.",
    "model_flag": "--model",

    // ── additions that make integration seamless ──
    "capabilities": { "stream": true, "preassigned_session_id": true, "think": true, "speed": true },
    "sandbox": { "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" },
    "stream_args": {
      "prefix_args": ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
      "session_id_flag": "--session-id",
      "resume_flag": "--resume"
    }
  }]
}
```

Field names are snake_case-canonical with camelCase aliases, so both
`prefix_args` and `prefixArgs` parse. See
[`docs/conformance.md`](docs/conformance.md) for the full field reference and
the validation rules.

## Adopting the accepted runtimes downstream

Other repos don't hand-copy adapters — they adopt the canonical accepted set.
Both paths resolve to the exact same bytes:

```rust
// Rust (e.g. coven core): pinned by the crate version.
use coven_runtime_registry::RegistryIndex;
let entry = RegistryIndex::canonical().resolve_latest("hermes")?;
```

```sh
# Any language: pinned by release tag + checksum.
curl -fsSLO "https://github.com/OpenCoven/coven-runtimes/releases/download/v0.1.0/registry-index.json"
```

Full recipes (pinning, checksum verification, resolution) are in
[`docs/adoption.md`](docs/adoption.md).

## Contributing

Two kinds of contribution, two bars:

- **Adding a runtime** — scaffold a manifest with `conjure new`, validate it,
  then `conjure registry add` it and open a PR. Merging it under
  [`registry/runtimes/`](registry) is what makes it *accepted*
  (see [`GOVERNANCE.md`](GOVERNANCE.md)). This is the common case and the tooling
  is built for it.
- **Changing the SDK crates** — must pass `fmt`, `clippy -D warnings`,
  `test --locked`, and `cargo deny`.

See [`CONTRIBUTING.md`](CONTRIBUTING.md) for the full checklist, and open a
[New runtime adapter](.github/ISSUE_TEMPLATE/new-runtime.yml) issue if you want
to discuss a runtime before authoring it.

## Development

```sh
cargo test --workspace     # unit + doc tests
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT © Valentina Alexander
