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
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
#[serde(deny_unknown_fields)]
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

/// A single runtime adapter definition.
///
/// Field names and `camelCase` aliases match coven's `ExternalHarnessAdapterSpec`
/// so this is a drop-in superset.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case", deny_unknown_fields)]
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

    #[test]
    fn rejects_unknown_manifest_fields() {
        let raw = r#"{
          "adapters": [{
            "id": "x", "label": "X", "executable": "x",
            "install_hint": "hint",
            "capabilties": { "stream": true }
          }]
        }"#;
        let err = AdapterManifest::from_json(raw).unwrap_err().to_string();
        assert!(err.contains("unknown field"), "{err}");
        assert!(err.contains("capabilties"), "{err}");
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
