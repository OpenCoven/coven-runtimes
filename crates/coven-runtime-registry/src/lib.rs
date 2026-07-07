//! # coven-runtime-registry
//!
//! A versioned catalog for distributing Coven runtime adapters, so
//! `coven adapter install <name>` (and `conjure`) can resolve a runtime by id
//! and version instead of everyone hand-copying `*.json` files.
//!
//! The registry is a single JSON document ([`RegistryIndex`]) that maps each
//! runtime id to one or more published [`RegistryEntry`] versions. Each entry
//! embeds the full [`RuntimeAdapter`] manifest plus distribution metadata
//! (checksum, publish date, yank status), so resolving a runtime yields
//! something [`coven_runtime_spec::validate_adapter`] can check immediately.
//!
//! Resolution is pure and offline: you load an index (from a file, an HTTP
//! fetch, or a bundled copy) and query it. Fetching/transport lives in the
//! caller.

use std::collections::BTreeMap;

use coven_runtime_spec::{validate_adapter, RuntimeAdapter, ValidationError};
use serde::{Deserialize, Serialize};

/// The registry index format version.
pub const INDEX_FORMAT: &str = "1";

/// The canonical registry of accepted runtimes, embedded at compile time.
///
/// These are the exact bytes of `canonical/index.json`, regenerated from
/// `registry/runtimes/**` by `conjure registry build` and kept in sync by a
/// drift-guard test. The same bytes are published as a release asset for
/// non-Rust consumers, so the embedded copy and the downloadable one never
/// disagree. Prefer [`RegistryIndex::canonical`] over parsing this yourself.
pub const CANONICAL_INDEX_JSON: &str = include_str!("../canonical/index.json");

/// Top-level registry document.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryIndex {
    /// Index format version (see [`INDEX_FORMAT`]).
    #[serde(default = "default_format")]
    pub format: String,
    /// Runtimes keyed by canonical id. `BTreeMap` keeps listings deterministic.
    #[serde(default)]
    pub runtimes: BTreeMap<String, Vec<RegistryEntry>>,
}

fn default_format() -> String {
    INDEX_FORMAT.to_string()
}

/// One published version of a runtime adapter.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RegistryEntry {
    /// Semver of this published adapter.
    pub version: String,
    /// The full adapter manifest for this version.
    pub adapter: RuntimeAdapter,
    /// Optional SHA-256 (hex) of the canonical manifest bytes, for integrity.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub sha256: Option<String>,
    /// ISO-8601 publish timestamp.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub published_at: Option<String>,
    /// When set, this version is yanked: resolvable by exact version, but never
    /// selected by "latest".
    #[serde(default, skip_serializing_if = "is_false")]
    pub yanked: bool,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// Why a resolution failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ResolveError {
    #[error("unknown runtime `{0}`")]
    UnknownRuntime(String),
    #[error("runtime `{runtime}` has no version `{version}`")]
    UnknownVersion { runtime: String, version: String },
    #[error("runtime `{0}` has no installable (non-yanked) versions")]
    NoInstallableVersions(String),
    #[error("version `{0}` is not valid semver")]
    BadSemver(String),
}

impl RegistryIndex {
    /// The canonical, accepted registry embedded in this crate — the list a
    /// downstream Rust consumer (e.g. `coven` core) resolves against, pinned by
    /// this crate's version.
    ///
    /// Infallible: the embedded [`CANONICAL_INDEX_JSON`] is guaranteed to parse
    /// by the `canonical_index_*` tests, which run in CI before any release.
    pub fn canonical() -> Self {
        Self::from_json(CANONICAL_INDEX_JSON)
            .expect("embedded canonical index must be valid JSON (guarded by tests)")
    }

    /// Parse an index from JSON text.
    pub fn from_json(raw: &str) -> Result<Self, serde_json::Error> {
        serde_json::from_str(raw)
    }

    /// Serialize to pretty JSON.
    pub fn to_json_pretty(&self) -> Result<String, serde_json::Error> {
        serde_json::to_string_pretty(self)
    }

