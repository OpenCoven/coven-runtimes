//! `conjure registry` — maintain the canonical list of accepted runtimes.
//!
//! The canonical registry is a directory of source manifests — the human-facing
//! approval surface — that compiles into a single committed index:
//!
//! ```text
//! registry/runtimes/<id>/<version>.json          ← source (accepted by merge)
//! crates/coven-runtime-registry/canonical/index.json  ← compiled, embedded, published
//! ```
//!
//! A runtime is *accepted* when its manifest is merged under `registry/runtimes/`
//! (gated by CODEOWNERS). `registry build` deterministically regenerates the
//! index from those sources; a drift-guard test asserts the committed index still
//! matches, so an edited manifest that wasn't rebuilt fails CI.

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

use anyhow::{anyhow, bail, Context, Result};
use clap::{Args, Subcommand};
use coven_runtime_registry::{RegistryEntry, RegistryIndex, INDEX_FORMAT};
use coven_runtime_spec::validate_manifest;

use super::{canonical_manifest, load_manifest, load_registry, manifest_digest};
use crate::datetime::now_iso8601;

/// Default location of the source manifests (relative to the repo root).
const DEFAULT_SOURCES: &str = "registry/runtimes";
/// Default location of the compiled canonical index (inside the registry crate,
/// so it is both `include_str!`-embeddable and packaged by `cargo publish`).
const DEFAULT_INDEX: &str = "crates/coven-runtime-registry/canonical/index.json";

#[derive(Args)]
pub struct RegistryArgs {
    #[command(subcommand)]
    command: RegistryCommand,
}

#[derive(Subcommand)]
enum RegistryCommand {
    /// Compile the canonical index from the source manifests.
    Build(BuildArgs),
    /// Verify the committed index matches the sources (non-zero exit on drift).
    Check(LocationArgs),
    /// Accept a manifest into the registry and rebuild the index.
    Add(AddArgs),
    /// List the accepted runtimes and their latest versions.
    List(ListArgs),
    /// Yank (or, with --unyank, restore) a published version.
    Yank(YankArgs),
}

#[derive(Args)]
pub struct LocationArgs {
    /// Directory of source manifests (`<id>/<version>.json`).
    #[arg(long, default_value = DEFAULT_SOURCES)]
    sources: PathBuf,
    /// Path of the compiled index.
    #[arg(long, default_value = DEFAULT_INDEX)]
    out: PathBuf,
}

#[derive(Args)]
pub struct BuildArgs {
    #[command(flatten)]
    loc: LocationArgs,
    /// Verify the committed index matches the sources instead of writing it.
    #[arg(long)]
    check: bool,
}

#[derive(Args)]
pub struct AddArgs {
    /// A one-adapter manifest (with a `version`) to accept into the registry.
    manifest: PathBuf,
    #[command(flatten)]
    loc: LocationArgs,
    /// Overwrite an existing source file for this exact version.
    #[arg(long)]
    force: bool,
}

#[derive(Args)]
pub struct ListArgs {
    /// Path of the compiled index to read.
    #[arg(long, default_value = DEFAULT_INDEX)]
    index: PathBuf,
}

#[derive(Args)]
pub struct YankArgs {
    /// Runtime id.
    id: String,
    /// Version to yank.
    version: String,
    /// Path of the compiled index to rewrite.
    #[arg(long, default_value = DEFAULT_INDEX)]
    out: PathBuf,
    /// Reverse a previous yank (mark the version installable again).
    #[arg(long)]
    unyank: bool,
}

pub fn run(args: RegistryArgs) -> Result<()> {
    match args.command {
        RegistryCommand::Build(a) => run_build(&a.loc, a.check),
        RegistryCommand::Check(loc) => run_build(&loc, true),
        RegistryCommand::Add(a) => run_add(a),
        RegistryCommand::List(a) => run_list(a),
        RegistryCommand::Yank(a) => run_yank(a),
    }
}

