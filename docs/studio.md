# `conjure studio` — the interactive manifest workbench

`conjure studio` is a full-screen terminal UI that collapses the
edit → validate → preview → probe loop of manifest authoring into one live
screen. Instead of alternating between a text editor and
`conjure validate`/`conjure test` runs, you edit fields in a form and watch
three panels react on every keystroke:

```sh
conjure studio aria.json                      # edit an existing manifest
conjure studio zephyr.json --flavor streaming # start a new one from a scaffold
```

If the file doesn't exist, the studio opens a scaffold named after the file
stem (so the filename must be a valid adapter id) and writes the file on first
save. If it does exist, it is loaded with the same strict parser as
`conjure validate` — typos and unknown fields are rejected before the UI opens.

## The screen

```
 conjure studio — aria.json • modified          adapter 1/1 (aria)
┌ Manifest ─────────────────────┐┌ Validation (0) ────────────────┐
│ ─ Identity                    ││ ✓ valid — zero problems        │
│   id                 aria     │└────────────────────────────────┘
│   label              Aria     │┌ Launch preview (illustrative) ─┐
│   executable         aria     ││ one-shot   $ aria exec -m ...  │
│ ─ Capabilities                ││ interactive$ aria              │
│   stream             [ ] false│└────────────────────────────────┘
│   ...                        ││┌ Conformance probe ─────────────┐
│                               ││ press t to probe the binary    │
└───────────────────────────────┘└────────────────────────────────┘
 what this field means (help for the focused row)
 status message   ↑/↓ move · Tab section · Enter edit · s save · …
```

- **Manifest form** (left) — every manifest field as a row, grouped into the
  sections the UI uses: Identity, Launch, Prompt binding, Model, Capabilities,
  Sandbox, Stream args, and Continuity args. Rows with validation problems are
  marked `✗` in red. Sandbox detail rows follow the selected mapping shape, so
  you can't edit a field the current shape doesn't have.
- **Validation** (top right) — the full `conjure validate` rule set re-run on
  every change (it's the same pure `validate_manifest` the CLI and CI use).
  Problems caused by the focused field are highlighted. The panel title shows
  the count; the PR bar is zero.
- **Launch preview** (middle right) — the argv Coven would compose for each
  mode the manifest claims: one-shot, interactive, sandboxed (full and
  read-only), streaming, and continuity init/resume. Placeholders like
  `<prompt>`, `<model-without-provider>`, `<provider/model>`, and `<session-id>`
  stand in for runtime values. Illustrative, not executed.
- **Conformance probe** (bottom right) — press `t` to run the same binary
  probe as `conjure test`: PATH lookup, bounded model + leading-subcommand help
  (with later prefix tokens omitted) or root help/version fallback, and soft
  checks that declared flags appear in the successful output.

## Keys

| Key | Action |
|-----|--------|
| `↑`/`↓` or `k`/`j` | move between fields |
| `Tab` / `Shift-Tab` | jump to next / previous section |
| `Enter` or `i` | edit text/args, toggle booleans, or cycle selectors |
| `Space` | toggle a boolean / cycle the sandbox kind or `model_id_transform` |
| `Enter` / `Esc` (while editing) | commit / cancel the edit |
| `[` / `]` | previous / next adapter in a multi-adapter manifest |
| `s` | save (canonical pretty JSON, identical to `conjure new` output) |
| `t` | probe the binary (`conjure test`'s checks) |
| `q` | quit — asks for confirmation if there are unsaved changes |
| `Ctrl-C` | force quit without saving |

## Field semantics worth knowing

- **Args fields** (`prefix args`, `full args`, …) are edited as
  space-separated tokens. Arguments that themselves contain spaces can't be
  expressed here — edit the JSON directly for that (rare) case.
- **Blank streaming/continuity sections normalize away.** If you clear every
  `stream_args` or `continuity_args` row, the manifest serializes without the
  block, exactly like a scaffold that never had it.
- **The sandbox selector cycles `none → flag → args`** and resets the
  mapping's values on each change, because the two shapes share no fields.
- **`model_id_transform` is a selector, not free text.** Space or Enter cycles
  `strip_provider ↔ preserve`; `preserve` is valid only when `model_flag` or
  `model_arg_template` declares model selection.
- **Saving with problems is allowed** (you'll get a status warning) so you can
  checkpoint work in progress — but `conjure validate` still gates the PR.

## Scripts and CI

The studio requires an interactive terminal and refuses to start when stdin or
stdout is piped — scripted flows should use `conjure new`, `conjure validate`,
and `conjure test`, which are the same machinery without the screen.
