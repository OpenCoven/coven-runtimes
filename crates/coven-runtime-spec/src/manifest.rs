//! The runtime adapter manifest — the JSON contract a new runtime ships.
//!
//! This is a superset of `coven`'s current `ExternalHarnessAdapterSpec`
//! (`harness.rs`): every field coven reads today is here with the same name and
//! the same `camelCase` serde aliases, so existing `*.json` adapters (e.g.
//! `hermes.json`) deserialize unchanged. The additions are the pieces coven
//! currently can't express in a manifest:
//!
//! - [`Capabilities`] — behavioral opt-ins that are hardcoded string checks today.
//! - [`SandboxMapping`] — permission mapping that adapters currently can't declare.
//! - [`StreamArgs`] — the stream-json launch args, required when `capabilities.stream`.
//! - registry metadata (`version`, `homepage`, `description`).

use serde::{Deserialize, Serialize};

use crate::capabilities::Capabilities;
use crate::sandbox::SandboxMapping;

/// A manifest file: a registry of one or more adapters. Matches coven's
/// `{ "adapters": [ ... ] }` envelope.
///
/// Parsing is **tolerant of unknown fields** so an index or manifest written
/// by a newer spec version still loads on older consumers (forward
/// compatibility). Authoring-time typo detection lives in
/// [`crate::validate::unknown_manifest_fields`] and the JSON Schema, which
/// `conjure` enforces.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterManifest {
    #[serde(default)]
    pub adapters: Vec<RuntimeAdapter>,
}

impl AdapterManifest {
    /// Parse a manifest from JSON text.
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    /// Serialize to pretty JSON (stable field order via struct definition).
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }
}

/// Args for the long-lived stream-json launch mode. Only meaningful when
/// [`Capabilities::stream`] is set. For Claude these are
/// `-p --input-format stream-json --output-format stream-json --verbose`.
///
/// Field naming follows the manifest convention: snake_case is canonical (so
/// coven's existing snake_case adapters parse unchanged) with camelCase aliases.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StreamArgs {
    /// argv tokens that put the runtime into persistent stream-json mode.
    #[serde(alias = "prefixArgs")]
    pub prefix_args: Vec<String>,
    /// Optional flag used to pre-assign the session id at launch
    /// (e.g. `--session-id`). Present only when
    /// [`Capabilities::preassigned_session_id`] is also set.
    #[serde(
        default,
        alias = "sessionIdFlag",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_id_flag: Option<String>,
    /// Optional flag used to resume an existing session (e.g. `--resume`).
    #[serde(default, alias = "resumeFlag", skip_serializing_if = "Option::is_none")]
    pub resume_flag: Option<String>,
}

/// Args for one-shot non-interactive session continuity: how a cold-started
/// turn initializes a named conversation or resumes an existing one via the
/// runtime CLI's own session mechanism (e.g. `--session-id` / `--resume`).
/// Mirrors `stream_args` for runtimes without a long-lived stream mode.
///
/// Field naming follows the manifest convention: snake_case is canonical with
/// camelCase aliases, matching coven's `ContinuityArgs`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContinuityArgs {
    /// argv tokens prepended when initializing a fresh named conversation.
    #[serde(default, alias = "initPrefixArgs")]
    pub init_prefix_args: Vec<String>,
    /// argv tokens prepended when resuming an existing conversation.
    #[serde(default, alias = "resumePrefixArgs")]
    pub resume_prefix_args: Vec<String>,
    /// Flag that pre-assigns the session id on a fresh launch
    /// (e.g. `--session-id`). Requires [`Capabilities::preassigned_session_id`].
    #[serde(
        default,
        alias = "sessionIdFlag",
        skip_serializing_if = "Option::is_none"
    )]
    pub session_id_flag: Option<String>,
    /// Flag that resumes an existing session (e.g. `--resume`). Omitted for
    /// runtimes whose resume id rides `resume_prefix_args` as a positional.
    #[serde(default, alias = "resumeFlag", skip_serializing_if = "Option::is_none")]
    pub resume_flag: Option<String>,
}