/// Compile the index from `loc.sources`, preserving `published_at`/`yanked` from
/// the existing `loc.out` and refusing to silently change a published version.
fn build_index(loc: &LocationArgs) -> Result<RegistryIndex> {
    let existing = if loc.out.exists() {
        load_registry(&loc.out)?
    } else {
        RegistryIndex {
            format: INDEX_FORMAT.to_string(),
            runtimes: BTreeMap::new(),
        }
    };

    let mut runtimes: BTreeMap<String, Vec<RegistryEntry>> = BTreeMap::new();

    for id_dir in sorted_dir(&loc.sources)? {
        if !id_dir.is_dir() {
            continue;
        }
        let id = file_name(&id_dir)?;

        for version_file in sorted_dir(&id_dir)? {
            if version_file.extension().and_then(|e| e.to_str()) != Some("json") {
                continue;
            }
            let version = version_file
                .file_stem()
                .and_then(|s| s.to_str())
                .ok_or_else(|| anyhow!("bad version filename {}", version_file.display()))?
                .to_string();

            let manifest = load_manifest(&version_file)?;
            let errors = validate_manifest(&manifest);
            if !errors.is_empty() {
                for e in &errors {
                    eprintln!("✗ {} — {e}", version_file.display());
                }
                bail!(
                    "{} has {} validation problem(s)",
                    version_file.display(),
                    errors.len()
                );
            }
            if manifest.adapters.len() != 1 {
                bail!(
                    "{} must contain exactly one adapter, found {}",
                    version_file.display(),
                    manifest.adapters.len()
                );
            }
            let adapter = manifest.adapters[0].clone();
            if adapter.id != id {
                bail!(
                    "{}: adapter id `{}` does not match its directory `{id}`",
                    version_file.display(),
                    adapter.id
                );
            }
            if let Some(v) = &adapter.version {
                if v != &version {
                    bail!(
                        "{}: adapter version `{v}` does not match its filename `{version}`",
                        version_file.display()
                    );
                }
            }

            let sha = manifest_digest(&manifest)?;
            // Preserve provenance from the committed index, and enforce that a
            // released (id, version) never changes content — bump the version.
            let (published_at, yanked) = match existing.resolve_exact(&id, &version).ok() {
                Some(prev) => {
                    if prev.sha256.as_deref() != Some(sha.as_str()) {
                        bail!(
                            "{id} {version} is already published with different content — \
                             bump the version instead of editing a released one \
                             (index sha {:?}, source sha {sha})",
                            prev.sha256
                        );
                    }
                    (prev.published_at.clone(), prev.yanked)
                }
                None => (Some(now_iso8601()), false),
            };

            runtimes.entry(id.clone()).or_default().push(RegistryEntry {
                version,
                adapter,
                sha256: Some(sha),
                published_at,
                yanked,
            });
        }
    }

    for entries in runtimes.values_mut() {
        entries.sort_by_key(|e| version_key(&e.version));
    }

    let index = RegistryIndex {
        format: INDEX_FORMAT.to_string(),
        runtimes,
    };

    let errors = index.validate();
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!("compiled index has {} validation problem(s)", errors.len());
    }
    Ok(index)
}

fn run_build(loc: &LocationArgs, check: bool) -> Result<()> {
    let index = build_index(loc)?;
    let serialized = format!("{}\n", index.to_json_pretty().context("serialize index")?);
    let runtimes = index.runtimes.len();
    let entries: usize = index.runtimes.values().map(Vec::len).sum();

    if check {
        if !loc.out.exists() {
            bail!(
                "no index at {} — run `conjure registry build`",
                loc.out.display()
            );
        }
        let current = fs::read_to_string(&loc.out)
            .with_context(|| format!("failed to read {}", loc.out.display()))?;
        if current != serialized {
            bail!(
                "registry index at {} is stale — run `conjure registry build` and commit the result",
                loc.out.display()
            );
        }
        println!(
            "✓ {} is up to date ({runtimes} runtime(s)).",
            loc.out.display()
        );
    } else {
        if let Some(parent) = loc.out.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("failed to create {}", parent.display()))?;
        }
        fs::write(&loc.out, &serialized)
            .with_context(|| format!("failed to write {}", loc.out.display()))?;
        println!(
            "✓ wrote {} ({runtimes} runtime(s), {entries} entr{}).",
            loc.out.display(),
            if entries == 1 { "y" } else { "ies" }
        );
    }
    Ok(())
}

