# Adopting accepted runtimes in a downstream repo

Once a runtime is [accepted](../GOVERNANCE.md) into the
[canonical registry](registry.md), any repo can adopt it without hand-copying
`*.json` files. The accepted list is published two ways from the same bytes, so
Rust and non-Rust consumers never disagree:

1. **Embedded in the `coven-runtime-registry` crate** — for Rust consumers.
2. **A checksummed release asset** — for any language.

Pick whichever matches your stack. Both resolve to the exact same index.

---

## Rust consumers (e.g. `coven` core)

Depend on the registry crate and read the embedded canonical index. You are
pinned to the accepted set as of that crate version — bumping the dependency is
how you adopt newly accepted runtimes.

```toml
# Cargo.toml
[dependencies]
coven-runtime-registry = "0.1"   # (or a git/path dep until published)
```

```rust
use coven_runtime_registry::RegistryIndex;

let registry = RegistryIndex::canonical();          // embedded, infallible
let entry = registry.resolve_latest("hermes")?;     // newest non-yanked version
let adapter = &entry.adapter;                        // a validated RuntimeAdapter

// Or pin an exact version (yanked versions still resolve by exact pin):
let pinned = registry.resolve_exact("hermes", "1.0.0")?;
```

`resolve_latest` / `resolve_exact` return a `RegistryEntry` whose `adapter` is a
`coven_runtime_spec::RuntimeAdapter` — the same type coven core will read
capabilities and sandbox mapping from (see [`integration.md`](integration.md)).

Because the index is embedded via `include_str!`, there is no network fetch and
no parse failure at runtime.

---

## Any-language consumers (release asset)

Each tagged release attaches:

- `registry-index.json` — the canonical index (a `RegistryIndex` JSON document).
- `registry-index.json.sha256` — its SHA-256, for integrity.

Pin to a **release tag** and verify the checksum before trusting the file:

```sh
TAG=v0.1.0
base="https://github.com/OpenCoven/coven-runtimes/releases/download/$TAG"
curl -fsSLO "$base/registry-index.json"
curl -fsSLO "$base/registry-index.json.sha256"
echo "$(cat registry-index.json.sha256)  registry-index.json" | sha256sum -c -
# macOS has no sha256sum; use:  … | shasum -a 256 -c -
```

The document shape (stable within an `INDEX_FORMAT`):

```jsonc
{
  "format": "1",
  "runtimes": {
    "<id>": [
      {
        "version": "1.0.0",
        "adapter": { /* the full RuntimeAdapter manifest */ },
        "sha256": "…",             // digest of the adapter's canonical bytes
        "published_at": "…Z",
        "yanked": false            // omit-if-false; skip these for "latest"
      }
    ]
  }
}
```

To resolve "latest", pick the highest semver entry with `yanked` not set. To
verify an individual adapter you pulled, its `sha256` is the digest
`conjure package <manifest> --check-only` prints for the same manifest.

---

## Pinning & upgrade policy

- **Always pin** — a crate version (Rust) or a release tag + checksum (asset).
  Don't track a moving branch.
- **Upgrade deliberately.** Bumping the pin adopts every runtime accepted since
  your last pin. Review the diff of the index (or the release notes) first.
- **Yanks are advisory to you.** A yanked version stays resolvable by exact pin;
  it's just excluded from "latest". If you pinned a version that later gets
  yanked, you keep working — but treat it as a signal to move off.

---

## Push notifications for downstream sync

Merges to `main` that touch `registry/**` (or the canonical index) fire a
`repository_dispatch` (`runtimes-registry-updated`) at downstream repos via
[`notify-downstream.yml`](../.github/workflows/notify-downstream.yml), so an
adopting repo can regenerate its committed registry module and open a reviewed
PR without polling. `coven-cave` listens with its `Sync runtimes registry`
workflow. To add another downstream, extend the notify workflow with a second
dispatch step and give this repo a token that can reach the target repo
(secret per downstream, e.g. `CAVE_DISPATCH_TOKEN`).