impl ContinuityArgs {
    /// The session-id flag, trimmed; `None` when absent or blank.
    pub fn session_id_flag(&self) -> Option<&str> {
        self.session_id_flag
            .as_deref()
            .map(str::trim)
            .filter(|flag| !flag.is_empty())
    }

    /// The resume flag, trimmed; `None` when absent or blank.
    pub fn resume_flag(&self) -> Option<&str> {
        self.resume_flag
            .as_deref()
            .map(str::trim)
            .filter(|flag| !flag.is_empty())
    }

    /// Whether these args can launch a fresh named conversation.
    pub fn has_init_launch(&self) -> bool {
        self.session_id_flag().is_some()
            || self
                .init_prefix_args
                .iter()
                .any(|arg| !arg.trim().is_empty())
    }

    /// Whether these args can resume an existing conversation.
    pub fn has_resume_launch(&self) -> bool {
        self.resume_flag().is_some()
            || self
                .resume_prefix_args
                .iter()
                .any(|arg| !arg.trim().is_empty())
    }
}

/// A single runtime adapter definition.
///
/// Field names and `camelCase` aliases match coven's `ExternalHarnessAdapterSpec`
/// so this is a drop-in superset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub struct RuntimeAdapter {
    /// Canonical id: lowercase letters, digits, `.`, `_`, `-`.
    pub id: String,
    /// Human display label, e.g. `"Hermes Agent"`.
    pub label: String,
    /// Bare executable name (no path separators, no whitespace).
    pub executable: String,

    /// argv prefix for an interactive launch (prompt appended last).
    #[serde(default, alias = "interactivePromptPrefixArgs")]
    pub interactive_prompt_prefix_args: Vec<String>,
    /// argv prefix for a one-shot non-interactive launch (prompt appended last).
    #[serde(default, alias = "nonInteractivePromptPrefixArgs")]
    pub non_interactive_prompt_prefix_args: Vec<String>,

    /// Flag that binds the one-shot prompt as `--flag=<prompt>` for runtimes
    /// with no positional prompt slot (e.g. Copilot's `--prompt`, Grok Build's
    /// `--single`). `None` means the prompt is the final positional argument.
    #[serde(default, alias = "promptFlag", skip_serializing_if = "Option::is_none")]
    pub prompt_flag: Option<String>,
    /// Flag that binds the prompt for an interactive-with-prompt launch
    /// (e.g. Copilot's `--interactive`). Falls back to `prompt_flag` semantics
    /// when absent.
    #[serde(
        default,
        alias = "interactivePromptFlag",
        skip_serializing_if = "Option::is_none"
    )]
    pub interactive_prompt_flag: Option<String>,

    /// Recovery / install guidance surfaced by `coven doctor`.
    pub install_hint: String,

    /// Flag that injects a system-prompt string (e.g. `--system-prompt`).
    /// `None` means identity is prepended to the prompt as a preamble instead.
    #[serde(
        default,
        alias = "systemPromptFlag",
        skip_serializing_if = "Option::is_none"
    )]
    pub system_prompt_flag: Option<String>,

    /// Simple `--flag <value>` model selector (e.g. `--model`).
    #[serde(default, alias = "modelFlag", skip_serializing_if = "Option::is_none")]
    pub model_flag: Option<String>,
    /// argv template for non-trivial model selection (e.g. `"-c model={model}"`).
    /// Takes precedence over `model_flag`. `{model}` is substituted per token.
    #[serde(
        default,
        alias = "modelArgTemplate",
        skip_serializing_if = "Option::is_none"
    )]
    pub model_arg_template: Option<String>,

    // ── Additions beyond coven's current manifest ────────────────────────────
    /// Behavioral capabilities. Defaults to the conservative baseline (all off).
    #[serde(default)]
    pub capabilities: Capabilities,
    /// Native sandbox/permission mapping. `None` => `coven run --permission` is
    /// a warned no-op for this runtime (today's behavior for all manifests).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sandbox: Option<SandboxMapping>,
    /// Stream-json launch args. Required when `capabilities.stream` is true.
    #[serde(default, alias = "streamArgs", skip_serializing_if = "Option::is_none")]
    pub stream_args: Option<StreamArgs>,
    /// One-shot non-interactive session-continuity args (init/resume a named
    /// conversation on a cold start). Mirrors `stream_args` for runtimes
    /// without a long-lived stream mode.
    #[serde(
        default,
        alias = "continuityArgs",
        skip_serializing_if = "Option::is_none"
    )]
    pub continuity_args: Option<ContinuityArgs>,

    // ── Registry metadata (optional; ignored by coven core) ──────────────────
    /// Semver of this adapter definition, for the registry index.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub version: Option<String>,
    /// Project homepage / docs URL.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub homepage: Option<String>,
    /// One-line description for registry listings.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

