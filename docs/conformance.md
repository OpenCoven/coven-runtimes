# Conformance Specification

This is the authoritative field reference and rule set for a Coven runtime
adapter manifest, as implemented by `coven-runtime-spec`. `conjure validate`,
the registry, and (eventually) `coven` core all enforce these same rules.

Schema version: **1** (`coven_runtime_spec::SCHEMA_VERSION`).

## Manifest envelope

```jsonc
{ "adapters": [ /* one or more RuntimeAdapter */ ] }
```

- `adapters` — non-empty array. An empty manifest is a validation error.

## RuntimeAdapter fields

Field names are **snake_case-canonical with camelCase aliases**. Both spellings
parse; canonical output is snake_case.

### Required

| Field | Type | Rule |
|-------|------|------|
| `id` | string | Lowercase `[a-z0-9._-]+`. Must not equal a built-in (`codex`, `claude`). Unique within a manifest. |
| `label` | string | Non-empty. |
| `executable` | string | Bare command name — no `/`, `\`, or whitespace. |
| `install_hint` | string | Non-empty. Surfaced by `coven doctor`. |

### Launch args

| Field (aliases) | Type | Notes |
|-----------------|------|-------|
| `interactive_prompt_prefix_args` (`interactivePromptPrefixArgs`) | string[] | argv prefix for interactive launch; prompt appended last. |
| `non_interactive_prompt_prefix_args` (`nonInteractivePromptPrefixArgs`) | string[] | argv prefix for one-shot launch; prompt appended last. |
| `prompt_flag` (`promptFlag`) | string \| null | Binds the one-shot prompt as `--flag=<prompt>` for runtimes with no positional prompt slot (Copilot `--prompt`, Grok Build `--single`). `null` ⇒ prompt is the final positional. **Non-blank when present.** |
| `interactive_prompt_flag` (`interactivePromptFlag`) | string \| null | Binds the prompt for an interactive-with-prompt launch (Copilot `--interactive`). **Non-blank when present.** |
| `system_prompt_flag` (`systemPromptFlag`) | string \| null | Flag that injects a system prompt (e.g. `--append-system-prompt`). `null` ⇒ identity is prepended to the prompt instead. |

### Model selection

| Field (aliases) | Type | Rule |
|-----------------|------|------|
| `model_flag` (`modelFlag`) | string \| null | Simple `--flag <value>` selector. |
| `model_arg_template` (`modelArgTemplate`) | string \| null | argv template for non-trivial selection. **Must contain `{model}`.** Takes precedence over `model_flag`. |

Declare neither and `coven run --model` is a warned no-op for the runtime.

### Capabilities

`capabilities` object — every field defaults to `false` (the conservative
baseline of a plain one-shot CLI). These replace coven's hardcoded
`harness_supports_*` string checks 1:1.

| Field (aliases) | Mirrors coven predicate | Requires |
|-----------------|-------------------------|----------|
| `stream` | `harness_supports_stream_mode` | `stream_args` (non-empty `prefix_args`) |
| `preassigned_session_id` (`preassignedSessionId`) | `harness_supports_preassigned_session_id` | `stream_args.session_id_flag` when `stream`, else `continuity_args.session_id_flag` |
| `think` | `harness_supports_think` | — |
| `speed` | `harness_supports_speed` | — |

### Sandbox

`sandbox` object (optional). Maps the composer's Access chip to the runtime's
native permission args. Omit it and `coven run --permission` is a warned no-op
(today's behavior for every manifest).

Two structural forms are accepted (distinguished by their fields, no tag):

**Flag form** — one flag, one value per policy (Codex, Claude):

| Field (aliases) | Type | Rule |
|-----------------|------|------|
| `flag` | string | Non-empty. e.g. `--sandbox` (Codex), `--permission-mode` (Claude). |
| `full` | string | Non-empty. Value for the unrestricted policy. |
| `read_only` (`readOnly`, `read-only`) | string | Non-empty. Value for read-only/plan policy. |

**Args form** — a whole argv list per policy, for runtimes whose permission
surface is boolean or repeatable flags (GitHub Copilot CLI):

| Field (aliases) | Type | Rule |
|-----------------|------|------|
| `full_args` (`fullArgs`) | string[] | ≥1 non-empty token. e.g. `["--allow-all"]`. |
| `read_only_args` (`readOnlyArgs`) | string[] | ≥1 non-empty token. e.g. `["--deny-tool", "write", "--deny-tool", "shell"]`. |

```jsonc
// Flag form (Claude)
"sandbox": { "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" }
// Args form (Copilot)
"sandbox": { "full_args": ["--allow-all"], "read_only_args": ["--deny-tool", "write", "--deny-tool", "shell"] }
```

### Stream args

`stream_args` object (optional; required when `capabilities.stream`).

| Field (aliases) | Type | Notes |
|-----------------|------|-------|
| `prefix_args` (`prefixArgs`) | string[] | argv that enters persistent stream-json mode. Non-empty when streaming. |
| `session_id_flag` (`sessionIdFlag`) | string \| null | Flag to pre-assign the session id. Required if `preassigned_session_id`. |
| `resume_flag` (`resumeFlag`) | string \| null | Flag to resume an existing session. |

Providing `stream_args` **without** `capabilities.stream` is flagged as dead
config.

### Continuity args

`continuity_args` (`continuityArgs`) object (optional). One-shot
non-interactive session continuity: how a cold-started turn initializes a
named conversation or resumes an existing one via the runtime CLI's own
session mechanism. Mirrors `stream_args` for runtimes without a long-lived
stream mode (GitHub Copilot CLI, Grok Build).

| Field (aliases) | Type | Notes |
|-----------------|------|-------|
| `init_prefix_args` (`initPrefixArgs`) | string[] | argv prepended when initializing a fresh named conversation. |
| `resume_prefix_args` (`resumePrefixArgs`) | string[] | argv prepended when resuming; runtimes with a positional resume id put it here. |
| `session_id_flag` (`sessionIdFlag`) | string \| null | Pre-assigns the session id on a fresh launch. Requires `capabilities.preassigned_session_id` (dead config otherwise). |
| `resume_flag` (`resumeFlag`) | string \| null | Resumes an existing session (e.g. `--resume`). |

`continuity_args` must declare a **usable init or resume launch**: a non-blank
`session_id_flag`/`resume_flag` or at least one non-blank prefix token.

### Event protocol

`event_protocol` (`eventProtocol`) string enum (optional). Declares that the
runtime's **finite one-shot** headless process emits a machine-readable stdout
protocol the host translates into its own event model. Mutually exclusive with
`capabilities.stream`: an event protocol exits after each prompt (continuity
rides `continuity_args` cold-start resume), while stream mode is one
long-lived bidirectional process.

| Value | Meaning |
|-------|---------|
| `grok-headless-v1` | Grok Build's public `--output-format streaming-json` schema (`text`/`thought`/`end`/`error` frames; unknown frame types are ignored by the host for forward compatibility). |

### Registry metadata (optional; ignored by coven core)

`version` (semver), `homepage` (URL), `description` (one line).

## Unknown fields & forward compatibility

Two layers with different strictness:

- **Parsing is tolerant.** `coven-runtime-spec` ignores fields it does not
  recognize, and an unrecognized `event_protocol` value parses as an internal
  `Unknown` marker. A registry index written by a newer spec version therefore
  still loads on older consumers, degrading only the affected adapter instead
  of failing the whole document. (Spec versions ≤ 0.1.3 predate this rule and
  reject any index containing newer fields — see
  [`adoption.md`](adoption.md).)
- **Authoring is strict.** `conjure` rejects any field no spec version
  recognizes (`unknown_manifest_fields`), and the JSON Schema declares
  `additionalProperties: false`, so typos fail before a manifest reaches the
  registry. `validate_manifest` also rejects an `Unknown` event protocol: an
  authored manifest must name a protocol its target spec knows.

`conjure` applies the strict layer to **registry indexes too** (`validate
--registry`, `registry build`, `registry yank`): those flows load and rewrite
the whole index, so content from a newer spec refuses to load rather than
being silently dropped or rewritten. Upgrade `conjure` before mutating an
index produced by a newer spec. The read-only `registry list` uses the
tolerant layer instead — one newer entry must not make every runtime
unlistable.

The `sandbox` object keeps strict field matching in both layers: its two
structural forms are distinguished by their field sets, so evolving the
sandbox shape is a spec-version event, not a silently-ignorable addition.

## Validation rules (summary)

`conjure validate` reports **all** problems in one pass:

1. Manifest declares ≥1 adapter.
2. Adapter ids are unique, well-formed, and not built-in collisions.
3. Executable is a bare command name.
4. `label` and `install_hint` are non-empty.
5. `model_arg_template`, if present, contains `{model}`.
6. `sandbox`, if present, has non-empty `flag` / `full` / `read_only` (flag
   form) or ≥1 non-empty token in each of `full_args` / `read_only_args`
   (args form).
7. `capabilities.stream` ⇒ `stream_args` with non-empty `prefix_args`.
8. `capabilities.preassigned_session_id` ⇒ the session id flag on the active
   launch path: `stream_args.session_id_flag` for streaming adapters,
   `continuity_args.session_id_flag` otherwise.
9. `stream_args` present ⇒ `capabilities.stream` true (no dead config).
10. `continuity_args` present ⇒ a usable init or resume launch; its
    `session_id_flag` requires `capabilities.preassigned_session_id`
    (no dead config).
11. `event_protocol` and `capabilities.stream` are mutually exclusive.
12. An `event_protocol` value the spec does not recognize fails validation
    (parse-level `Unknown` is for index consumers, never for authored
    manifests).

## Conformance probe (`conjure test`)

Beyond the static rules, `conjure test` runs **dynamic** checks that need the
runtime present:

- the `executable` resolves on `PATH`;
- it runs cleanly for a bounded probe (`--version`, then `--help`);
- declared `model_flag` / sandbox flags are mentioned in probe output
  (a **soft warning** only — CLIs don't always list every flag).

The probe never sends a prompt or does work. Use `--skip-binary` in CI where the
runtime isn't installed.
