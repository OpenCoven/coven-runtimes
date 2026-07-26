//! The launch preview: the concrete argv lines Coven would compose from the
//! adapter's declarations, with `<prompt>`, model-id, `<system>`, and
//! `<session-id>` placeholders.
//!
//! The preview is **illustrative** — exact composition order lives in coven
//! core — but every token shown comes straight from the manifest, so a typo'd
//! flag or a misplaced prefix arg is visible the moment it's typed. The prompt
//! is always the last token, matching the documented contract.

use coven_runtime_spec::{ModelIdTransform, Permission, RuntimeAdapter};

/// One preview line: a launch-mode label and its argv.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct PreviewLine {
    pub(crate) label: &'static str,
    pub(crate) argv: Vec<String>,
}

impl PreviewLine {
    pub(crate) fn command(&self) -> String {
        self.argv.join(" ")
    }
}

/// Model-selection tokens: template form wins over the simple flag, matching
/// the spec's precedence.
fn model_tokens(adapter: &RuntimeAdapter) -> Vec<String> {
    let placeholder = model_placeholder(adapter);
    if let Some(template) = &adapter.model_arg_template {
        return template
            .split_whitespace()
            .map(|token| token.replace("{model}", placeholder))
            .collect();
    }
    if let Some(flag) = &adapter.model_flag {
        return vec![flag.clone(), placeholder.into()];
    }
    Vec::new()
}

fn model_placeholder(adapter: &RuntimeAdapter) -> &'static str {
    match adapter.model_id_transform {
        ModelIdTransform::StripProvider => "<model-without-provider>",
        ModelIdTransform::Preserve => "<provider/model>",
    }
}

fn system_tokens(adapter: &RuntimeAdapter) -> Vec<String> {
    match &adapter.system_prompt_flag {
        Some(flag) => vec![flag.clone(), "\"<system>\"".into()],
        None => Vec::new(),
    }
}

/// The one-shot prompt binding: `--flag="<prompt>"` or a positional.
fn prompt_tokens(flag: Option<&str>) -> Vec<String> {
    match flag {
        Some(flag) => vec![format!("{flag}=\"<prompt>\"")],
        None => vec!["--".into(), "\"<prompt>\"".into()],
    }
}

fn line(
    label: &'static str,
    adapter: &RuntimeAdapter,
    sandbox_tokens: Vec<String>,
    mode_tokens: Vec<String>,
    prompt_tokens: Vec<String>,
) -> PreviewLine {
    let mut argv = vec![adapter.executable.clone()];
    argv.extend(model_tokens(adapter));
    argv.extend(sandbox_tokens);
    argv.extend(system_tokens(adapter));
    argv.extend(mode_tokens);
    argv.extend(prompt_tokens);
    PreviewLine { label, argv }
}