impl RuntimeAdapter {
    /// Whether this adapter declares any model-selection mechanism.
    pub fn supports_model(&self) -> bool {
        self.model_flag.is_some() || self.model_arg_template.is_some()
    }

    /// Whether this adapter declares a sandbox/permission mechanism.
    pub fn supports_permission(&self) -> bool {
        self.sandbox.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The current hermes.json shipped by coven must deserialize unchanged, with
    /// all additions defaulting to the conservative baseline.
    #[test]
    fn parses_legacy_hermes_manifest_unchanged() {
        let raw = r#"{
          "adapters": [
            {
              "id": "hermes",
              "label": "Hermes Agent",
              "executable": "hermes",
              "interactive_prompt_prefix_args": ["chat", "--source", "coven", "-q"],
              "non_interactive_prompt_prefix_args": ["chat", "--source", "coven", "-Q", "-q"],
              "install_hint": "Install Hermes Agent, add it to PATH, and complete Hermes setup before using this adapter.",
              "system_prompt_flag": null
            }
          ]
        }"#;
        let manifest = AdapterManifest::from_json(raw).expect("hermes manifest parses");
        assert_eq!(manifest.adapters.len(), 1);
        let hermes = &manifest.adapters[0];
        assert_eq!(hermes.id, "hermes");
        assert_eq!(hermes.label, "Hermes Agent");
        assert_eq!(hermes.executable, "hermes");
        assert_eq!(
            hermes.interactive_prompt_prefix_args,
            vec!["chat", "--source", "coven", "-q"]
        );
        assert!(hermes.system_prompt_flag.is_none());
        // Additions default to baseline / none.
        assert!(hermes.capabilities.is_baseline());
        assert!(hermes.sandbox.is_none());
        assert!(hermes.stream_args.is_none());
        assert!(hermes.prompt_flag.is_none());
        assert!(hermes.interactive_prompt_flag.is_none());
        assert!(hermes.continuity_args.is_none());
        assert!(!hermes.supports_model());
        assert!(!hermes.supports_permission());
    }

