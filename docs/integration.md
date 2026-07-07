# Integrating `coven-runtimes` into `coven` core

This repo ships the runtime SDK (spec + CLI + registry). It becomes *load-bearing*
only when `coven` core reads the manifest's `capabilities` block instead of
deciding runtime behavior with hardcoded `harness_id == "claude"` checks. That
core change is a **coordinated follow-up PR**, described here so it can land
cleanly after this repo is published.

> **Status:** planned. Nothing in `coven` core imports this crate yet. This
> document is the integration contract, not a record of completed work.

Where the manifests *come from* once core reads them is the
[canonical registry](registry.md): core resolves accepted runtimes from the
embedded `RegistryIndex::canonical()` (see [`adoption.md`](adoption.md)) rather
than scanning loose `*.json` files. This doc covers the *reading* side — the
`harness.rs` seam — which is orthogonal to how the list is maintained.

## The seam today

In `coven`'s `crates/coven-cli/src/harness.rs`:

- `HarnessCommandSpec` describes a runtime's *args* (prefixes, model flag,
  system-prompt flag, sandbox).
- Behavioral decisions are **string checks**, not data:
  - `harness_supports_stream_mode(id)` → `id == "claude"`
  - `harness_supports_preassigned_session_id(id)` → `id == "claude"`
  - `harness_supports_think(id)` / `harness_supports_speed(id)` → same shape.
- `ExternalHarnessAdapterSpec::into_spec` hardcodes `sandbox: None`, so a
  manifest-defined runtime can never map `coven run --permission`.

Net effect: a new streaming runtime cannot stream, pre-assign a session id, or
map a sandbox without editing core Rust.

## The target state

1. `coven` core takes a dependency on `coven-runtime-spec`.
2. `HarnessCommandSpec` gains a `capabilities: Capabilities` field (and, for
   manifest adapters, an optional `sandbox` + `stream_args`), populated from the
   manifest via `coven-runtime-spec` types.
3. The four `harness_supports_*` predicates become field reads:

   ```rust
   // before
   fn harness_supports_stream_mode(id: &str) -> bool { id == "claude" }
   // after
   spec.capabilities.stream
   ```

4. Built-in harnesses (Codex, Claude) declare their capabilities in the same
   struct the manifests use, so built-ins and external adapters travel the same
   code path. Claude's built-in spec sets
   `capabilities { stream, preassigned_session_id, think, speed }` all true with
   the matching `stream_args`; Codex stays baseline.
5. `into_spec` maps the manifest's `sandbox` through instead of forcing `None`.

Because the manifest is a **backward-compatible superset** of coven's current
adapter JSON (same field names, same camelCase aliases), existing `*.json`
adapters — including the bundled `hermes` recipe — deserialize unchanged and
simply pick up the conservative baseline (all capabilities off).

## Suggested PR sequence

1. **coven-runtimes** (this repo): publish v0.1. No core impact.
2. **coven core, PR 1 — adopt the types (no behavior change):** add the
   `coven-runtime-spec` dependency, add `capabilities` to `HarnessCommandSpec`,
   populate built-ins with their real capabilities, and rewrite the
   `harness_supports_*` predicates as field reads. Existing tests must stay
   green — this is a pure refactor from string checks to data.
3. **coven core, PR 2 — extend the manifest loader:** parse `capabilities`,
   `sandbox`, and `stream_args` from external adapter manifests and honor them
   (sandbox mapping, stream launch). Add a test that a manifest-declared
   streaming runtime is driven in stream mode.

Keeping the type adoption (PR 1) separate from loader behavior (PR 2) keeps each
diff reviewable and the refactor bisectable.

## Verification checklist for the core PRs

- `hermes.json` and any other existing adapters still load and behave identically.
- Claude's built-in behavior is byte-for-byte unchanged (stream, session id,
  think, speed) after moving from string checks to declared capabilities.
- A new manifest with `capabilities.stream = true` + `stream_args` is launched
  in stream mode without any core edit — the acceptance test for the whole effort.
- `coven run --permission read-only` maps through a manifest's `sandbox` block.
