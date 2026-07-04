<!-- Thanks for contributing to coven-runtimes. Fill in the relevant section. -->

## What & why

<!-- What does this PR do, and why? Link any issue: Closes #NN -->

## Type

- [ ] New runtime adapter (manifest)
- [ ] Update to an existing adapter
- [ ] SDK change (`coven-runtime-*` crate)
- [ ] Docs / tooling / CI

---

## If this adds or changes a runtime adapter

- [ ] `covenrt validate <manifest> --verbose` passes with zero problems
- [ ] Every declared capability is real (no `stream: true` without working `stream_args`, etc.)
- [ ] `id` is `[a-z0-9._-]+` and doesn't collide with a built-in (`codex`, `claude`)
- [ ] `install_hint` tells a user how to obtain the binary
- [ ] If added to the registry index: entry `adapter.id` matches its runtime key and `version` is semver
- [ ] Ran `covenrt test <manifest>` against the real binary (or noted why it was skipped)

## If this changes the SDK

- [ ] `cargo fmt --all --check` clean
- [ ] `cargo clippy --workspace --all-targets -- -D warnings` clean
- [ ] `cargo test --workspace --locked` green
- [ ] Manifest shape changes are backward-compatible (added fields only; snake_case + camelCase aliases)
- [ ] JSON Schema updated in the same PR if the manifest shape changed
- [ ] `SCHEMA_VERSION` bumped if the change is backward-incompatible
- [ ] `coven-runtime-spec` remains pure (no I/O / async / process spawning)

## Notes for reviewers

<!-- Anything non-obvious: tradeoffs, follow-ups, coven-core coordination. -->
