# Compatibility registry publisher

`coven-runtimes` publishes the public, signed compatibility contracts used by
Coven Cave’s OpenCode and Grok integrations. The publisher owns the private
keys; Cave packages only public endpoints, public keys, and immutable
sequence/hash checkpoints.

## Public endpoints

After GitHub Pages is configured to deploy from Actions, Cave fetches these
direct HTTPS documents (no redirects):

- `https://opencoven.github.io/coven-runtimes/opencode/current.json`
- `https://opencoven.github.io/coven-runtimes/grok/current.json`

Every deployed source sequence is also preserved at
`/<runtime>/<sequence>.json`. `current.json` is byte-identical to the latest
sequence file. A signed update must use a strictly higher sequence; never
rewrite an existing sequence file.

## Key custody and deployment

Create the protected GitHub environment `compatibility-registry-publisher` in
this repository. Store exactly these environment secrets there:

- `COMPATIBILITY_OPENCODE_PRIVATE_KEY_PEM`
- `COMPATIBILITY_GROK_PRIVATE_KEY_PEM`

Each value is a distinct PEM-encoded Ed25519 private key. Generate and retain
the keys in the organization-controlled secret manager before copying them to
the protected environment. Do not commit them, place them in Coven Cave, add
them to Cave Actions secrets, or print them in workflow logs.

Enable GitHub Pages with the **GitHub Actions** source, then manually dispatch
`Publish compatibility registry` only for a reviewed `main` commit. The
workflow verifies source contracts before it signs them, uploads only `site/`,
and deploys through the Pages environment.

## Cave release handoff

For each runtime, export its PEM public key and read `site/checkpoints.json`.
Set only these public Cave repository secrets:

| Runtime | Endpoint secret | Public-key secret | Checkpoint secret |
| --- | --- | --- | --- |
| OpenCode | `OPENCODE_SCHEMA_REGISTRY_URL` | `OPENCODE_SCHEMA_REGISTRY_PUBLIC_KEY` | `OPENCODE_SCHEMA_REGISTRY_CHECKPOINT` |
| Grok | `GROK_SCHEMA_REGISTRY_URL` | `GROK_SCHEMA_REGISTRY_PUBLIC_KEY` | `GROK_SCHEMA_REGISTRY_CHECKPOINT` |

The checkpoint is exactly `{ "sequence": <number>, "payloadHash":
"<lowercase sha256>" }`. Sequence one is the pinned Cave baseline and has no
`keyId`. During a later key rotation, add the selected `keyId` to the reviewed
source for the higher sequence and use the corresponding `*_PUBLIC_KEYS`
secret with one to four named keys. Retain the previous public key until all
supported Cave releases have moved beyond it.

Before a Cave release, fetch both public endpoints without following redirects,
verify their signatures with the public keys, compare their payload hashes to
the checkpoints, and run Cave’s two release guard scripts. A missing key,
non-HTTPS URL, redirect, invalid signature, expired bundle, or checkpoint
mismatch is a release failure.
