//! Manifest scaffolding templates for `conjure new`.

use coven_runtime_spec::{AdapterManifest, Capabilities, RuntimeAdapter};

/// Which starting point to scaffold.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flavor {
    /// A plain one-shot CLI runtime (baseline capabilities). Matches Codex/Hermes.
    Minimal,
    /// A streaming, session-resumable runtime with sandbox mapping. Matches Claude.
    Streaming,
}

impl Flavor {
    pub fn parse(s: &str) -> Result<Self, String> {
        match s.trim().to_ascii_lowercase().as_str() {
            "minimal" | "basic" | "oneshot" => Ok(Flavor::Minimal),
            "streaming" | "stream" | "full" => Ok(Flavor::Streaming),
            other => Err(format!(
                "unknown flavor `{other}`; expected `minimal` or `streaming`"
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
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
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
            version: Some("0.1.0".into()),
            homepage: None,
            description: Some(format!("{id} streaming runtime adapter for Coven.")),
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
        assert!(Flavor::parse("bogus").is_err());
    }

    #[test]
    fn title_case_splits_separators() {
        assert_eq!(title_case("my-cool_runtime.v2"), "My Cool Runtime V2");
        assert_eq!(title_case("hermes"), "Hermes");
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
}
