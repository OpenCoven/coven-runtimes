//! # coven-runtime-spec
//!
//! The shared contract for integrating a new runtime into the Coven.
//!
//! A **runtime** (Coven calls them "harnesses" today — Codex, Claude Code,
//! Hermes) is an agent CLI that Coven drives to do work. Adding one currently
//! means either shipping a built-in Rust spec in `coven` core or hand-writing an
//! adapter `*.json` whose expressiveness stops short of the runtime's real
//! behavior: stream mode, session pre-assignment, think/speed, and sandbox
//! mapping are all decided by hardcoded `harness_id == "claude"` checks in
//! `coven`'s `harness.rs`.
//!
//! This crate is the single source of truth for the adapter **manifest** so a
//! new runtime *declares* what it can do and `coven` core *reads* it:
//!
//! - [`AdapterManifest`] / [`RuntimeAdapter`] — the JSON contract (a
//!   backward-compatible superset of coven's `ExternalHarnessAdapterSpec`).
//! - [`Capabilities`] — behavioral opt-ins (`stream`, `preassignedSessionId`,
//!   `think`, `speed`) that replace the hardcoded string checks.
//! - [`SandboxMapping`] / [`Permission`] — native permission mapping that
//!   adapters previously could not express.
//! - [`validate_manifest`] — pure validation shared by `covenrt`, the registry,
//!   and (eventually) coven's loader, so one rule set governs everywhere.
//!
//! It has no async, no I/O, and no process spawning — just types and rules — so
//! it can be a dependency of `coven` core, the `covenrt` CLI, and CI alike.

pub mod capabilities;
pub mod manifest;
pub mod sandbox;
pub mod validate;

pub use capabilities::Capabilities;
pub use manifest::{AdapterManifest, RuntimeAdapter, StreamArgs};
pub use sandbox::{Permission, SandboxMapping};
pub use validate::{validate_adapter, validate_manifest, ValidationError, BUILT_IN_IDS};

/// The manifest schema version this crate implements. Bumped on
/// backward-incompatible changes to [`RuntimeAdapter`].
pub const SCHEMA_VERSION: &str = "1";
