# Troubleshooting

Symptom-indexed fixes for the errors you'll actually hit, in the order you'll
hit them: authoring → validation → conformance → registry → CI. Commands and
outputs below are real, not paraphrased.

## Parsing errors (before validation even runs)

**`failed to parse manifest … missing field 'install_hint'`**

The manifest didn't deserialize — a required field (`id`, `label`,
`executable`, `install_hint`) is missing, or a field name is misspelled. Note
the parser is strict: unknown fields are rejected so typos can't silently
become dead config. Both `snake_case` and `camelCase` spellings parse
(`prefix_args` / `prefixArgs`); anything else is unknown.

**`unknown field 'capabilties'`** (or similar)

A typo. Check the spelling against [`conformance.md`](conformance.md) or let
your editor validate against
[`schema/adapter-manifest.schema.json`](../schema/adapter-manifest.schema.json).

## Validation errors (`conjure validate`)

Validation reports **all** problems in one pass. The common ones:

**`id collides with a built-in harness (codex, claude)`**

`codex` and `claude` are built into coven core; a manifest can't redefine
them. Pick another id — lowercase `[a-z0-9._-]+`.

**`declares stream but no 'stream_args' provided`**

`capabilities.stream: true` requires a `stream_args` block with non-empty
`prefix_args`. Either add the real stream argv, or set `stream: false` if the
runtime doesn't have a persistent stream-JSON mode. **Don't fake it** — a
false `stream: true` hangs a real session.

**`declares preassigned session id but no 'stream_args.session_id_flag'`**

Same shape: the capability claims coven can pick the session id, so the
manifest must say which flag delivers it.

**`'stream_args' provided but 'capabilities.stream' is false (dead config)`**

Either the runtime streams (set the capability) or it doesn't (delete the
block).

**`executable must be a bare command name (no path separators or whitespace)`**

No `/`, `\`, or whitespace — `"opencode"`, not `"/usr/local/bin/opencode"`.
PATH resolution is the runtime's install problem, and `install_hint` is where
you tell the user how to solve it.

**`model_arg_template … template must contain the '{model}' placeholder`**

The template is an argv with a placeholder; without `{model}` there's nowhere
to put the chosen model.

## Conformance failures (`conjure test`)

**`executable 'aria' not found on PATH`** (exit code 1)

The probe runs the *real* binary. Install the runtime per your own
`install_hint` (if following the hint doesn't fix this, the hint isn't good
enough — improve it). In environments where the runtime can't be installed
(CI), use `--skip-binary` and run the full probe locally before the PR.

**`declared model flag '--model' not seen in probe output (verify manually)`** (warning)

Soft warning only — CLIs don't always list every flag in `--help`. Verify the
flag works by hand (`<cmd> --model <m> …`) and note that in your PR.

**The probe hangs**

`conjure test` only ever calls `--version` / `--help` with a bounded wait. If
that hangs, the runtime is doing something interactive on those flags —
worth reporting upstream, and worth a note in your PR.

## Registry errors (`conjure registry …`)

**`registry check` fails / CI "registry drift" step is red**

A source manifest under `registry/runtimes/` changed without rebuilding the
committed index. Fix:

```sh
conjure registry build
git add crates/coven-runtime-registry/canonical/index.json
```

**`<id> <version> is already published with different content` (immutability error)**

You edited a released `(id, version)`'s content. Released versions are
immutable (same rule as crates.io) — bump the version instead:

```sh
# bump "version" in the manifest, then
conjure registry add my-runtime.json
```

**`registry/runtimes/<id>/<version>.json already exists — bump the version or pass --force`**

`registry add` won't overwrite an existing source file. `--force` exists for
fixing a mistake *within the same unmerged PR*, never for rewriting a version
that already merged — the immutability check above catches that regardless.

**`registry add` rejects the manifest**

`registry add` requires exactly **one** adapter
(`registry sources hold one adapter per file`) with a semver `version`
(`adapter must set a 'version' to be accepted into the registry`). Split
multi-adapter manifests and add a `version` field.

## CI failures on your PR

CI runs exactly what [`CONTRIBUTING.md`](../CONTRIBUTING.md) tells you to run
locally. Mapping the failing job to the fix:

| Failing CI step | Local reproduction | Usual fix |
|-----------------|--------------------|-----------|
| `Check formatting` | `cargo fmt --all --check` | `cargo fmt --all` |
| `Clippy` | `cargo clippy --workspace --all-targets -- -D warnings` | Fix the lint; there are **no exceptions** for `-D warnings`. |
| `Registry index in sync` | `conjure registry check` | `conjure registry build` and commit the index. |
| `Test` | `cargo test --workspace --locked` | See below for the two tests specific to this repo. |
| `Dependency audit` | `cargo deny check advisories licenses bans sources` | Reconsider the new dependency; prefer `default-features = false`. |

**`schema_examples` test fails**

The JSON Schema, the example manifests, and the Rust types must agree.

- Added an example manifest? Add it to the list in
  `crates/coven-runtime-spec/tests/schema_examples.rs`.
- Added a manifest field to the Rust types? Add it to
  `schema/adapter-manifest.schema.json` too — **both** snake_case and
  camelCase spellings — in the same PR.

**`committed_index_matches_sources` test fails**

Same as registry drift above: `conjure registry build`, commit the result.

**`--locked` fails to resolve**

You changed a dependency without committing `Cargo.lock`. Run the build once
without `--locked` and commit the updated lockfile.

## Runtime works in your terminal but not under Coven

- **Prompt lands in the wrong place** — coven appends the prompt as the *last*
  argv token after `non_interactive_prompt_prefix_args`. If the runtime needs
  the prompt behind a flag or on stdin, the prefix-args model may not fit;
  open an issue rather than shipping an adapter that half-works.
- **Streaming session hangs** — your `stream_args.prefix_args` don't actually
  hold the process open for stream-JSON input, or the runtime buffers output.
  Re-verify by hand: run the exact argv from the manifest and type an event.
- **`--permission` / `--model` silently ignored** — the manifest declares no
  `sandbox` / model selector; that's a warned no-op by design. Add the mapping
  if the runtime has one.

## Still stuck?

Open a [New runtime adapter](https://github.com/OpenCoven/coven-runtimes/issues/new/choose)
issue with the manifest, the exact command, and the output. Capability claims
that are hard to verify are exactly what review discussion is for.
