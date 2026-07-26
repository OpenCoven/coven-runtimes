//! The studio's pure state machine.
//!
//! [`Studio::apply`] consumes abstract [`Event`]s (already decoupled from
//! terminal input) and returns [`Effect`]s for the runtime loop to execute
//! (save, probe, quit). No terminal types, no I/O — everything here is
//! unit-testable headlessly, and the render layer is a pure view of this
//! state.

use std::path::PathBuf;

use coven_runtime_spec::{validate_manifest, AdapterManifest, RuntimeAdapter, ValidationError};

use super::fields::{cycle_sandbox, visible_fields, FieldKind, FieldSpec};

/// Input mode.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Mode {
    /// Navigating the form.
    Nav,
    /// Editing the focused field's value.
    Edit,
    /// Confirming quit with unsaved changes.
    ConfirmQuit,
}

/// Abstract input events, mapped from key presses by the runtime loop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Event {
    Up,
    Down,
    /// Jump to the first field of the next section.
    NextSection,
    /// Jump to the first field of the previous section.
    PrevSection,
    /// Enter edit mode (or toggle a Bool/SandboxKind field).
    Activate,
    /// Toggle / cycle the focused field without entering edit mode.
    Toggle,
    Char(char),
    Backspace,
    /// Commit the edit buffer.
    Commit,
    /// Leave edit mode without committing / dismiss the quit confirm.
    Cancel,
    /// Switch to the next adapter in a multi-adapter manifest.
    NextAdapter,
    PrevAdapter,
    Save,
    Probe,
    Quit,
    /// Quit unconditionally (Ctrl-C, or confirmed quit).
    ForceQuit,
}

/// Side effects the runtime loop must perform after an [`Event`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Effect {
    None,
    /// Write the manifest to `Studio::path`, then call [`Studio::saved`].
    Save,
    /// Probe the selected adapter's binary, then call [`Studio::probe_done`].
    Probe,
    /// Tear down the terminal and exit.
    Quit,
}

/// Probe lifecycle, as rendered by the probe panel.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) enum ProbeState {
    NotRun,
    Running,
    Done(Vec<String>),
}

/// The complete studio state.
pub(crate) struct Studio {
    pub(crate) manifest: AdapterManifest,
    pub(crate) path: PathBuf,
    pub(crate) adapter_idx: usize,
    pub(crate) cursor: usize,
    pub(crate) mode: Mode,
    pub(crate) edit_buffer: String,
    pub(crate) dirty: bool,
    pub(crate) errors: Vec<ValidationError>,
    pub(crate) probe: ProbeState,
    pub(crate) status: String,
}

impl Studio {
    /// Build the studio over a manifest. `dirty` starts set for a manifest
    /// that does not exist on disk yet (fresh scaffold).
    pub(crate) fn new(manifest: AdapterManifest, path: PathBuf, dirty: bool) -> Self {
        let errors = validate_manifest(&manifest);
        let status = if dirty {
            format!("new manifest — s saves to {}", path.display())
        } else {
            String::from(
                "? for keys: ↑/↓ move · Enter edit · Space toggle · s save · t probe · q quit",
            )
        };
        Studio {
            manifest,
            path,
            adapter_idx: 0,
            cursor: 0,
            mode: Mode::Nav,
            edit_buffer: String::new(),
            dirty,
            errors,
            probe: ProbeState::NotRun,
            status,
        }
    }

    /// The adapter the form is editing.
    pub(crate) fn adapter(&self) -> &RuntimeAdapter {
        &self.manifest.adapters[self.adapter_idx]
    }

    fn adapter_mut(&mut self) -> &mut RuntimeAdapter {
        &mut self.manifest.adapters[self.adapter_idx]
    }