    /// All runtime ids, sorted.
    pub fn runtime_ids(&self) -> Vec<&str> {
        self.runtimes.keys().map(String::as_str).collect()
    }

    /// Resolve the latest non-yanked version of a runtime by semver ordering.
    pub fn resolve_latest(&self, runtime: &str) -> Result<&RegistryEntry, ResolveError> {
        let entries = self
            .runtimes
            .get(runtime)
            .ok_or_else(|| ResolveError::UnknownRuntime(runtime.to_string()))?;

        let mut best: Option<(&RegistryEntry, SemVer)> = None;
        for entry in entries.iter().filter(|e| !e.yanked) {
            let parsed = SemVer::parse(&entry.version)
                .ok_or_else(|| ResolveError::BadSemver(entry.version.clone()))?;
            match &best {
                Some((_, cur)) if *cur >= parsed => {}
                _ => best = Some((entry, parsed)),
            }
        }
        best.map(|(e, _)| e)
            .ok_or_else(|| ResolveError::NoInstallableVersions(runtime.to_string()))
    }

    /// Resolve an exact version (yanked versions are still resolvable by exact
    /// pin — yank only excludes them from "latest").
    pub fn resolve_exact(
        &self,
        runtime: &str,
        version: &str,
    ) -> Result<&RegistryEntry, ResolveError> {
        let entries = self
            .runtimes
            .get(runtime)
            .ok_or_else(|| ResolveError::UnknownRuntime(runtime.to_string()))?;
        entries
            .iter()
            .find(|e| e.version == version)
            .ok_or_else(|| ResolveError::UnknownVersion {
                runtime: runtime.to_string(),
                version: version.to_string(),
            })
    }

    /// Validate every adapter in the index (delegates to the spec's rules) and
    /// additionally require each entry's `adapter.id` to match its runtime key.
    /// Returns all problems across the whole index.
    pub fn validate(&self) -> Vec<ValidationError> {
        let mut errors = Vec::new();
        for (runtime_id, entries) in &self.runtimes {
            for entry in entries {
                if &entry.adapter.id != runtime_id {
                    errors.push(ValidationError {
                        adapter_id: Some(entry.adapter.id.clone()),
                        field: "id",
                        message: format!("adapter id does not match registry key `{runtime_id}`"),
                    });
                }
                errors.extend(validate_adapter(&entry.adapter));
            }
        }
        errors
    }
}

/// Minimal semver (major.minor.patch) for ordering registry versions.
/// Pre-release / build metadata is not supported yet; such versions fail to
/// parse and surface as [`ResolveError::BadSemver`] rather than sorting wrong.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct SemVer {
    major: u64,
    minor: u64,
    patch: u64,
}

