//! `conjure` subcommand implementations.

pub mod new;
pub mod package;
pub mod test;
pub mod validate;

use std::fs;
use std::path::Path;

use anyhow::{Context, Result};
use coven_runtime_registry::RegistryIndex;
use coven_runtime_spec::AdapterManifest;

/// Load and parse a manifest file, with a path-tagged error on failure.
pub(crate) fn load_manifest(path: &Path) -> Result<AdapterManifest> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read manifest {}", path.display()))?;
    AdapterManifest::from_json(&raw)
        .with_context(|| format!("failed to parse manifest {}", path.display()))
}

/// Load and parse a registry index file, with a path-tagged error on failure.
pub(crate) fn load_registry(path: &Path) -> Result<RegistryIndex> {
    let raw = fs::read_to_string(path)
        .with_context(|| format!("failed to read registry index {}", path.display()))?;
    RegistryIndex::from_json(&raw)
        .with_context(|| format!("failed to parse registry index {}", path.display()))
}
