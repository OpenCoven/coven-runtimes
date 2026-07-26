# Coven runtime shims

Thin wrappers that adapt a runtime's native CLI to the Coven harness's launch
convention (which appends the user prompt as a positional argument behind an
options terminator: `<cmd> <prefix args…> -- "<prompt>"`).

## `hermes-coven` (legacy compatibility)

`hermes chat` has **no positional prompt slot** — its query is only accepted via
`-q/--query <value>`. Under the harness convention the invocation became
`hermes chat … -q -- "<prompt>"`, which starved `-q` of its value:

```
hermes chat: error: argument -q/--query: expected one argument
```

`hermes-coven` captures the trailing positional prompt and re-emits it as the
inline value of `-q`, so the harness can drive Hermes correctly. With no prompt
(interactive), it launches the REPL and strips any stray `-q`.

This shim is retained for exact historical Hermes recipe versions `1.0.1` and
`1.0.2`. The latest `1.0.3` recipe invokes native `hermes` with explicit
`chat`/prompt flags and does **not** require `hermes-coven`.

### Install for an exact legacy recipe

```sh
install -m 0755 shims/hermes-coven "$HOME/.local/bin/hermes-coven"
# ensure ~/.local/bin is on PATH; `hermes` itself must also be installed.
```

The `1.0.1` and `1.0.2` Hermes manifests set
`"executable": "hermes-coven"`, so the shim must be on `PATH` only when one of
those exact versions is pinned.