impl SemVer {
    fn parse(s: &str) -> Option<Self> {
        let mut parts = s.trim().split('.');
        let major = parts.next()?.parse().ok()?;
        let minor = parts.next()?.parse().ok()?;
        let patch = parts.next()?.parse().ok()?;
        if parts.next().is_some() {
            return None; // more than 3 segments
        }
        Some(SemVer {
            major,
            minor,
            patch,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coven_runtime_spec::Capabilities;

    fn adapter(id: &str) -> RuntimeAdapter {
        RuntimeAdapter {
            id: id.into(),
            label: "R".into(),
            executable: id.into(),
            interactive_prompt_prefix_args: vec![],
            non_interactive_prompt_prefix_args: vec!["exec".into()],
            install_hint: "install".into(),
            system_prompt_flag: None,
            model_flag: None,
            model_arg_template: None,
            capabilities: Capabilities::BASELINE,
            sandbox: None,
            stream_args: None,
            version: None,
            homepage: None,
            description: None,
        }
    }

    fn entry(id: &str, version: &str, yanked: bool) -> RegistryEntry {
        RegistryEntry {
            version: version.into(),
            adapter: adapter(id),
            sha256: None,
            published_at: None,
            yanked,
        }
    }

    fn index_with(id: &str, entries: Vec<RegistryEntry>) -> RegistryIndex {
        let mut runtimes = BTreeMap::new();
        runtimes.insert(id.to_string(), entries);
        RegistryIndex {
            format: INDEX_FORMAT.into(),
            runtimes,
        }
    }

    #[test]
    fn resolve_latest_picks_highest_semver() {
        let idx = index_with(
            "aria",
            vec![
                entry("aria", "0.9.0", false),
                entry("aria", "1.2.0", false),
                entry("aria", "1.10.0", false), // must beat 1.2.0 numerically
            ],
        );
        let latest = idx.resolve_latest("aria").unwrap();
        assert_eq!(latest.version, "1.10.0");
    }

    #[test]
    fn resolve_latest_skips_yanked() {
        let idx = index_with(
            "aria",
            vec![
                entry("aria", "1.0.0", false),
                entry("aria", "2.0.0", true), // yanked, must be skipped
            ],
        );
        assert_eq!(idx.resolve_latest("aria").unwrap().version, "1.0.0");
    }

    #[test]
    fn resolve_exact_finds_yanked() {
        let idx = index_with("aria", vec![entry("aria", "2.0.0", true)]);
        // Latest refuses (only version is yanked)…
        assert!(matches!(
            idx.resolve_latest("aria"),
            Err(ResolveError::NoInstallableVersions(_))
        ));
        // …but an exact pin still resolves it.
        assert_eq!(idx.resolve_exact("aria", "2.0.0").unwrap().version, "2.0.0");
    }

    #[test]
    fn unknown_runtime_and_version_errors() {
        let idx = index_with("aria", vec![entry("aria", "1.0.0", false)]);
        assert!(matches!(
            idx.resolve_latest("nope"),
            Err(ResolveError::UnknownRuntime(_))
        ));
        assert!(matches!(
            idx.resolve_exact("aria", "9.9.9"),
            Err(ResolveError::UnknownVersion { .. })
        ));
    }

    #[test]
    fn bad_semver_surfaces_as_error() {
        let idx = index_with("aria", vec![entry("aria", "not-semver", false)]);
        assert!(matches!(
            idx.resolve_latest("aria"),
            Err(ResolveError::BadSemver(_))
        ));
    }

    #[test]
    fn validate_flags_id_key_mismatch() {
        let idx = index_with("aria", vec![entry("mismatch", "1.0.0", false)]);
        let errs = idx.validate();
        assert!(errs
            .iter()
            .any(|e| e.field == "id" && e.message.contains("does not match")));
    }

    #[test]
    fn index_round_trips_json() {
        let idx = index_with("aria", vec![entry("aria", "1.0.0", false)]);
        let reparsed = RegistryIndex::from_json(&idx.to_json_pretty().unwrap()).unwrap();
        assert_eq!(idx, reparsed);
    }

    #[test]
    fn format_defaults_when_missing() {
        let idx = RegistryIndex::from_json(r#"{ "runtimes": {} }"#).unwrap();
        assert_eq!(idx.format, INDEX_FORMAT);
    }

    // The embedded canonical index (the published, accepted list) must always be
    // loadable and internally consistent — this guards the `canonical()`
    // `expect`, so a malformed index fails `cargo test`, never a consumer.
    #[test]
    fn canonical_index_parses_and_validates() {
        let idx = RegistryIndex::canonical();
        assert_eq!(idx.format, INDEX_FORMAT);
        assert!(!idx.runtimes.is_empty(), "canonical index is empty");
        let errors = idx.validate();
        assert!(errors.is_empty(), "canonical index invalid: {errors:?}");
    }

    #[test]
    fn canonical_index_resolves_seeded_runtimes() {
        let idx = RegistryIndex::canonical();
        // The seeded accepted runtimes must resolve by "latest".
        assert!(idx.resolve_latest("hermes").is_ok());
        assert!(idx.resolve_latest("copilot").is_ok());
    }
}
