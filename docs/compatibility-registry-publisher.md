# Compatibility registry publisher

`coven-runtimes` publishes the public, signed compatibility contracts used by
Coven Cave’s OpenCode, Grok, and OpenClaw integrations. The publisher owns the
private keys; Cave packages only public endpoints, public keys or rotation
keyrings, and immutable sequence/hash checkpoints.

## Public endpoints

After GitHub Pages is configured to deploy from Actions, Cave fetches these
direct HTTPS documents (no redirects):

- `https://opencoven.github.io/coven-runtimes/opencode/current.json`
- `https://opencoven.github.io/coven-runtimes/grok/current.json`
- `https://opencoven.github.io/coven-runtimes/openclaw/current.json`

Every deployed source sequence is also preserved at
`/<runtime>/<sequence>.json`. `current.json` is byte-identical to the latest
sequence file. A signed update must use a strictly higher sequence; never
rewrite an existing sequence file. OpenClaw follows the same immutable
sequence contract as OpenCode and Grok.

## Key custody and deployment

Create the protected GitHub environment `compatibility-registry-publisher` in
this repository. Store exactly these environment secrets there:

- `COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM`
- `COMPATIBILITY_GROK_PRIVATE_KEY_PEM`
- `COMPATIBILITY_OPENCLAW_PRIVATE_KEY_PEM`

Each value is a distinct PEM-encoded Ed25519 private key. Generate and retain
the keys in the organization-controlled secret manager before copying them to
the protected environment. Do not commit them, place them in Coven Cave, add
them to Cave Actions secrets, or print them in workflow logs. Private signing
keys must never enter Cave, logs, issues, PR bodies, or published output.

Enable GitHub Pages with the **GitHub Actions** source, then manually dispatch
`Publish compatibility registry` only for a reviewed `main` commit. The
workflow verifies source contracts, hydrates the published history, signs only
new sequences, uploads only `site/`, and deploys through the Pages environment.
Before publishing an OpenClaw source contract, review it against Cave’s built-in
OpenClaw payload so the publisher’s genesis or successor sequence matches the
bounded Cave contract you intend to ship.

### Historical hydration and key rotation

Every publisher run starts with an empty staging tree and fetches the canonical
HTTPS `checkpoints.json` from GitHub Pages with a fresh unguessable query value,
`no-store` fetch semantics, and explicit no-cache request headers, without
following a redirect. Immutable sequence URLs remain stable and cacheable. The
hydrator validates the runtime allowlist and checkpoint fields, then fetches
every immutable sequence from one through each checkpoint. Missing sequence
files, redirects, network or TLS failures, malformed checkpoints, unknown
runtimes, and non-success responses all fail closed before signing. A 404 from
the cache-busted canonical `checkpoints.json` request also fails closed by
default.

This repository already has published OpenCode and Grok history, so the normal
publisher workflow must never authorize bootstrap. A partial production
checkpoint is valid: the existing OpenCode and Grok histories are hydrated
while OpenClaw publishes its genesis. Never use bootstrap authorization for recovery
from an outage, Pages misconfiguration, missing history, or a failed
publication.

Only a truly new Pages deployment with no published registry history may use
the one-time CLI authorization:

```sh
node scripts/hydrate-compatibility-registry-site.mjs --allow-bootstrap-404
```

The flag authorizes only an actual 404 from the canonical `checkpoints.json`
request. Redirects, network failures, malformed responses, and every other
status remain fatal. Remove the flag immediately after the first publication;
all later runs must use the normal bootstrap-disabled command.

The builder validates every hydrated file as a signed Ed25519 bundle and binds
each source directory to its payload runtime (`opencode`, `grok-build`, or
`openclaw`). The hydrated unsigned canonical payload must match the reviewed
source, and the latest payload hash must match its checkpoint. Existing
sequence files are then preserved byte-for-byte; `current.json` copies the
latest immutable file exactly, and checkpoints are regenerated from the
preserved and newly signed bundles.

This preservation rule is essential during a key rotation: replacing the
current private key must never re-sign historical sequences or change their
bytes. Add a higher reviewed source sequence for the new key. The publisher
signs only that new sequence and carries all earlier signatures forward
unchanged. Public-key verification is intentionally left to Cave and the
release guards; publisher hydration trusts the canonical bytes only after the
reviewed unsigned payload and checkpoint hash checks succeed.

## Cave release handoff

For each runtime, export its PEM public key and read `site/checkpoints.json`.
Set only these public Cave repository secrets:

| Runtime | Endpoint secret | Public-key secret | Checkpoint secret |
| --- | --- | --- | --- |
| OpenCode | `OPENCODE_SCHEMA_REGISTRY_URL` | `OPENCODE_SCHEMA_REGISTRY_PUBLIC_KEY` | `OPENCODE_SCHEMA_REGISTRY_CHECKPOINT` |
| Grok | `GROK_SCHEMA_REGISTRY_URL` | `GROK_SCHEMA_REGISTRY_PUBLIC_KEY` | `GROK_SCHEMA_REGISTRY_CHECKPOINT` |
| OpenClaw | `OPENCLAW_SCHEMA_REGISTRY_URL` | `OPENCLAW_SCHEMA_REGISTRY_PUBLIC_KEY` | `OPENCLAW_SCHEMA_REGISTRY_CHECKPOINT` |

The checkpoint is exactly `{ "sequence": <number>, "payloadHash":
"<lowercase sha256>" }`. Sequence one is the pinned Cave baseline and has no
`keyId`. During a later key rotation, add the selected `keyId` to the reviewed
source for the higher sequence and use the corresponding `*_PUBLIC_KEYS`
secret with one to four named keys. Retain the previous public key until all
supported Cave releases have moved beyond it. OpenClaw may therefore hand off
either `OPENCLAW_SCHEMA_REGISTRY_PUBLIC_KEY` or the rotation keyring
`OPENCLAW_SCHEMA_REGISTRY_PUBLIC_KEYS`, depending on whether the release is on
a single active key or an overlap window.

Before a Cave release, fetch all three public endpoints without following
redirects, verify their signatures with the corresponding public keys or
keyrings, compare their payload hashes and sequences to the checkpoints, and
run all three Cave release guard scripts:
`check-opencode-registry-release.mjs`,
`check-grok-registry-release.mjs`, and
`check-openclaw-registry-release.mjs`. A missing key, non-HTTPS URL, redirect,
invalid signature, expired bundle, or checkpoint mismatch is a release
failure.