    #[test]
    fn accepts_camel_case_aliases() {
        let raw = r#"{
          "adapters": [{
            "id": "x", "label": "X", "executable": "x",
            "interactivePromptPrefixArgs": ["chat"],
            "nonInteractivePromptPrefixArgs": ["exec"],
            "install_hint": "hint",
            "modelFlag": "--model",
            "streamArgs": { "prefixArgs": ["-p"], "sessionIdFlag": "--session-id" }
          }]
        }"#;
        let m = AdapterManifest::from_json(raw).unwrap();
        let a = &m.adapters[0];
        assert_eq!(a.interactive_prompt_prefix_args, vec!["chat"]);
        assert_eq!(a.model_flag.as_deref(), Some("--model"));
        assert_eq!(
            a.stream_args.as_ref().unwrap().session_id_flag.as_deref(),
            Some("--session-id")
        );
    }

    /// Forward compatibility: a manifest written by a NEWER spec version (with
    /// fields this version has never heard of) must still parse rather than
    /// failing the document. Typo strictness is the authoring layer's job
    /// (`unknown_manifest_fields` + the JSON Schema via `conjure validate`).
    #[test]
    fn tolerates_fields_from_newer_spec_versions() {
        let raw = r#"{
          "adapters": [{
            "id": "x", "label": "X", "executable": "x",
            "install_hint": "hint",
            "future_field": { "from": "v9" },
            "event_protocol": "from-a-newer-spec",
            "capabilities": { "stream": false, "future_capability": true }
          }]
        }"#;
        let manifest = AdapterManifest::from_json(raw).expect("newer-spec manifest parses");
        let adapter = &manifest.adapters[0];
        assert_eq!(adapter.id, "x");
        assert!(!adapter.capabilities.stream);
    }

    /// A Copilot-shaped adapter — args-form sandbox, JSONL streaming — parses,
    /// exposes the right capability surface, and round-trips losslessly.
    #[test]
    fn copilot_shaped_adapter_round_trips() {
        let raw = r#"{
          "adapters": [{
            "id": "copilot", "label": "GitHub Copilot CLI", "executable": "copilot",
            "interactive_prompt_prefix_args": ["-i"],
            "non_interactive_prompt_prefix_args": ["-s", "-p"],
            "install_hint": "npm install -g @github/copilot",
            "model_flag": "--model",
            "capabilities": { "stream": true, "preassigned_session_id": true },
            "sandbox": { "full_args": ["--allow-all"], "read_only_args": ["--deny-tool", "write", "--deny-tool", "shell"] },
            "stream_args": { "prefix_args": ["--output-format", "json", "--stream", "on", "-p"], "session_id_flag": "--session-id", "resume_flag": "--resume" },
            "version": "1.0.0"
          }]
        }"#;
        let m = AdapterManifest::from_json(raw).unwrap();
        let a = &m.adapters[0];
        assert!(a.capabilities.stream);
        assert!(a.capabilities.preassigned_session_id);
        assert!(a.supports_model());
        assert!(a.supports_permission());
        match a.sandbox.as_ref().unwrap() {
            crate::sandbox::SandboxMapping::Args {
                full_args,
                read_only_args,
            } => {
                assert_eq!(full_args, &["--allow-all"]);
                assert_eq!(
                    read_only_args,
                    &["--deny-tool", "write", "--deny-tool", "shell"]
                );
            }
            other => panic!("expected args-form sandbox, got {other:?}"),
        }

        let reparsed = AdapterManifest::from_json(&m.to_json_pretty().unwrap()).unwrap();
        assert_eq!(m, reparsed);
    }

    /// A Grok-Build-shaped adapter — flag-bound prompt, plain one-shot output,
    /// cold-start continuity instead of stream mode — parses, exposes the right
    /// surface, and round-trips losslessly.
    #[test]
    fn grok_shaped_adapter_round_trips() {
        let raw = r#"{
          "adapters": [{
            "id": "grok", "label": "Grok Build", "executable": "grok",
            "interactive_prompt_prefix_args": ["--no-auto-update", "--no-alt-screen", "--output-format", "plain"],
            "non_interactive_prompt_prefix_args": ["--no-auto-update", "--no-alt-screen", "--output-format", "plain"],
            "install_hint": "Install Grok Build and run `grok login`.",
            "system_prompt_flag": "--rules",
            "prompt_flag": "--single",
            "interactive_prompt_flag": "--single",
            "model_flag": "--model",
            "capabilities": { "stream": false, "preassigned_session_id": true },
            "sandbox": { "full_args": ["--permission-mode", "bypassPermissions", "--sandbox", "off"], "read_only_args": ["--permission-mode", "default", "--sandbox", "read-only"] },
            "continuity_args": {
              "init_prefix_args": ["--no-auto-update", "--no-alt-screen", "--output-format", "plain"],
              "resume_prefix_args": ["--no-auto-update", "--no-alt-screen", "--output-format", "plain"],
              "session_id_flag": "--session-id",
              "resume_flag": "--resume"
            },
            "version": "1.0.0"
          }]
        }"#;
        let m = AdapterManifest::from_json(raw).unwrap();
        let a = &m.adapters[0];
        assert_eq!(a.prompt_flag.as_deref(), Some("--single"));
        assert_eq!(a.interactive_prompt_flag.as_deref(), Some("--single"));
        assert!(!a.capabilities.stream);
        assert!(a.capabilities.preassigned_session_id);
        let continuity = a.continuity_args.as_ref().unwrap();
        assert_eq!(continuity.session_id_flag(), Some("--session-id"));
        assert_eq!(continuity.resume_flag(), Some("--resume"));
        assert!(continuity.has_init_launch());
        assert!(continuity.has_resume_launch());

        let reparsed = AdapterManifest::from_json(&m.to_json_pretty().unwrap()).unwrap();
        assert_eq!(m, reparsed);
    }

    #[test]
    fn accepts_camel_case_aliases_for_continuity() {
        let raw = r#"{
          "adapters": [{
            "id": "x", "label": "X", "executable": "x",
            "install_hint": "hint",
            "promptFlag": "--single",
            "interactivePromptFlag": "--single",
            "capabilities": { "preassignedSessionId": true },
            "continuityArgs": { "initPrefixArgs": ["run"], "sessionIdFlag": "--session-id", "resumeFlag": "--resume" }
          }]
        }"#;
        let m = AdapterManifest::from_json(raw).unwrap();
        let a = &m.adapters[0];
        assert_eq!(a.prompt_flag.as_deref(), Some("--single"));
        assert_eq!(a.interactive_prompt_flag.as_deref(), Some("--single"));
        let continuity = a.continuity_args.as_ref().unwrap();
        assert_eq!(continuity.init_prefix_args, vec!["run"]);
        assert_eq!(continuity.session_id_flag(), Some("--session-id"));
        assert_eq!(continuity.resume_flag(), Some("--resume"));
    }

    #[test]
    fn continuity_launch_predicates_ignore_blank_tokens() {
        let blank = ContinuityArgs {
            init_prefix_args: vec!["  ".into()],
            resume_prefix_args: vec![],
            session_id_flag: Some("  ".into()),
            resume_flag: None,
        };
        assert!(blank.session_id_flag().is_none());
        assert!(!blank.has_init_launch());
        assert!(!blank.has_resume_launch());

        let resume_positional = ContinuityArgs {
            init_prefix_args: vec![],
            resume_prefix_args: vec!["exec".into(), "resume".into()],
            session_id_flag: None,
            resume_flag: None,
        };
        assert!(!resume_positional.has_init_launch());
        assert!(resume_positional.has_resume_launch());
    }

    #[test]
    fn full_adapter_round_trips() {
        let raw = r#"{
          "adapters": [{
            "id": "claude", "label": "Claude Code", "executable": "claude",
            "interactive_prompt_prefix_args": [],
            "non_interactive_prompt_prefix_args": ["--print"],
            "install_hint": "npm install -g @anthropic-ai/claude-code",
            "system_prompt_flag": "--system-prompt",
            "model_flag": "--model",
            "capabilities": { "stream": true, "preassignedSessionId": true, "think": true, "speed": true },
            "sandbox": { "flag": "--permission-mode", "full": "bypassPermissions", "readOnly": "plan" },
            "stream_args": { "prefix_args": ["-p", "--input-format", "stream-json", "--output-format", "stream-json", "--verbose"], "session_id_flag": "--session-id", "resume_flag": "--resume" },
            "version": "1.0.0"
          }]
        }"#;
        let m = AdapterManifest::from_json(raw).unwrap();
        let a = &m.adapters[0];
        assert!(a.capabilities.stream);
        assert!(a.capabilities.preassigned_session_id);
        assert!(a.supports_permission());
        assert_eq!(a.version.as_deref(), Some("1.0.0"));

        // Round-trip through pretty JSON and back is lossless.
        let reparsed = AdapterManifest::from_json(&m.to_json_pretty().unwrap()).unwrap();
        assert_eq!(m, reparsed);
    }
}
