//! `conjure studio` runtime: terminal lifecycle, key mapping, and effect
//! execution around the pure [`state::Studio`] machine.
//!
//! Everything interesting is testable without a TTY: state transitions in
//! [`state`], field accessors in [`fields`], argv composition in [`preview`],
//! and rendering in [`render`] (via ratatui's `TestBackend`). This module is
//! the thin impure shell: crossterm events in, effects (save/probe/quit) out.

mod fields;
mod preview;
mod render;
mod state;

use std::fs;
use std::path::PathBuf;

use anyhow::{Context, Result};
use coven_runtime_spec::AdapterManifest;
use ratatui::crossterm::event::{self, Event as TermEvent, KeyCode, KeyEventKind, KeyModifiers};

use crate::commands::canonical_manifest;
use crate::commands::test::{probe_adapter_report, ProbeReport};
use state::{Effect, Event, Mode, Studio};

/// Run the studio over a manifest until the user quits. `fresh` marks a
/// manifest that doesn't exist on disk yet (starts dirty).
pub(crate) fn run(manifest: AdapterManifest, path: PathBuf, fresh: bool) -> Result<()> {
    let mut studio = Studio::new(manifest, path, fresh);
    let mut terminal = ratatui::init();
    let result = event_loop(&mut terminal, &mut studio);
    ratatui::restore();
    result
}

fn event_loop(terminal: &mut ratatui::DefaultTerminal, studio: &mut Studio) -> Result<()> {
    loop {
        terminal
            .draw(|frame| render::draw(frame, studio))
            .context("draw studio frame")?;

        let Some(event) = next_event(studio.mode)? else {
            continue;
        };
        match studio.apply(event) {
            Effect::None => {}
            Effect::Quit => return Ok(()),
            Effect::Save => {
                let result = save(studio);
                studio.saved(result);
            }
            Effect::Probe => {
                // Paint the "probing…" state before the bounded blocking probe.
                terminal
                    .draw(|frame| render::draw(frame, studio))
                    .context("draw studio frame")?;
                let report = probe_report_lines(studio);
                studio.probe_done(report);
            }
        }
    }
}

fn save(studio: &Studio) -> std::result::Result<(), String> {
    let bytes = canonical_manifest(&studio.manifest).map_err(|e| e.to_string())?;
    fs::write(&studio.path, bytes).map_err(|e| e.to_string())
}

/// Probe the selected adapter with the exact machinery `conjure test` uses and
/// flatten the outcome into renderable lines.
fn probe_report_lines(studio: &Studio) -> Vec<String> {
    let adapter = studio.adapter();
    match probe_adapter_report(adapter) {
        ProbeReport::Ok { probe, warnings } => {
            let mut lines = vec![format!("✓ `{}` responded to `{probe}`", adapter.executable)];
            if warnings.is_empty() {
                lines.push("✓ every declared flag seen in probe output".into());
            }
            lines.extend(warnings.into_iter().map(|w| format!("⚠ {w}")));
            lines
        }
        ProbeReport::NotFound => vec![
            format!("✗ executable `{}` not found on PATH", adapter.executable),
            format!("  {}", adapter.install_hint),
        ],
        ProbeReport::NotRunnable(msg) => {
            vec![format!(
                "✗ `{}` did not run cleanly: {msg}",
                adapter.executable
            )]
        }
    }
}

/// Block for the next key press and map it to a state [`Event`].
/// Filters key releases (Windows terminals emit both) and non-key events.
fn next_event(mode: Mode) -> Result<Option<Event>> {
    let term_event = event::read().context("read terminal event")?;
    let TermEvent::Key(key) = term_event else {
        return Ok(None);
    };
    if key.kind != KeyEventKind::Press {
        return Ok(None);
    }
    if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
        return Ok(Some(Event::ForceQuit));
    }
    Ok(match mode {
        Mode::Edit => match key.code {
            KeyCode::Enter => Some(Event::Commit),
            KeyCode::Esc => Some(Event::Cancel),
            KeyCode::Backspace => Some(Event::Backspace),
            KeyCode::Char(c) => Some(Event::Char(c)),
            _ => None,
        },
        Mode::ConfirmQuit => match key.code {
            KeyCode::Char('y') | KeyCode::Enter => Some(Event::Commit),
            KeyCode::Char('n') | KeyCode::Esc => Some(Event::Cancel),
            KeyCode::Char('q') => Some(Event::Quit),
            _ => None,
        },
        Mode::Nav => match key.code {
            KeyCode::Up | KeyCode::Char('k') => Some(Event::Up),
            KeyCode::Down | KeyCode::Char('j') => Some(Event::Down),
            KeyCode::Tab => Some(Event::NextSection),
            KeyCode::BackTab => Some(Event::PrevSection),
            KeyCode::Enter | KeyCode::Char('i') | KeyCode::Char('e') => Some(Event::Activate),
            KeyCode::Char(' ') => Some(Event::Toggle),
            KeyCode::Char('[') => Some(Event::PrevAdapter),
            KeyCode::Char(']') => Some(Event::NextAdapter),
            KeyCode::Char('s') => Some(Event::Save),
            KeyCode::Char('t') => Some(Event::Probe),
            KeyCode::Char('q') | KeyCode::Esc => Some(Event::Quit),
            _ => None,
        },
    })
}
