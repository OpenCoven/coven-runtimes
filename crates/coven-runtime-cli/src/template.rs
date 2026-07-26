//! Manifest scaffolding templates for `conjure new`.

use coven_runtime_spec::{AdapterManifest, Capabilities, ModelIdTransform, RuntimeAdapter};

/// Which starting point to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// A plain one-shot CLI runtime (baseline capabilities). Matches Codex/Hermes.
    Minimal,
    /// A streaming, session-resumable runtime with sandbox mapping. Matches Claude.
    Streaming,
    /// A one-shot runtime with native session continuity (pre-assigned ids,
    /// cold-start resume) but no stream mode — the pattern Grok Build landed
    /// with. The scaffold is a generic starting point, not a copy of any
    /// specific adapter.
    Continuity,
}

impl Flavor {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" | "basic" | "oneshot" => Ok(Flavor::Minimal),
            "streaming" | "stream" | "full" => Ok(Flavor::Streaming),
            "continuity" | "session" | "resume" => Ok(Flavor::Continuity),
            other => Err(format!(
                "unknown flavor `{other}`; expected `minimal`, `streaming` or `continuity`"
            )),
        }
    }
}

/// Build a scaffold manifest for the given adapter id + flavor.
pub fn scaffold(id: &str, flavor: Flavor) -> AdapterManifest {
    let label = title_case(id);
    let adapter = match flavor {
        Flavor::Minimal => RuntimeAdapter {
            id: id.to_string(),
            label,
            executable: id.to_string(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            install_hint: format!("Install {id}, add it to PATH, then complete its setup."),
            system_prompt_flag: None,
            model_flag: Some("--model".into()),
            model_arg_template: None,
            model_id_transform: ModelIdTransform::StripProvider,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
            continuity_args: None,
            prompt_flag: None,
            interactive_prompt_flag: None,
            version: Some("0.1.0".into()),
            homepage: None,
            description: Some(format!("{id} runtime adapter for Coven.")),
        },
        Flavor::Streaming => RuntimeAdapter {
            id: id.to_string(),
            label,
            executable: id.to_string(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["--print".into()],
            install_hint: format!("Install {id}, add it to PATH, then complete its setup."),
            system_prompt_flag: Some("--system-prompt".into()),
            model_flag: Some("--model".into()),
            model_arg_template: None,
            model_id_transform: ModelIdTransform::StripProvider,
            capabilities: Capabilities {
                stream: true,
                preassigned_session_id: true,
                think: true,
                speed: true,
            },
            sandbox: Some(coven_runtime_spec::SandboxMapping::Flag {
                flag: "--permission-mode".into(),
                full: "bypassPermissions".into(),
                read_only: "plan".into(),
            }),
            stream_args: Some(coven_runtime_spec::StreamArgs {
                prefix_args: vec![
                    "-p".into(),
                    "--input-format".into(),
                    "stream-json".into(),
                    "--output-format".into(),
                    "stream-json".into(),
                    "--verbose".into(),
                ],
                session_id_flag: Some("--session-id".into()),
                resume_flag: Some("--resume".into()),
            }),
            continuity_args: None,
            prompt_flag: None,
            interactive_prompt_flag: None,
            version: Some("0.1.0".into()),
            homepage: None,
            description: Some(format!("{id} streaming runtime adapter for Coven.")),
        },
        // The session-continuity pattern (Grok Build is the registry's
        // canonical example): every turn is a fresh process, but the runtime
        // pre-assigns session ids and resumes them via its own CLI flags.
        // Generic placeholders — swap in the runtime's real args.
        Flavor::Continuity => RuntimeAdapter {
            id: id.to_string(),
            label,
            executable: id.to_string(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["run".into()],
            install_hint: format!("Install {id}, add it to PATH, then complete its setup."),
            system_prompt_flag: None,
            model_flag: Some("--model".into()),
            model_arg_template: None,
            model_id_transform: ModelIdTransform::StripProvider,
            capabilities: Capabilities {
                stream: false,
                preassigned_session_id: true,
                think: false,
                speed: false,
            },
            sandbox: None,
            stream_args: None,
            continuity_args: Some(coven_runtime_spec::ContinuityArgs {
                init_prefix_args: vec!["run".into()],
                resume_prefix_args: vec!["run".into()],
                session_id_flag: Some("--session-id".into()),
                resume_flag: Some("--resume".into()),
            }),
            prompt_flag: None,
            interactive_prompt_flag: None,
            version: Some("0.1.0".into()),
            homepage: None,
            description: Some(format!(
                "{id} session-continuity runtime adapter for Coven."
            )),
        },
    };
    AdapterManifest {
        adapters: vec![adapter],
    }
}

/// Turn `my-runtime` / `my_runtime` into `My Runtime` for a default label.
fn title_case(id: &str) -> String {
    id.split(['-', '_', '.'])
        .filter(|s| !s.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            match chars.next() {
                Some(first) => first.to_ascii_uppercase().to_string() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_spec::validate_manifest;

    #[test]
    fn flavor_parses_aliases() {
        assert_eq!(Flavor::parse("minimal").unwrap(), Flavor::Minimal);
        assert_eq!(Flavor::parse("STREAM").unwrap(), Flavor::Streaming);
        assert_eq!(Flavor::parse("continuity").unwrap(), Flavor::Continuity);
        assert_eq!(Flavor::parse("session").unwrap(), Flavor::Continuity);
        assert!(Flavor::parse("bogus").is_err());
    }

    #[test]
    fn title_case_splits_separators() {
        assert_eq!(title_case("my-cool_runtime.v2"), "My Cool Runtime V2");
        assert_eq!(title_case("hermes"), "Hermes");
    }

    #[test]
    fn scaffold_model_transforms_default_to_strip_provider() {
        for flavor in [Flavor::Minimal, Flavor::Streaming, Flavor::Continuity] {
            assert_eq!(
                scaffold("aria", flavor).adapters[0].model_id_transform,
                ModelIdTransform::StripProvider,
                "{flavor:?}"
            );
        }
    }

    #[test]
    fn minimal_scaffold_validates_clean() {
        let m = scaffold("aria", Flavor::Minimal);
        assert!(
            validate_manifest(&m).is_empty(),
            "{:?}",
            validate_manifest(&m)
        );
        assert!(m.adapters[0].capabilities.is_baseline());
    }

    #[test]
    fn streaming_scaffold_validates_clean() {
        let m = scaffold("aria", Flavor::Streaming);
        let errs = validate_manifest(&m);
        assert!(errs.is_empty(), "{errs:?}");
        assert!(m.adapters[0].capabilities.stream);
        assert!(m.adapters[0].supports_permission());
    }

    #[test]
    fn continuity_scaffold_validates_clean() {
        let m = scaffold("aria", Flavor::Continuity);
        let errs = validate_manifest(&m);
        assert!(errs.is_empty(), "{errs:?}");
        let adapter = &m.adapters[0];
        assert!(!adapter.capabilities.stream);
        assert!(adapter.capabilities.preassigned_session_id);
        let continuity = adapter.continuity_args.as_ref().unwrap();
        assert!(continuity.has_init_launch());
        assert!(continuity.has_resume_launch());
        assert_eq!(continuity.session_id_flag(), Some("--session-id"));
    }
}
