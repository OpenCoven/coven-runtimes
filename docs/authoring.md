# Authoring a runtime adapter, end to end

This is the narrative walkthrough: from a runtime CLI on your PATH to an
**accepted** manifest in the [canonical registry](registry.md), with every
command you'll run and the output you should expect. The field-by-field
reference lives in [`conformance.md`](conformance.md); the acceptance bar in
[`../GOVERNANCE.md`](../GOVERNANCE.md). When something fails, see
[`troubleshooting.md`](troubleshooting.md).

**The journey:**

1. [Study the runtime](#1-study-the-runtime) — learn its CLI surface.
2. [Scaffold](#2-scaffold-the-manifest) — `conjure new`.
3. [Fill in the manifest](#3-fill-in-the-manifest) — map the CLI to fields.
4. [Validate](#4-validate) — `conjure validate`, zero problems.
5. [Conformance-test](#5-conformance-test) — `conjure test` against the binary.
6. [Accept into the registry](#6-accept-into-the-registry) — `conjure registry add`.
7. [Open the PR](#7-open-the-pr) — merging it is what makes the runtime accepted.

Build the toolkit once first:

```sh
git clone https://github.com/OpenCoven/coven-runtimes && cd coven-runtimes
cargo build --release            # binary at target/release/conjure
export PATH="$PWD/target/release:$PATH"
```

---

## 1. Study the runtime

Everything in the manifest is a *claim about how the runtime's CLI behaves*.
Before writing any JSON, answer these questions with the real binary:

| Question | How to find out | Manifest field it feeds |
|----------|-----------------|-------------------------|
| What's the bare command name? | `which <cmd>` | `executable` |
| How does a one-shot, non-interactive prompt run? | Look for `exec` / `run` / `-p` in `<cmd> --help` | `non_interactive_prompt_prefix_args` |
| How do you pick a model? | `--model`? a template like `-m provider/model`? | `model_flag` / `model_arg_template` |
| Can you inject a system prompt via a flag? | e.g. `--append-system-prompt` | `system_prompt_flag` (`null` if not) |
| Does it have a persistent stream-JSON mode? | Look for `--output-format stream-json` or similar | `capabilities.stream` + `stream_args` |
| Can you pre-assign / resume a session id? | `--session-id`, `--resume` | `preassigned_session_id`, `session_id_flag`, `resume_flag` |
| How do permissions map? | One flag with values, or repeatable boolean flags? | `sandbox` (flag form vs args form) |

Run the candidate commands **by hand** first. If you can't make the runtime
stream from your own terminal, don't declare `stream: true` — a false
capability hangs a real Coven session.

## 2. Scaffold the manifest

Pick the flavor that matches what you learned:

```sh
conjure new aria                        # minimal: plain one-shot runtime
conjure new zephyr --flavor streaming   # streaming + sandbox skeleton
```

```
Created aria.json (minimal flavor).
Next: edit it, then `conjure validate aria.json`.
```

The minimal scaffold is a complete, already-valid manifest with every
capability `false` — the conservative baseline. You only *add* claims you can
prove.

## 3. Fill in the manifest

Work through the scaffold top to bottom. Using a real example (the shape the
[OpenCode adapter](../examples/opencode.json) landed with):

```jsonc
{
  "adapters": [{
    "id": "opencode",                 // lowercase [a-z0-9._-]+, not codex/claude
    "label": "OpenCode",              // human name, shown in pickers
    "executable": "opencode",         // bare command — no paths, no spaces
    "non_interactive_prompt_prefix_args": ["run"],   // `opencode run <prompt>`
    "install_hint": "Install OpenCode (https://opencode.ai/docs/cli) and ensure `opencode` resolves on PATH. Verify with `opencode --version`.",
    "system_prompt_flag": null,       // no CLI flag → identity gets prepended
    "model_flag": "--model",
    "capabilities": {                 // baseline: claim nothing you can't prove
      "stream": false,
      "preassigned_session_id": false,
      "think": false,
      "speed": false
    },
    "version": "0.1.0",               // required for registry acceptance
    "homepage": "https://opencode.ai",
    "description": "OpenCode runtime adapter for Coven."
  }]
}
```

Guidance that isn't obvious from the field reference:

- **`install_hint` is UX, not a formality.** It's what `coven doctor` prints
  when the binary is missing. Include the install URL *and* the verify command.
- **Prompt-prefix args**: Coven appends the prompt as the *last* argv token
  after your prefix. `["run"]` means `opencode run "<prompt>"`.
- **Only one of `model_flag` / `model_arg_template`** is needed. Use the
  template form (`"--model {model}"`-style argv) only when selection is more
  than `--flag value`; it must contain `{model}`.
- **Streaming runtimes** additionally need:

  ```jsonc
  "capabilities": { "stream": true, "preassigned_session_id": true },
  "stream_args": {
    "prefix_args": ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"],
    "session_id_flag": "--session-id",
    "resume_flag": "--resume"
  }
  ```

- **Sandbox** has two forms — pick by the runtime's permission surface:

  ```jsonc
  // Flag form: one flag, one value per policy (Claude-style)
  "sandbox": { "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" }
  // Args form: a whole argv per policy (Copilot-style boolean/repeatable flags)
  "sandbox": { "full_args": ["--allow-all"], "read_only_args": ["--deny-tool", "write", "--deny-tool", "shell"] }
  ```

- **`version` is required for registry acceptance** and must be semver — it
  becomes the filename under `registry/runtimes/<id>/<version>.json`.

Editor support: point your editor's JSON language server at
[`schema/adapter-manifest.schema.json`](../schema/adapter-manifest.schema.json)
for completion and inline validation while you edit.

## 4. Validate

```sh
conjure validate aria.json --verbose
```

```
· aria (Aria) — exe `aria`, capabilities: baseline
✓ aria.json valid (1 adapter).
```

Validation reports **all** problems in one pass, so you fix everything in one
round trip:

```
✗ adapter `claude` [id]: id collides with a built-in harness (codex, claude)
✗ adapter `claude` [capabilities.stream]: declares stream but no `stream_args` provided
Error: 2 problem(s) found
```

The PR bar is **zero problems** — warnings included.

## 5. Conformance-test

Static rules can't tell whether your claims are *true*. `conjure test` probes
the real binary:

```sh
conjure test aria.json
```

It checks that `executable` resolves on PATH, that the binary runs cleanly for
a bounded `--version` / `--help` probe, and (as a soft warning) that declared
flags appear in the help output. It never sends a prompt or does work.

```
✓ static validation passed
✗ aria — executable `aria` not found on PATH (Install aria, add it to PATH, …)
Error: conformance probe failed for one or more adapters
```

No binary in your environment (CI, containers)? `--skip-binary` runs the static
half only — but run the full probe on a machine that has the runtime before
opening the PR, and say so in the PR description.

**Beyond the probe, verify each declared capability by hand once:**

- `stream: true` → run the runtime with your exact `stream_args.prefix_args`
  and confirm it emits stream-JSON events and stays alive for input.
- `preassigned_session_id` → launch with `session_id_flag <uuid>`, then
  `resume_flag <uuid>` resumes that session.
- `sandbox.read_only` → confirm the policy actually blocks a write.

## 6. Accept into the registry

Acceptance = your manifest living under
[`registry/runtimes/<id>/<version>.json`](../registry) with the compiled index
in sync. One command does the mechanical part:

```sh
conjure registry add aria.json
```

This validates, copies the manifest to `registry/runtimes/aria/0.1.0.json`,
and rebuilds `crates/coven-runtime-registry/canonical/index.json` — commit
**both**. Verify what CI will verify:

```sh
conjure registry check     # committed index == sources, non-zero exit on drift
conjure registry list      # your runtime shows up with its capabilities
```

```
aria             0.1.0    baseline
copilot          1.0.0    stream, preassignedSessionId
coven-code       1.0.1    stream, preassignedSessionId, think, speed
grok             1.0.0    preassignedSessionId
hermes           1.0.2    baseline
opencode         0.1.0    baseline
```

Two paths, depending on your confidence:

- **Example first** — land the manifest under [`examples/`](../examples) (and
  add it to the `schema_examples` test) without registry acceptance. Good for
  a first iteration; registry acceptance can be a follow-up PR.
- **Straight to accepted** — `conjure registry add` in the same PR. Expect the
  [GOVERNANCE](../GOVERNANCE.md) review to hold you to the capability-truth bar.

## 7. Open the PR

Branch, commit (`feat: add <id> runtime adapter`), and fill in the PR template —
its checklist is the merge bar. Before pushing, run the same gates CI runs:

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets -- -D warnings
cargo test --workspace --locked
conjure registry check                     # if you touched the registry
```

In the PR description, state **how you verified each non-baseline capability**
(the command you ran and what you observed). That's the main thing a reviewer
can't check from the diff alone.

After the merge, released versions are **immutable**: to change the adapter,
add a new `registry/runtimes/<id>/<newversion>.json` — never edit a published
file (see [`registry.md`](registry.md)).
