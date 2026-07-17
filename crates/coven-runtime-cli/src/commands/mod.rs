//! `conjure` subcommand implementations.

pub mod new;
pub mod package;
pub mod registry;
pub mod test;
pub mod validate;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use coven_runtime_registry::RegistryIndex;
use coven_runtime_spec::AdapterManifest;

use crate::sha256::sha256_hex;

/// Load and parse a manifest file, with a path-tagged error on failure.
///
/// `conjure` is the authoring surface, so unlike the spec's tolerant parse it
/// rejects fields no spec version recognizes — typos must fail here, before a
/// manifest reaches the registry or a runtime host.
pub(crate) fn load_manifest(path: &Path) -> Result<AdapterManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest {}", path.display()))?;
    let value: serde_json::Value = serde_json::from_str(&raw)
        .with_context(|| format!("failed to parse manifest {}", path.display()))?;
    let unknown = coven_runtime_spec::unknown_manifest_fields(&value);
    if !unknown.is_empty() {
        // The most common shape mistake first: a registry index run as a
        // manifest must not surface as a pile of "unknown field" errors.
        if value.get("runtimes").is_some() && value.get("adapters").is_none() {
            anyhow::bail!(
                "{} looks like a registry index, not an adapter manifest; re-run with --registry",
                path.display()
            );
        }
        anyhow::bail!(
            "manifest {} contains unrecognized fields (typo?): {}",
            path.display(),
            unknown.join(", ")
        );
    }
    AdapterManifest::from_json(&raw)
        .with_context(|| format!("failed to parse manifest {}", path.display()))
}

/// Canonical bytes of a manifest: pretty JSON from the parsed model (drops
/// unknown fields, normalizes key order) plus a trailing newline. Shared by
/// `conjure package` and `conjure registry build` so a source manifest and its
/// registry entry checksum agree byte-for-byte.
pub(crate) fn canonical_manifest(manifest: &AdapterManifest) -> Result<String> {
    Ok(format!(
        "{}\n",
        manifest.to_json_pretty().context("serialize manifest")?
    ))
}

/// The lowercase-hex SHA-256 of a manifest's [`canonical_manifest`] bytes — the
/// value stored in a registry entry's `sha256` field.
pub(crate) fn manifest_digest(manifest: &AdapterManifest) -> Result<String> {
    Ok(sha256_hex(canonical_manifest(manifest)?.as_bytes()))
}

/// Load and parse a registry index file, with a path-tagged error on failure.
pub(crate) fn load_registry(path: &Path) -> Result<RegistryIndex> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read registry index {}", path.display()))?;
    RegistryIndex::from_json(&raw)
        .with_context(|| format!("failed to parse registry index {}", path.display()))
}
