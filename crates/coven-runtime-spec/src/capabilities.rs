//! Behavioral capability model for a Coven runtime.
//!
//! Today `coven` core decides these with hardcoded `harness_id == "claude"`
//! string checks scattered across `harness.rs` (`harness_supports_stream_mode`,
//! `harness_supports_preassigned_session_id`, `harness_supports_think`,
//! `harness_supports_speed`). That means teaching Coven about a *new* streaming
//! runtime requires editing core Rust.
//!
//! This type moves those decisions into the adapter manifest so a new runtime
//! declares what it can do and core reads it. The field set is a 1:1 mirror of
//! the existing `harness_supports_*` predicates, so the core refactor is a
//! mechanical swap: `harness_supports_stream_mode(id)` becomes
//! `spec.capabilities.stream`.

use serde::{Deserialize, Serialize};

/// What a runtime can do beyond the baseline one-shot prompt.
///
/// Every field defaults to `false` — the conservative baseline that matches a
/// plain non-interactive CLI (like Codex today). A runtime opts in only to what
/// it actually supports, and [`crate::validate`] cross-checks those claims
/// against the rest of the manifest (e.g. `stream` requires stream args).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(default)]
pub struct Capabilities {
    /// Long-lived stream-json process: stdin reads newline-delimited JSON
    /// messages, stdout writes newline-delimited JSON events. Mirrors
    /// `harness_supports_stream_mode`. Requires [`crate::manifest::StreamArgs`].
    pub stream: bool,

    /// The runtime lets the caller pre-assign a session id at launch
    /// (e.g. `claude --session-id <uuid>`). Runtimes that auto-generate ids
    /// (like Codex) leave this `false`; the chat layer captures the id from the
    /// first turn's output instead. Mirrors
    /// `harness_supports_preassigned_session_id`. Accepts both snake_case and
    /// camelCase (`preassignedSessionId`) spellings.
    #[serde(alias = "preassignedSessionId")]
    pub preassigned_session_id: bool,

    /// The runtime supports an extended-thinking / high-effort toggle.
    /// Mirrors `harness_supports_think`.
    pub think: bool,

    /// The runtime supports a fast/balanced/thorough speed selection.
    /// Mirrors `harness_supports_speed`.
    pub speed: bool,
}

impl Capabilities {
    /// The conservative baseline (all `false`) — a plain one-shot CLI.
    pub const BASELINE: Self = Self {
        stream: false,
        preassigned_session_id: false,
        think: false,
        speed: false,
    };

    /// True if the runtime declares at least one non-baseline capability.
    pub fn is_baseline(self) -> bool {
        self == Self::BASELINE
    }

    /// Stable list of `(name, enabled)` pairs, for diagnostics / `covenrt`
    /// output. Order is stable so snapshots and diffs are deterministic.
    pub fn as_pairs(self) -> [(&'static str, bool); 4] {
        [
            ("stream", self.stream),
            ("preassignedSessionId", self.preassigned_session_id),
            ("think", self.think),
            ("speed", self.speed),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_is_baseline() {
        assert!(Capabilities::default().is_baseline());
        assert_eq!(Capabilities::default(), Capabilities::BASELINE);
    }

    #[test]
    fn deserializes_camel_case_and_defaults_missing() {
        // Only `stream` declared; the rest default to false.
        let caps: Capabilities = serde_json::from_str(r#"{ "stream": true }"#).unwrap();
        assert!(caps.stream);
        assert!(!caps.preassigned_session_id);
        assert!(!caps.think);
        assert!(!caps.speed);
        assert!(!caps.is_baseline());
    }

    #[test]
    fn accepts_preassigned_session_id_camel_case() {
        let caps: Capabilities =
            serde_json::from_str(r#"{ "preassignedSessionId": true }"#).unwrap();
        assert!(caps.preassigned_session_id);
    }

    #[test]
    fn empty_object_is_baseline() {
        let caps: Capabilities = serde_json::from_str("{}").unwrap();
        assert!(caps.is_baseline());
    }

    #[test]
    fn pairs_are_stable_order() {
        let names: Vec<&str> = Capabilities::BASELINE
            .as_pairs()
            .iter()
            .map(|(n, _)| *n)
            .collect();
        assert_eq!(names, ["stream", "preassignedSessionId", "think", "speed"]);
    }
}
