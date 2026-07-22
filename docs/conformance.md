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
| `preassigned_session_id` (`preassignedSessionId`) | `harness_supports_preassigned_session_id` | `stream_args.session_id_flag` |
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

### Registry metadata (optional; ignored by coven core)

`version` (semver), `homepage` (URL), `description` (one line).

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
8. `capabilities.preassigned_session_id` ⇒ `stream_args.session_id_flag`.
9. `stream_args` present ⇒ `capabilities.stream` true (no dead config).

## Conformance probe (`conjure test`)

Beyond the static rules, `conjure test` runs **dynamic** checks that need the
runtime present:

- the `executable` resolves on `PATH`;
- it runs cleanly for a bounded probe (`--version`, then `--help`);
- declared `model_flag` / `system_prompt_flag` / sandbox / stream flags are
  mentioned in probe output (a **soft warning** only — CLIs don't always list
  every flag).

The probe never sends a prompt or does work. Use `--skip-binary` in CI where the
runtime isn't installed.
