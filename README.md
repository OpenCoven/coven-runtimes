# coven-runtimes

**The runtime SDK, conformance toolkit, and registry for the Coven.**

A *runtime* is an agent CLI the Coven drives to do work — today those are Codex,
Claude Code, and Hermes. `coven-runtimes` is how you add a new one **without
editing `coven` core**: declare what the runtime can do in a validated manifest,
conformance-test it against the real binary, and publish it to a registry.

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
| **`coven-runtime-cli`** (`covenrt`) | The authoring toolkit: `new`, `validate`, `test` (conformance probe), `package`. |
| **`coven-runtime-registry`** | A versioned index format + resolver for distributing adapters (`coven adapter install <name>`). |

Plus [`schema/`](schema) (JSON Schema for editors/CI),
[`examples/`](examples) (dogfooded reference manifests), and
[`docs/`](docs) (the conformance spec + integration guide).

## Quickstart

```sh
# Build the CLI
cargo build --release           # binary at target/release/covenrt

# Scaffold a new runtime adapter
covenrt new aria                        # minimal one-shot runtime → aria.json
covenrt new zephyr --flavor streaming   # streaming + sandbox → zephyr.json

# Validate against the shared spec
covenrt validate aria.json --verbose

# Validate a registry index (every entry + id/key match)
covenrt validate --registry registry-index.json

# Conformance-test against the real binary (probes PATH + a --version/--help call)
covenrt test aria.json
covenrt test aria.json --skip-binary    # static rules only (CI without the runtime)

# Package for publishing (canonical JSON + SHA-256)
covenrt package aria.json
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

## Development

```sh
cargo test --workspace     # unit + doc tests
cargo fmt --all
cargo clippy --workspace --all-targets -- -D warnings
```

## License

MIT © Valentina Alexander