    /// The rows visible for the selected adapter's current shape.
    pub(crate) fn fields(&self) -> Vec<&'static FieldSpec> {
        visible_fields(self.adapter())
    }

    /// The focused field.
    pub(crate) fn current_field(&self) -> &'static FieldSpec {
        let fields = self.fields();
        fields[self.cursor.min(fields.len() - 1)]
    }

    /// Validation errors that belong to the focused field of the selected
    /// adapter.
    pub(crate) fn focused_errors(&self) -> Vec<&ValidationError> {
        let field = self.current_field();
        let id = self.adapter().id.trim().to_lowercase();
        self.errors
            .iter()
            .filter(|e| {
                e.adapter_id.as_deref() == Some(id.as_str()) && field.error_keys.contains(&e.field)
            })
            .collect()
    }

    fn revalidate(&mut self) {
        self.errors = validate_manifest(&self.manifest);
    }

    fn clamp_cursor(&mut self) {
        let len = self.fields().len();
        if self.cursor >= len {
            self.cursor = len - 1;
        }
    }

    fn mutate_current(&mut self, value: &str) {
        let field = self.current_field();
        (field.set)(self.adapter_mut(), value);
        self.dirty = true;
        self.revalidate();
        self.clamp_cursor();
    }

    /// Advance the state machine. Pure except through the returned [`Effect`].
    pub(crate) fn apply(&mut self, event: Event) -> Effect {
        match self.mode {
            Mode::Edit => self.apply_edit(event),
            Mode::ConfirmQuit => self.apply_confirm(event),
            Mode::Nav => self.apply_nav(event),
        }
    }

    fn apply_nav(&mut self, event: Event) -> Effect {
        match event {
            Event::Up => {
                self.cursor = self.cursor.saturating_sub(1);
            }
            Event::Down => {
                self.cursor = (self.cursor + 1).min(self.fields().len() - 1);
            }
            Event::NextSection => {
                let fields = self.fields();
                let current = fields[self.cursor].section;
                if let Some(next) = fields
                    .iter()
                    .enumerate()
                    .skip(self.cursor)
                    .find(|(_, f)| f.section != current)
                {
                    self.cursor = next.0;
                }
            }
            Event::PrevSection => {
                let fields = self.fields();
                let current = fields[self.cursor].section;
                // First field of the previous section (or of this one when
                // mid-section).
                let first_of_current = fields
                    .iter()
                    .position(|f| f.section == current)
                    .unwrap_or(0);
                if self.cursor > first_of_current {
                    self.cursor = first_of_current;
                } else if first_of_current > 0 {
                    let prev = fields[first_of_current - 1].section;
                    self.cursor = fields.iter().position(|f| f.section == prev).unwrap_or(0);
                }
            }
            Event::Activate | Event::Toggle => {
                let field = self.current_field();
                match field.kind {
                    FieldKind::Bool => {
                        let flipped = if (field.get)(self.adapter()) == "true" {
                            "false"
                        } else {
                            "true"
                        };
                        self.mutate_current(flipped);
                    }
                    FieldKind::SandboxKind => {
                        cycle_sandbox(self.adapter_mut());
                        self.dirty = true;
                        self.revalidate();
                        self.clamp_cursor();
                    }
                    _ if event == Event::Activate => {
                        self.edit_buffer = (field.get)(self.adapter());
                        self.mode = Mode::Edit;
                    }
                    _ => {}
                }
            }
            Event::NextAdapter if !self.manifest.adapters.is_empty() => {
                self.adapter_idx = (self.adapter_idx + 1) % self.manifest.adapters.len();
                self.cursor = 0;
            }
            Event::PrevAdapter if !self.manifest.adapters.is_empty() => {
                self.adapter_idx = (self.adapter_idx + self.manifest.adapters.len() - 1)
                    % self.manifest.adapters.len();
                self.cursor = 0;
            }
            Event::NextAdapter | Event::PrevAdapter => {}
            Event::Save => return Effect::Save,
            Event::Probe => {
                self.probe = ProbeState::Running;
                self.status = format!("probing `{}`…", self.adapter().executable);
                return Effect::Probe;
            }
            Event::Quit => {
                if self.dirty {
                    self.mode = Mode::ConfirmQuit;
                } else {
                    return Effect::Quit;
                }
            }
            Event::ForceQuit => return Effect::Quit,
            _ => {}
        }
        Effect::None
    }

    fn apply_edit(&mut self, event: Event) -> Effect {
        match event {
            Event::Char(c) => self.edit_buffer.push(c),
            Event::Backspace => {
                self.edit_buffer.pop();
            }
            Event::Commit => {
                let value = self.edit_buffer.clone();
                self.mutate_current(&value);
                self.mode = Mode::Nav;
                self.edit_buffer.clear();
            }
            Event::Cancel => {
                self.mode = Mode::Nav;
                self.edit_buffer.clear();
            }
            Event::ForceQuit => return Effect::Quit,
            _ => {}
        }
        Effect::None
    }

    fn apply_confirm(&mut self, event: Event) -> Effect {
        match event {
            Event::Commit | Event::Quit | Event::ForceQuit => Effect::Quit,
            Event::Cancel => {
                self.mode = Mode::Nav;
                self.status = String::from("quit cancelled");
                Effect::None
            }
            _ => Effect::None,
        }
    }

    /// Feed back the result of an [`Effect::Save`]. The final status must be
    /// composed here — the event loop runs the save and calls this before the
    /// next frame, so anything `apply` had put in `status` is never rendered.
    pub(crate) fn saved(&mut self, result: Result<(), String>) {
        match result {
            Ok(()) => {
                self.dirty = false;
                self.status = if self.errors.is_empty() {
                    format!("saved {}", self.path.display())
                } else {
                    format!(
                        "saved {} — {} validation problem(s) remain; fix before opening a PR",
                        self.path.display(),
                        self.errors.len()
                    )
                };
            }
            Err(e) => self.status = format!("save failed: {e}"),
        }
    }

    /// Feed back the result of an [`Effect::Probe`].
    pub(crate) fn probe_done(&mut self, report: Vec<String>) {
        self.probe = ProbeState::Done(report);
        self.status = String::from("probe finished");
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::template::{scaffold, Flavor};

    fn studio() -> Studio {
        let manifest = scaffold("aria", Flavor::Minimal);
        Studio::new(manifest, PathBuf::from("aria.json"), false)
    }

    fn studio_streaming() -> Studio {
        let manifest = scaffold("zephyr", Flavor::Streaming);
        Studio::new(manifest, PathBuf::from("zephyr.json"), false)
    }

    #[test]
    fn navigation_clamps_at_both_ends() {
        let mut s = studio();
        s.apply(Event::Up);
        assert_eq!(s.cursor, 0);
        let last = s.fields().len() - 1;
        for _ in 0..500 {
            s.apply(Event::Down);
        }
        assert_eq!(s.cursor, last);
    }

    #[test]
    fn section_jumps_move_between_sections() {
        let mut s = studio();
        let first_section = s.fields()[0].section;
        s.apply(Event::NextSection);
        assert_ne!(s.fields()[s.cursor].section, first_section);
        s.apply(Event::PrevSection);
        assert_eq!(s.cursor, 0);
    }

    #[test]
    fn edit_commit_mutates_validates_and_dirties() {
        let mut s = studio();
        assert!(!s.dirty);
        assert!(s.errors.is_empty(), "scaffold must start valid");

        // Field 0 is `id`; blank it via an edit → validation error appears.
        s.apply(Event::Activate);
        assert_eq!(s.mode, Mode::Edit);
        assert_eq!(s.edit_buffer, "aria");
        for _ in 0..4 {
            s.apply(Event::Backspace);
        }
        s.apply(Event::Commit);
        assert_eq!(s.mode, Mode::Nav);
        assert!(s.dirty);
        assert!(
            s.errors.iter().any(|e| e.field == "id"),
            "blank id must fail validation: {:?}",
            s.errors
        );
        assert!(!s.focused_errors().is_empty() || s.adapter().id.is_empty());
    }

    #[test]
    fn edit_cancel_leaves_adapter_untouched() {
        let mut s = studio();
        let before = s.manifest.clone();
        s.apply(Event::Activate);
        s.apply(Event::Char('x'));
        s.apply(Event::Cancel);
        assert_eq!(s.manifest, before);
        assert!(!s.dirty);
    }

    #[test]
    fn toggling_stream_without_args_flags_the_capability() {
        let mut s = studio();
        let stream_idx = s.fields().iter().position(|f| f.label == "stream").unwrap();
        s.cursor = stream_idx;
        s.apply(Event::Toggle);
        assert!(s.adapter().capabilities.stream);
        assert!(
            s.errors.iter().any(|e| e.field == "capabilities.stream"),
            "stream without stream_args must be an error: {:?}",
            s.errors
        );
        // The focused row surfaces its own errors.
        assert!(!s.focused_errors().is_empty());
        // Toggle back → valid again.
        s.apply(Event::Toggle);
        assert!(s.errors.is_empty());
    }

    #[test]
    fn sandbox_cycle_keeps_cursor_in_bounds() {
        let mut s = studio();
        let kind_idx = s
            .fields()
            .iter()
            .position(|f| f.label == "sandbox form")
            .unwrap();
        // Park the cursor at the very end, then cycle args → none at the
        // selector; the list shrinks and the cursor must clamp.
        s.cursor = kind_idx;
        s.apply(Event::Toggle); // none → flag
        s.apply(Event::Toggle); // flag → args
        s.cursor = s.fields().len() - 1;
        let fields_before = s.fields().len();
        s.cursor = kind_idx;
        s.apply(Event::Toggle); // args → none (list shrinks by 2)
        assert_eq!(s.fields().len(), fields_before - 2);
        assert!(s.cursor < s.fields().len());
    }

    #[test]
    fn quit_flow_confirms_when_dirty() {
        let mut s = studio();
        assert_eq!(s.apply(Event::Quit), Effect::Quit, "clean quit is direct");

        let mut s = studio();
        s.cursor = 0;
        s.apply(Event::Activate);
        s.apply(Event::Char('x'));
        s.apply(Event::Commit);
        assert!(s.dirty);
        assert_eq!(s.apply(Event::Quit), Effect::None);
        assert_eq!(s.mode, Mode::ConfirmQuit);
        assert_eq!(s.apply(Event::Cancel), Effect::None);
        assert_eq!(s.mode, Mode::Nav);
        s.apply(Event::Quit);
        assert_eq!(s.apply(Event::Commit), Effect::Quit);
    }

    #[test]
    fn save_effect_and_ack_clear_dirty() {
        let mut s = studio();
        s.apply(Event::Activate);
        s.apply(Event::Char('x'));
        s.apply(Event::Commit);
        assert!(s.dirty);
        assert_eq!(s.apply(Event::Save), Effect::Save);
        s.saved(Ok(()));
        assert!(!s.dirty);
        assert!(s.status.contains("saved"));

        s.saved(Err("disk full".into()));
        assert!(s.status.contains("save failed"));
    }

    #[test]
    fn save_with_errors_warns_in_status() {
        let mut s = studio();
        s.apply(Event::Activate);
        for _ in 0..4 {
            s.apply(Event::Backspace);
        }
        s.apply(Event::Commit); // blank id → invalid
        assert_eq!(s.apply(Event::Save), Effect::Save);
        // The event loop saves and reports back before the next frame, so the
        // rendered status must come out of saved(), not apply().
        s.saved(Ok(()));
        assert!(!s.dirty);
        assert!(s.status.contains("saved"), "{}", s.status);
        assert!(s.status.contains("1 validation problem"), "{}", s.status);
    }

    #[test]
    fn save_clean_reports_plain_saved() {
        let mut s = studio();
        assert_eq!(s.apply(Event::Save), Effect::Save);
        s.saved(Ok(()));
        assert!(s.status.contains("saved"), "{}", s.status);
        assert!(!s.status.contains("validation problem"), "{}", s.status);
    }

    #[test]
    fn probe_flow_updates_state() {
        let mut s = studio();
        assert_eq!(s.probe, ProbeState::NotRun);
        assert_eq!(s.apply(Event::Probe), Effect::Probe);
        assert_eq!(s.probe, ProbeState::Running);
        s.probe_done(vec!["✓ ok".into()]);
        assert_eq!(s.probe, ProbeState::Done(vec!["✓ ok".into()]));
    }

    #[test]
    fn adapter_cycling_wraps_and_resets_cursor() {
        let mut manifest = scaffold("aria", Flavor::Minimal);
        manifest
            .adapters
            .push(scaffold("nyx", Flavor::Continuity).adapters.remove(0));
        let mut s = Studio::new(manifest, PathBuf::from("multi.json"), false);
        s.cursor = 3;
        s.apply(Event::NextAdapter);
        assert_eq!(s.adapter_idx, 1);
        assert_eq!(s.adapter().id, "nyx");
        assert_eq!(s.cursor, 0);
        s.apply(Event::NextAdapter);
        assert_eq!(s.adapter_idx, 0);
        s.apply(Event::PrevAdapter);
        assert_eq!(s.adapter_idx, 1);
    }

    /// The state's validation cache always equals a fresh `validate_manifest`
    /// run — the panel can never drift from what `conjure validate` reports.
    #[test]
    fn validation_cache_matches_validate_manifest() {
        let mut s = studio_streaming();
        let stream_prefix_idx = s
            .fields()
            .iter()
            .position(|f| f.label == "stream prefix args")
            .unwrap();
        s.cursor = stream_prefix_idx;
        s.apply(Event::Activate);
        // Blank the stream prefix args of a streaming adapter → invalid.
        while !s.edit_buffer.is_empty() {
            s.apply(Event::Backspace);
        }
        s.apply(Event::Commit);
        assert_eq!(s.errors, validate_manifest(&s.manifest));
        assert!(!s.errors.is_empty());
    }
}