fn run_add(args: AddArgs) -> Result<()> {
    let manifest = load_manifest(&args.manifest)?;
    let errors = validate_manifest(&manifest);
    if !errors.is_empty() {
        for e in &errors {
            eprintln!("✗ {e}");
        }
        bail!(
            "cannot accept an invalid manifest ({} problem(s))",
            errors.len()
        );
    }
    if manifest.adapters.len() != 1 {
        bail!(
            "registry sources hold one adapter per file; found {}",
            manifest.adapters.len()
        );
    }
    let adapter = &manifest.adapters[0];
    let version = adapter
        .version
        .clone()
        .ok_or_else(|| anyhow!("adapter must set a `version` to be accepted into the registry"))?;

    let dest_dir = args.loc.sources.join(&adapter.id);
    fs::create_dir_all(&dest_dir)
        .with_context(|| format!("failed to create {}", dest_dir.display()))?;
    let dest = dest_dir.join(format!("{version}.json"));
    if dest.exists() && !args.force {
        bail!(
            "{} already exists — bump the version or pass --force",
            dest.display()
        );
    }
    // Store the canonical form so the source and its checksum are stable.
    fs::write(&dest, canonical_manifest(&manifest)?)
        .with_context(|| format!("failed to write {}", dest.display()))?;
    println!(
        "✓ accepted {} → {}",
        args.manifest.display(),
        dest.display()
    );

    run_build(&args.loc, false)
}

fn run_list(args: ListArgs) -> Result<()> {
    let index = load_registry(&args.index)?;
    if index.runtimes.is_empty() {
        println!("(no accepted runtimes)");
        return Ok(());
    }
    for id in index.runtime_ids() {
        match index.resolve_latest(id) {
            Ok(entry) => println!(
                "{id:<16} {:<8} {}",
                entry.version,
                capability_summary(entry)
            ),
            Err(_) => println!("{id:<16} {:<8} (all versions yanked)", "-"),
        }
    }
    Ok(())
}

fn run_yank(args: YankArgs) -> Result<()> {
    let mut index = load_registry(&args.out)?;
    let entries = index
        .runtimes
        .get_mut(&args.id)
        .ok_or_else(|| anyhow!("unknown runtime `{}`", args.id))?;
    let entry = entries
        .iter_mut()
        .find(|e| e.version == args.version)
        .ok_or_else(|| anyhow!("runtime `{}` has no version `{}`", args.id, args.version))?;

    let target = !args.unyank;
    if entry.yanked == target {
        println!(
            "· {} {} is already {}.",
            args.id,
            args.version,
            if target { "yanked" } else { "installable" }
        );
        return Ok(());
    }
    entry.yanked = target;

    let serialized = format!("{}\n", index.to_json_pretty().context("serialize index")?);
    fs::write(&args.out, serialized)
        .with_context(|| format!("failed to write {}", args.out.display()))?;
    println!(
        "✓ {} {} is now {}.",
        args.id,
        args.version,
        if target { "yanked" } else { "installable" }
    );
    Ok(())
}

/// A one-line "stream, think" style capability summary, or "baseline".
fn capability_summary(entry: &RegistryEntry) -> String {
    let on: Vec<&str> = entry
        .adapter
        .capabilities
        .as_pairs()
        .iter()
        .filter(|(_, on)| *on)
        .map(|(name, _)| *name)
        .collect();
    if on.is_empty() {
        "baseline".to_string()
    } else {
        on.join(", ")
    }
}

/// Numeric sort key for `major.minor.patch`, falling back to string order.
fn version_key(v: &str) -> (u64, u64, u64, String) {
    let mut parts = v.split('.');
    let mut next = || parts.next().and_then(|s| s.parse().ok()).unwrap_or(0);
    (next(), next(), next(), v.to_string())
}

/// Directory entries as paths, sorted by file name for deterministic output.
fn sorted_dir(dir: &Path) -> Result<Vec<PathBuf>> {
    let mut paths: Vec<PathBuf> = fs::read_dir(dir)
        .with_context(|| format!("failed to read {}", dir.display()))?
        .map(|e| e.map(|e| e.path()))
        .collect::<std::result::Result<_, _>>()
        .with_context(|| format!("failed to list {}", dir.display()))?;
    paths.sort();
    Ok(paths)
}

fn file_name(path: &Path) -> Result<String> {
    Ok(path
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or_else(|| anyhow!("bad path {}", path.display()))?
        .to_string())
}
