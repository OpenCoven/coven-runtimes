//! The launch preview: the concrete argv lines Coven would compose from the
//! adapter's declarations, with `<prompt>`, `<model>`, `<system>`, and
//! `<session-id>` placeholders.
//!
//! The preview is **illustrative** — exact composition order lives in coven
//! core — but every token shown comes straight from the manifest, so a typo'd
//! flag or a misplaced prefix arg is visible the moment it's typed. The prompt
//! is always the last token, matching the documented contract.

use coven_runtime_spec::{Permission, RuntimeAdapter};

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
    if let Some(template) = &adapter.model_arg_template {
        return template.split_whitespace().map(str::to_string).collect();
    }
    if let Some(flag) = &adapter.model_flag {
        return vec![flag.clone(), "<model>".into()];
    }
    Vec::new()
}

fn system_tokens(adapter: &RuntimeAdapter) -> Vec<String> {
    match &adapter.system_prompt_flag {
        Some(flag) => vec![flag.clone(), "\"<system>\"".into()],
        None => Vec::new(),
    }
}

/// The one-shot prompt binding: `--flag "<prompt>"` or a positional.
fn prompt_tokens(flag: Option<&str>) -> Vec<String> {
    match flag {
        Some(flag) => vec![flag.to_string(), "\"<prompt>\"".into()],
        None => vec!["\"<prompt>\"".into()],
    }
}

fn line(
    label: &'static str,
    adapter: &RuntimeAdapter,
    parts: impl IntoIterator<Item = Vec<String>>,
) -> PreviewLine {
    let mut argv = vec![adapter.executable.clone()];
    for part in parts {
        argv.extend(part);
    }
    PreviewLine { label, argv }
}

/// Build the preview for every launch mode the adapter declares.
pub(crate) fn launch_preview(adapter: &RuntimeAdapter) -> Vec<PreviewLine> {
    let mut lines = Vec::new();

    lines.push(line(
        "one-shot",
        adapter,
        [
            adapter.non_interactive_prompt_prefix_args.clone(),
            model_tokens(adapter),
            system_tokens(adapter),
            prompt_tokens(adapter.prompt_flag.as_deref()),
        ],
    ));

    lines.push(line(
        "interactive",
        adapter,
        [
            adapter.interactive_prompt_prefix_args.clone(),
            model_tokens(adapter),
            system_tokens(adapter),
            prompt_tokens(
                adapter
                    .interactive_prompt_flag
                    .as_deref()
                    .or(adapter.prompt_flag.as_deref()),
            ),
        ],
    ));

    if let Some(sandbox) = &adapter.sandbox {
        lines.push(line(
            "sandbox full",
            adapter,
            [
                adapter.non_interactive_prompt_prefix_args.clone(),
                sandbox.args(Permission::Full),
                prompt_tokens(adapter.prompt_flag.as_deref()),
            ],
        ));
        lines.push(line(
            "sandbox read-only",
            adapter,
            [
                adapter.non_interactive_prompt_prefix_args.clone(),
                sandbox.args(Permission::ReadOnly),
                prompt_tokens(adapter.prompt_flag.as_deref()),
            ],
        ));
    }

    if let Some(stream) = &adapter.stream_args {
        let mut launch = vec![stream.prefix_args.clone()];
        if let Some(flag) = &stream.session_id_flag {
            launch.push(vec![flag.clone(), "<session-id>".into()]);
        }
        lines.push(line("stream", adapter, launch));
        if let Some(flag) = &stream.resume_flag {
            lines.push(line(
                "stream resume",
                adapter,
                [
                    stream.prefix_args.clone(),
                    vec![flag.clone(), "<session-id>".into()],
                ],
            ));
        }
    }

    if let Some(continuity) = &adapter.continuity_args {
        let mut init = vec![continuity.init_prefix_args.clone()];
        if let Some(flag) = continuity.session_id_flag() {
            init.push(vec![flag.to_string(), "<session-id>".into()]);
        }
        init.push(prompt_tokens(adapter.prompt_flag.as_deref()));
        lines.push(line("continuity init", adapter, init));

        let mut resume = vec![continuity.resume_prefix_args.clone()];
        if let Some(flag) = continuity.resume_flag() {
            resume.push(vec![flag.to_string(), "<session-id>".into()]);
        }
        resume.push(prompt_tokens(adapter.prompt_flag.as_deref()));
        lines.push(line("continuity resume", adapter, resume));
    }

    lines
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_spec::{AdapterManifest, Capabilities, SandboxMapping};

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
            r#"claude --print --model <model> --system-prompt "<system>" "<prompt>""#
        );
        assert_eq!(
            find(&lines, "stream").command(),
            "claude -p --output-format stream-json --session-id <session-id>"
        );
        assert_eq!(
            find(&lines, "stream resume").command(),
            "claude -p --output-format stream-json --resume <session-id>"
        );
        assert_eq!(
            find(&lines, "sandbox read-only").command(),
            r#"claude --print --permission-mode plan "<prompt>""#
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
            r#"grok --output-format plain --model <model> --single "<prompt>""#
        );
        assert_eq!(
            find(&lines, "continuity init").command(),
            r#"grok --output-format plain --session-id <session-id> --single "<prompt>""#
        );
        assert_eq!(
            find(&lines, "continuity resume").command(),
            r#"grok --output-format plain --resume <session-id> --single "<prompt>""#
        );
        assert_eq!(
            find(&lines, "sandbox full").command(),
            r#"grok --output-format plain --permission-mode bypassPermissions --single "<prompt>""#
        );
        assert!(lines.iter().all(|l| l.label != "stream"));
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
        assert_eq!(lines[0].command(), r#"aria run "<prompt>""#);
        assert_eq!(lines[1].command(), r#"aria "<prompt>""#);
        // The prompt is always the final token.
        for l in &lines {
            assert_eq!(l.argv.last().unwrap(), "\"<prompt>\"");
        }
    }

    /// `model_arg_template` takes precedence over `model_flag`, matching the
    /// spec.
    #[test]
    fn model_template_wins_over_flag() {
        let a = RuntimeAdapter {
            model_flag: Some("--model".into()),
            model_arg_template: Some("-c model={model}".into()),
            ..adapter(
                r#"{ "adapters": [{ "id": "x", "label": "X", "executable": "x", "install_hint": "h" }]}"#,
            )
        };
        let one_shot = launch_preview(&a).remove(0);
        assert_eq!(one_shot.command(), r#"x -c model={model} "<prompt>""#);
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
            r#"x --single "<prompt>""#
        );

        a.interactive_prompt_flag = Some("--interactive".into());
        let lines = launch_preview(&a);
        assert_eq!(
            find(&lines, "interactive").command(),
            r#"x --interactive "<prompt>""#
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