/// Build the preview for every launch mode the adapter declares.
pub(crate) fn launch_preview(adapter: &RuntimeAdapter) -> Vec<PreviewLine> {
    let mut lines = Vec::new();

    lines.push(line(
        "one-shot",
        adapter,
        Vec::new(),
        adapter.non_interactive_prompt_prefix_args.clone(),
        prompt_tokens(adapter.prompt_flag.as_deref()),
    ));

    lines.push(line(
        "interactive",
        adapter,
        Vec::new(),
        adapter.interactive_prompt_prefix_args.clone(),
        prompt_tokens(
            adapter
                .interactive_prompt_flag
                .as_deref()
                .or(adapter.prompt_flag.as_deref()),
        ),
    ));

    if let Some(sandbox) = &adapter.sandbox {
        lines.push(line(
            "sandbox full",
            adapter,
            sandbox.args(Permission::Full),
            adapter.non_interactive_prompt_prefix_args.clone(),
            prompt_tokens(adapter.prompt_flag.as_deref()),
        ));
        lines.push(line(
            "sandbox read-only",
            adapter,
            sandbox.args(Permission::ReadOnly),
            adapter.non_interactive_prompt_prefix_args.clone(),
            prompt_tokens(adapter.prompt_flag.as_deref()),
        ));
    }

    if let Some(stream) = &adapter.stream_args {
        let mut launch = stream.prefix_args.clone();
        if let Some(flag) = &stream.session_id_flag {
            launch.extend([flag.clone(), "<session-id>".into()]);
        }
        lines.push(line("stream", adapter, Vec::new(), launch, Vec::new()));
        if let Some(flag) = &stream.resume_flag {
            lines.push(line(
                "stream resume",
                adapter,
                Vec::new(),
                stream
                    .prefix_args
                    .iter()
                    .cloned()
                    .chain([flag.clone(), "<session-id>".into()])
                    .collect(),
                Vec::new(),
            ));
        }
    }

    if let Some(continuity) = &adapter.continuity_args {
        if continuity.has_init_launch() {
            let mut init = continuity.init_prefix_args.clone();
            if let Some(flag) = continuity.session_id_flag() {
                init.extend([flag.to_string(), "<session-id>".into()]);
            }
            lines.push(line(
                "continuity init",
                adapter,
                Vec::new(),
                init,
                prompt_tokens(adapter.prompt_flag.as_deref()),
            ));
        }

        if continuity.has_resume_launch() {
            let mut resume = continuity.resume_prefix_args.clone();
            if let Some(flag) = continuity.resume_flag() {
                resume.push(flag.to_string());
            }
            resume.push("<session-id>".into());
            lines.push(line(
                "continuity resume",
                adapter,
                Vec::new(),
                resume,
                prompt_tokens(adapter.prompt_flag.as_deref()),
            ));
        }
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_registry::RegistryIndex;
    use coven_runtime_spec::{AdapterManifest, Capabilities, ModelIdTransform, SandboxMapping};

    fn adapter(raw: &str) -> RuntimeAdapter {
        AdapterManifest::from_json(raw)
            .expect("test adapter parses")
            .adapters
            .remove(0)
    }

    fn find<'a>(lines: &'a [PreviewLine], label: &str) -> &'a PreviewLine {
        lines
            .iter()
            .find(|l| l.label == label)
            .unwrap_or_else(|| panic!("no `{label}` line in {lines:?}"))
    }

    fn canonical_adapter(id: &str) -> RuntimeAdapter {
        RegistryIndex::canonical()
            .resolve_latest(id)
            .unwrap_or_else(|error| panic!("resolve latest `{id}`: {error}"))
            .adapter
            .clone()
    }

    fn canonical_manifest(adapter: RuntimeAdapter) -> String {
        format!(
            "{}\n",
            AdapterManifest {
                adapters: vec![adapter]
            }
            .to_json_pretty()
            .expect("serialize canonical one-adapter manifest")
        )
    }

    /// Claude shape: positional prompt, flag-form sandbox, stream mode.
    #[test]
    fn claude_shape_preview() {
        let a = adapter(
            r#"{ "adapters": [{
                "id": "claude", "label": "Claude Code", "executable": "claude",
                "non_interactive_prompt_prefix_args": ["--print"],
                "install_hint": "npm i -g @anthropic-ai/claude-code",
                "system_prompt_flag": "--system-prompt",
                "model_flag": "--model",
                "capabilities": { "stream": true, "preassigned_session_id": true },
                "sandbox": { "flag": "--permission-mode", "full": "bypassPermissions", "read_only": "plan" },
                "stream_args": { "prefix_args": ["-p", "--output-format", "stream-json"], "session_id_flag": "--session-id", "resume_flag": "--resume" }
            }]}"#,
        );
        let lines = launch_preview(&a);

        assert_eq!(
            find(&lines, "one-shot").command(),
            r#"claude --model <model-without-provider> --system-prompt "<system>" --print -- "<prompt>""#
        );
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"claude --model <model-without-provider> --system-prompt "<system>" -- "<prompt>""#
        );
        assert_eq!(
            find(&lines, "stream").command(),
            r#"claude --model <model-without-provider> --system-prompt "<system>" -p --output-format stream-json --session-id <session-id>"#
        );
        assert_eq!(
            find(&lines, "stream resume").command(),
            r#"claude --model <model-without-provider> --system-prompt "<system>" -p --output-format stream-json --resume <session-id>"#
        );
        assert_eq!(
            find(&lines, "sandbox full").command(),
            r#"claude --model <model-without-provider> --permission-mode bypassPermissions --system-prompt "<system>" --print -- "<prompt>""#
        );
        assert_eq!(
            find(&lines, "sandbox read-only").command(),
            r#"claude --model <model-without-provider> --permission-mode plan --system-prompt "<system>" --print -- "<prompt>""#
        );
        assert!(lines.iter().all(|l| l.label != "continuity init"));
    }

    /// Grok shape: flag-bound prompt, args-form sandbox, continuity mode.
    #[test]
    fn grok_shape_preview() {
        let a = adapter(
            r#"{ "adapters": [{
                "id": "grok", "label": "Grok Build", "executable": "grok",
                "non_interactive_prompt_prefix_args": ["--output-format", "plain"],
                "install_hint": "Install Grok Build.",
                "prompt_flag": "--single",
                "system_prompt_flag": "--rules",
                "model_flag": "--model",
                "capabilities": { "preassigned_session_id": true },
                "sandbox": { "full_args": ["--permission-mode", "bypassPermissions"], "read_only_args": ["--sandbox", "read-only"] },
                "continuity_args": {
                    "init_prefix_args": ["--output-format", "plain"],
                    "resume_prefix_args": ["--output-format", "plain"],
                    "session_id_flag": "--session-id",
                    "resume_flag": "--resume"
                }
            }]}"#,
        );
        let lines = launch_preview(&a);

        assert_eq!(
            find(&lines, "one-shot").command(),
            r#"grok --model <model-without-provider> --rules "<system>" --output-format plain --single="<prompt>""#
        );
        assert_eq!(
            find(&lines, "continuity init").command(),
            r#"grok --model <model-without-provider> --rules "<system>" --output-format plain --session-id <session-id> --single="<prompt>""#
        );
        assert_eq!(
            find(&lines, "continuity resume").command(),
            r#"grok --model <model-without-provider> --rules "<system>" --output-format plain --resume <session-id> --single="<prompt>""#
        );
        assert_eq!(
            find(&lines, "sandbox full").command(),
            r#"grok --model <model-without-provider> --permission-mode bypassPermissions --rules "<system>" --output-format plain --single="<prompt>""#
        );
        assert!(lines.iter().all(|l| l.label != "stream"));
    }

    #[test]
    fn copilot_shape_preview_uses_single_prompt_tokens() {
        let a = adapter(
            r#"{ "adapters": [{
                "id": "copilot", "label": "GitHub Copilot CLI", "executable": "copilot",
                "install_hint": "Install GitHub Copilot CLI.",
                "non_interactive_prompt_prefix_args": ["--no-color"],
                "prompt_flag": "--prompt",
                "interactive_prompt_flag": "--interactive",
                "model_flag": "--model",
                "sandbox": {
                    "full_args": ["--allow-all"],
                    "read_only_args": ["--deny-tool", "write"]
                }
            }]}"#,
        );
        let lines = launch_preview(&a);

        assert_eq!(
            find(&lines, "one-shot").argv,
            [
                "copilot",
                "--model",
                "<model-without-provider>",
                "--no-color",
                "--prompt=\"<prompt>\""
            ]
        );
        assert_eq!(
            find(&lines, "interactive").argv,
            [
                "copilot",
                "--model",
                "<model-without-provider>",
                "--interactive=\"<prompt>\""
            ]
        );
        assert_eq!(
            find(&lines, "sandbox read-only").argv,
            [
                "copilot",
                "--model",
                "<model-without-provider>",
                "--deny-tool",
                "write",
                "--no-color",
                "--prompt=\"<prompt>\""
            ]
        );
    }

    /// Minimal adapter: exactly the one-shot + interactive pair, prompt last.
    #[test]
    fn minimal_preview_has_two_lines() {
        let a = RuntimeAdapter {
            id: "aria".into(),
            label: "Aria".into(),
            executable: "aria".into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["run".into()],
            prompt_flag: None,
            interactive_prompt_flag: None,
            install_hint: "hint".into(),
            system_prompt_flag: None,
            model_flag: None,
            model_arg_template: None,
            model_id_transform: ModelIdTransform::StripProvider,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
            continuity_args: None,
            version: None,
            homepage: None,
            description: None,
        };
        let lines = launch_preview(&a);
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].command(), r#"aria run -- "<prompt>""#);
        assert_eq!(lines[1].command(), r#"aria -- "<prompt>""#);
        // Positional prompt data is always protected by an options terminator.
        for l in &lines {
            assert_eq!(&l.argv[l.argv.len() - 2..], ["--", "\"<prompt>\""]);
        }
    }

    #[test]
    fn preview_distinguishes_preserved_and_stripped_model_ids() {
        let mut adapter = adapter(
            r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h", "model_flag": "--model" }]}"#,
        );
        assert!(launch_preview(&adapter)[0]
            .command()
            .contains("<model-without-provider>"));
        adapter.model_id_transform = ModelIdTransform::Preserve;
        assert!(launch_preview(&adapter)[0]
            .command()
            .contains("<provider/model>"));
    }

    #[test]
    fn latest_hermes_recipe_preview_is_native_and_preserves_model_id() {
        let a = canonical_adapter("hermes");
        let lines = launch_preview(&a);

        assert_eq!(
            find(&lines, "one-shot").command(),
            r#"hermes --model <provider/model> chat --source coven -Q --query="<prompt>""#
        );
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"hermes --model <provider/model> chat --source coven --query="<prompt>""#
        );
        assert_eq!(a.version.as_deref(), Some("1.0.3"));
        assert_eq!(a.model_id_transform, ModelIdTransform::Preserve);
    }

    #[test]
    fn latest_opencode_recipe_preview_uses_run_and_preserves_model_id() {
        let a = canonical_adapter("opencode");
        let lines = launch_preview(&a);
        let expected = r#"opencode --model <provider/model> run -- "<prompt>""#;

        assert_eq!(find(&lines, "one-shot").command(), expected);
        assert_eq!(find(&lines, "interactive").command(), expected);
        assert_eq!(a.version.as_deref(), Some("0.1.1"));
        assert_eq!(a.model_id_transform, ModelIdTransform::Preserve);
    }

    #[test]
    fn latest_coven_code_recipe_preview_covers_one_shot_interactive_and_stream() {
        let a = canonical_adapter("coven-code");
        let lines = launch_preview(&a);

        assert_eq!(
            find(&lines, "one-shot").command(),
            r#"coven-code --model <provider/model> --append-system-prompt "<system>" --print -- "<prompt>""#
        );
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"coven-code --model <provider/model> --append-system-prompt "<system>" -- "<prompt>""#
        );
        assert_eq!(
            find(&lines, "stream").command(),
            "coven-code --model <provider/model> --append-system-prompt \"<system>\" \
             --print --input-format stream-json --output-format stream-json \
             --session-id <session-id>"
        );
        assert_eq!(
            find(&lines, "stream resume").command(),
            "coven-code --model <provider/model> --append-system-prompt \"<system>\" \
             --print --input-format stream-json --output-format stream-json \
             --resume <session-id>"
        );
        assert_eq!(a.version.as_deref(), Some("1.0.2"));
        assert_eq!(a.model_id_transform, ModelIdTransform::Preserve);
    }

    #[test]
    fn latest_examples_match_canonical_one_adapter_manifests_byte_for_byte() {
        for (id, example) in [
            ("hermes", include_str!("../../../../examples/hermes.json")),
            (
                "opencode",
                include_str!("../../../../examples/opencode.json"),
            ),
        ] {
            assert_eq!(
                example,
                canonical_manifest(canonical_adapter(id)),
                "{id} example drifted from canonical latest"
            );
        }
    }

    /// `model_arg_template` takes precedence over `model_flag`, matching the
    /// spec.
    #[test]
    fn model_template_wins_over_flag() {
        let a = RuntimeAdapter {
            model_flag: Some("--model".into()),
            model_arg_template: Some("-c model={model}".into()),
            model_id_transform: ModelIdTransform::StripProvider,
            ..adapter(
                r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h" }]}"#,
            )
        };
        let one_shot = launch_preview(&a).remove(0);
        assert_eq!(
            one_shot.command(),
            r#"x -c model=<model-without-provider> -- "<prompt>""#
        );
    }

    #[test]
    fn model_transform_applies_to_template_placeholders() {
        let mut a = adapter(
            r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h", "model_arg_template": "-c model={model}" }]}"#,
        );
        assert_eq!(
            launch_preview(&a)[0].command(),
            r#"x -c model=<model-without-provider> -- "<prompt>""#
        );

        a.model_id_transform = ModelIdTransform::Preserve;
        assert_eq!(
            launch_preview(&a)[0].command(),
            r#"x -c model=<provider/model> -- "<prompt>""#
        );
    }

    #[test]
    fn model_transform_applies_to_stream_and_continuity_previews() {
        let mut a = adapter(
            r#"{ "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "h",
                "system_prompt_flag": "--rules",
                "model_flag": "--ignored",
                "model_arg_template": "-c model={model}",
                "prompt_flag": "--single",
                "capabilities": { "stream": true, "preassigned_session_id": true },
                "stream_args": {
                    "prefix_args": ["--stream"],
                    "session_id_flag": "--session-id",
                    "resume_flag": "--resume"
                },
                "continuity_args": {
                    "init_prefix_args": ["run"],
                    "resume_prefix_args": ["run"],
                    "session_id_flag": "--session-id",
                    "resume_flag": "--resume"
                }
            }]}"#,
        );

        for (transform, placeholder) in [
            (ModelIdTransform::StripProvider, "<model-without-provider>"),
            (ModelIdTransform::Preserve, "<provider/model>"),
        ] {
            a.model_id_transform = transform;
            let lines = launch_preview(&a);
            for (label, expected) in [
                (
                    "stream",
                    format!(
                        r#"x -c model={placeholder} --rules "<system>" --stream --session-id <session-id>"#
                    ),
                ),
                (
                    "stream resume",
                    format!(
                        r#"x -c model={placeholder} --rules "<system>" --stream --resume <session-id>"#
                    ),
                ),
                (
                    "continuity init",
                    format!(
                        r#"x -c model={placeholder} --rules "<system>" run --session-id <session-id> --single="<prompt>""#
                    ),
                ),
                (
                    "continuity resume",
                    format!(
                        r#"x -c model={placeholder} --rules "<system>" run --resume <session-id> --single="<prompt>""#
                    ),
                ),
            ] {
                let command = find(&lines, label).command();
                assert_eq!(command, expected, "{transform:?} {label}");
                assert!(!command.contains("--ignored"), "{command}");
            }
        }
    }

    #[test]
    fn continuity_preview_only_renders_supported_launches() {
        let init_only = adapter(
            r#"{ "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "h",
                "capabilities": { "preassigned_session_id": true },
                "continuity_args": {
                    "init_prefix_args": ["start"],
                    "session_id_flag": "--session-id"
                }
            }]}"#,
        );
        let continuity = init_only.continuity_args.as_ref().unwrap();
        assert!(continuity.has_init_launch());
        assert!(!continuity.has_resume_launch());
        let lines = launch_preview(&init_only);
        assert_eq!(
            find(&lines, "continuity init").command(),
            r#"x start --session-id <session-id> -- "<prompt>""#
        );
        assert!(lines.iter().all(|line| line.label != "continuity resume"));

        let prefix_only_init = adapter(
            r#"{ "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "h",
                "continuity_args": { "init_prefix_args": ["start"] }
            }]}"#,
        );
        let continuity = prefix_only_init.continuity_args.as_ref().unwrap();
        assert!(continuity.has_init_launch());
        assert!(!continuity.has_resume_launch());
        let lines = launch_preview(&prefix_only_init);
        assert_eq!(
            find(&lines, "continuity init").command(),
            r#"x start -- "<prompt>""#
        );
        assert!(lines.iter().all(|line| line.label != "continuity resume"));

        let positional_resume = adapter(
            r#"{ "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "h",
                "continuity_args": { "resume_prefix_args": ["resume"] }
            }]}"#,
        );
        let continuity = positional_resume.continuity_args.as_ref().unwrap();
        assert!(!continuity.has_init_launch());
        assert!(continuity.has_resume_launch());
        let lines = launch_preview(&positional_resume);
        assert!(lines.iter().all(|line| line.label != "continuity init"));
        assert_eq!(
            find(&lines, "continuity resume").command(),
            r#"x resume <session-id> -- "<prompt>""#
        );

        let flagged_resume = adapter(
            r#"{ "adapters": [{
                "id": "x", "label": "X", "executable": "x", "install_hint": "h",
                "continuity_args": {
                    "resume_prefix_args": ["run"],
                    "resume_flag": "--resume"
                }
            }]}"#,
        );
        let continuity = flagged_resume.continuity_args.as_ref().unwrap();
        assert!(!continuity.has_init_launch());
        assert!(continuity.has_resume_launch());
        let lines = launch_preview(&flagged_resume);
        assert!(lines.iter().all(|line| line.label != "continuity init"));
        assert_eq!(
            find(&lines, "continuity resume").command(),
            r#"x run --resume <session-id> -- "<prompt>""#
        );
    }

    #[test]
    fn prompt_flag_renders_one_argv_token() {
        assert_eq!(
            prompt_tokens(Some("--single")),
            vec![r#"--single="<prompt>""#]
        );
        assert_eq!(prompt_tokens(None), vec!["--", r#""<prompt>""#]);
    }

    /// Interactive prompt binding falls back to `prompt_flag` when
    /// `interactive_prompt_flag` is absent.
    #[test]
    fn interactive_falls_back_to_prompt_flag() {
        let mut a = adapter(
            r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h", "prompt_flag": "--single" }]}"#,
        );
        let lines = launch_preview(&a);
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"x --single="<prompt>""#
        );

        a.interactive_prompt_flag = Some("--interactive".into());
        let lines = launch_preview(&a);
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"x --interactive="<prompt>""#
        );
    }

    #[test]
    fn stream_without_optional_flags_has_no_session_tokens() {
        let mut a = adapter(
            r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h" }]}"#,
        );
        a.stream_args = Some(coven_runtime_spec::StreamArgs {
            prefix_args: vec!["--stream".into()],
            session_id_flag: None,
            resume_flag: None,
        });
        let lines = launch_preview(&a);
        assert_eq!(find(&lines, "stream").command(), "x --stream");
        assert!(lines.iter().all(|l| l.label != "stream resume"));
    }

    // Silence unused-import warning for SandboxMapping in this cfg(test).
    #[allow(dead_code)]
    fn _uses(_: Option<SandboxMapping>) {}
}
