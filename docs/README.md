# Documentation

Start from what you're trying to do:

| I want to… | Read |
|------------|------|
| **Add a new runtime** to the Coven, start to finish | [`authoring.md`](authoring.md) — the end-to-end tutorial |
| Look up a **manifest field or validation rule** | [`conformance.md`](conformance.md) — the authoritative reference |
| **Fix an error** from `conjure` or CI | [`troubleshooting.md`](troubleshooting.md) — symptom-indexed |
| **Consume the accepted runtimes** from another repo | [`adoption.md`](adoption.md) — Rust crate & release-asset recipes |
| Understand **how the canonical registry is maintained** | [`registry.md`](registry.md) — layout, rebuilds, immutability, yanks |
| Follow the **`coven` core integration** plan | [`integration.md`](integration.md) — the `harness.rs` seam |

Around the docs:

- [`../README.md`](../README.md) — what this repo is, quickstart, workspace layout.
- [`../CONTRIBUTING.md`](../CONTRIBUTING.md) — the contribution bar for both
  adapter PRs and SDK PRs.
- [`../GOVERNANCE.md`](../GOVERNANCE.md) — who accepts runtimes, and against
  what bar.
- [`../AGENTS.md`](../AGENTS.md) — the workflow contract for AI agents opening
  PRs here.

**The 60-second orientation:** a *runtime* is an agent CLI the Coven drives. A
*manifest* declares what that CLI can do (streaming, session ids, sandbox
mapping) so `coven` core can read capabilities instead of hardcoding them. The
`conjure` CLI scaffolds, validates, conformance-tests, and packages manifests.
A runtime is *accepted* when its manifest merges into
[`registry/runtimes/`](../registry), which compiles into a checksummed index
that downstream repos consume. `authoring.md` walks that whole journey.
