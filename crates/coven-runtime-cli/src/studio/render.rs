//! Pure view: renders a [`Studio`] state into a ratatui frame. No state
//! mutation happens here — scrolling offsets are derived from the cursor each
//! draw, so the renderer stays a function of the state.

use ratatui::layout::{Constraint, Direction, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Frame;

use super::fields::FieldKind;
use super::preview::launch_preview;
use super::state::{Mode, ProbeState, Studio};

const ACCENT: Color = Color::Magenta;
const ERR: Color = Color::Red;
const OK: Color = Color::Green;
const WARN: Color = Color::Yellow;
const DIM: Color = Color::DarkGray;

pub(crate) fn draw(frame: &mut Frame, studio: &Studio) {
    let [title_area, body_area, help_area, status_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Length(1),
            Constraint::Min(8),
            Constraint::Length(1),
            Constraint::Length(1),
        ])
        .areas(frame.area());

    draw_title(frame, studio, title_area);

    let [form_area, right_area] = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(52), Constraint::Percentage(48)])
        .areas(body_area);

    draw_form(frame, studio, form_area);

    let preview_lines = preview_lines(studio);
    let preview_height = preview_panel_height(&preview_lines, right_area.width, right_area.height);
    let [validation_area, preview_area, probe_area] = Layout::default()
        .direction(Direction::Vertical)
        .constraints([
            Constraint::Min(3),
            Constraint::Length(preview_height),
            Constraint::Min(3),
        ])
        .areas(right_area);

    draw_validation(frame, studio, validation_area);
    draw_preview(frame, preview_lines, preview_area);
    draw_probe(frame, studio, probe_area);

    draw_help(frame, studio, help_area);
    draw_status(frame, studio, status_area);
}

fn draw_title(frame: &mut Frame, studio: &Studio, area: Rect) {
    let adapters = studio.manifest.adapters.len();
    let mut spans = vec![
        Span::styled(
            " conjure studio ",
            Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
        ),
        Span::raw(format!("— {}", studio.path.display())),
    ];
    if studio.dirty {
        spans.push(Span::styled(" • modified", Style::default().fg(WARN)));
    }
    spans.push(Span::styled(
        format!(
            "   adapter {}/{} ({})",
            studio.adapter_idx + 1,
            adapters,
            studio.adapter().id
        ),
        Style::default().fg(DIM),
    ));
    frame.render_widget(Paragraph::new(Line::from(spans)), area);
}

/// The form: section headers interleaved with `label  value` rows, scrolled
/// so the focused row stays visible.
fn draw_form(frame: &mut Frame, studio: &Studio, area: Rect) {
    let fields = studio.fields();
    let error_fields: Vec<&'static str> = {
        let id = studio.adapter().id.trim().to_lowercase();
        studio
            .errors
            .iter()
            .filter(|e| e.adapter_id.as_deref() == Some(id.as_str()))
            .map(|e| e.field)
            .collect()
    };

    let mut lines: Vec<Line> = Vec::new();
    let mut cursor_row = 0usize;
    let mut last_section = None;
    for (idx, field) in fields.iter().enumerate() {
        if last_section != Some(field.section) {
            last_section = Some(field.section);
            lines.push(Line::from(Span::styled(
                format!("─ {} ", field.section.title()),
                Style::default().fg(ACCENT).add_modifier(Modifier::BOLD),
            )));
        }

        let focused = idx == studio.cursor;
        if focused {
            cursor_row = lines.len();
        }
        let has_error = field.error_keys.iter().any(|k| error_fields.contains(k));

        let marker = if has_error { "✗ " } else { "  " };
        let value = if focused && studio.mode == Mode::Edit {
            format!("{}▏", studio.edit_buffer)
        } else {
            let v = (field.get)(studio.adapter());
            match field.kind {
                FieldKind::Bool => {
                    if v == "true" {
                        "[x] true".into()
                    } else {
                        "[ ] false".into()
                    }
                }
                FieldKind::SandboxKind | FieldKind::ModelIdTransform => format!("‹{v}›"),
                _ if v.is_empty() => "—".into(),
                _ => v,
            }
        };

        let mut style = Style::default();
        if focused {
            style = style.add_modifier(Modifier::REVERSED);
        }
        let value_style = if has_error {
            style.fg(ERR)
        } else if focused && studio.mode == Mode::Edit {
            style.fg(WARN)
        } else {
            style
        };
        lines.push(Line::from(vec![
            Span::styled(marker, Style::default().fg(ERR)),
            Span::styled(format!("{:<28}", field.label), style),
            Span::styled(value, value_style),
        ]));
    }

    let viewport = area.height.saturating_sub(2) as usize; // borders
    let scroll = cursor_row.saturating_sub(viewport.saturating_sub(1).max(1)) as u16;

    frame.render_widget(
        Paragraph::new(lines)
            .scroll((scroll, 0))
            .block(Block::default().borders(Borders::ALL).title(" Manifest ")),
        area,
    );
}

fn draw_validation(frame: &mut Frame, studio: &Studio, area: Rect) {
    let focused: Vec<_> = studio.focused_errors();
    let mut lines: Vec<Line> = Vec::new();
    if studio.errors.is_empty() {
        lines.push(Line::from(Span::styled(
            "✓ valid — zero problems (the PR bar)",
            Style::default().fg(OK),
        )));
    } else {
        let ordered_errors = studio
            .errors
            .iter()
            .filter(|error| focused.contains(error))
            .chain(
                studio
                    .errors
                    .iter()
                    .filter(|error| !focused.contains(error)),
            );
        for error in ordered_errors {
            let is_focused = focused.contains(&error);
            let style = if is_focused {
                Style::default().fg(ERR).add_modifier(Modifier::BOLD)
            } else {
                Style::default().fg(ERR)
            };
            lines.push(Line::from(Span::styled(format!("✗ {error}"), style)));
        }
    }
    let title = format!(" Validation ({}) ", studio.errors.len());
    frame.render_widget(
        Paragraph::new(lines)
            .wrap(Wrap { trim: false })
            .block(Block::default().borders(Borders::ALL).title(title)),
        area,
    );
}

fn preview_lines(studio: &Studio) -> Vec<Line<'static>> {
    launch_preview(studio.adapter())
        .into_iter()
        .map(|preview| {
            Line::from(vec![
                Span::styled(
                    format!("{:<18}", preview.label),
                    Style::default().fg(ACCENT),
                ),
                Span::styled("$ ", Style::default().fg(DIM)),
                Span::raw(preview.command()),
            ])
        })
        .collect()
}

/// Reserve enough vertical space for wrapped preview commands while leaving
/// useful validation and probe panels. The extra row per logical line covers
/// word-wrap slack that a width-only estimate cannot predict.
fn preview_panel_height(lines: &[Line<'_>], panel_width: u16, available_height: u16) -> u16 {
    let inner_width = panel_width.saturating_sub(2).max(1) as usize;
    let content_height = lines
        .iter()
        .map(|line| line.width().div_ceil(inner_width).saturating_add(1))
        .sum::<usize>();
    let desired = u16::try_from(content_height.saturating_add(2)).unwrap_or(u16::MAX);
    desired.clamp(3, available_height.saturating_sub(6).max(3))
}

fn draw_preview(frame: &mut Frame, lines: Vec<Line<'static>>, area: Rect) {
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Launch preview (illustrative) "),
        ),
        area,
    );
}

fn draw_probe(frame: &mut Frame, studio: &Studio, area: Rect) {
    let lines: Vec<Line> = match &studio.probe {
        ProbeState::NotRun => vec![Line::from(Span::styled(
            "press t to probe the binary (PATH + --version/--help, soft flag checks)",
            Style::default().fg(DIM),
        ))],
        ProbeState::Running => vec![Line::from(Span::styled(
            format!("probing `{}`…", studio.adapter().executable),
            Style::default().fg(WARN),
        ))],
        ProbeState::Done(report) => report
            .iter()
            .map(|line| {
                let style = if line.starts_with('✗') {
                    Style::default().fg(ERR)
                } else if line.starts_with('⚠') {
                    Style::default().fg(WARN)
                } else {
                    Style::default().fg(OK)
                };
                Line::from(Span::styled(line.clone(), style))
            })
            .collect(),
    };
    frame.render_widget(
        Paragraph::new(lines).wrap(Wrap { trim: false }).block(
            Block::default()
                .borders(Borders::ALL)
                .title(" Conformance probe "),
        ),
        area,
    );
}

fn draw_help(frame: &mut Frame, studio: &Studio, area: Rect) {
    frame.render_widget(
        Paragraph::new(Line::from(Span::styled(
            format!(" {}", studio.current_field().help),
            Style::default().fg(DIM),
        ))),
        area,
    );
}

fn draw_status(frame: &mut Frame, studio: &Studio, area: Rect) {
    let line = match studio.mode {
        Mode::ConfirmQuit => Line::from(Span::styled(
            " unsaved changes — quit anyway? y / n",
            Style::default().fg(WARN).add_modifier(Modifier::BOLD),
        )),
        Mode::Edit => Line::from(Span::styled(
            " editing — Enter commit · Esc cancel",
            Style::default().fg(WARN),
        )),
        Mode::Nav => Line::from(vec![
            Span::styled(format!(" {}", studio.status), Style::default()),
            Span::styled(
                "   ↑/↓ move · Tab section · Enter edit · Space toggle · [/] adapter · s save · t probe · q quit",
                Style::default().fg(DIM),
            ),
        ]),
    };
    frame.render_widget(Paragraph::new(line), area);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::studio::state::Event;
    use crate::template::{scaffold, Flavor};
    use ratatui::backend::TestBackend;
    use ratatui::Terminal;
    use std::path::PathBuf;

    fn rendered(studio: &Studio) -> String {
        let mut terminal = Terminal::new(TestBackend::new(120, 40)).unwrap();
        terminal.draw(|f| draw(f, studio)).unwrap();
        let buffer = terminal.backend().buffer().clone();
        let mut out = String::new();
        for y in 0..buffer.area.height {
            for x in 0..buffer.area.width {
                out.push_str(buffer[(x, y)].symbol());
            }
            out.push('\n');
        }
        out
    }

    fn normalized_right_panel(screen: &str, title: &str, next_title: &str) -> String {
        let lines: Vec<&str> = screen.lines().collect();
        let start = lines
            .iter()
            .position(|line| line.contains(title))
            .unwrap_or_else(|| panic!("`{title}` panel title"));
        let end = lines
            .iter()
            .enumerate()
            .skip(start + 1)
            .find(|(_, line)| line.contains(next_title))
            .map(|(index, _)| index)
            .unwrap_or_else(|| panic!("`{next_title}` panel title"));
        let panel_x = lines[start]
            .chars()
            .enumerate()
            .filter(|(_, character)| *character == '┌')
            .map(|(index, _)| index)
            .last()
            .expect("right panel boundary");

        lines[start + 1..end]
            .iter()
            .map(|line| {
                line.chars()
                    .skip(panel_x + 1)
                    .take_while(|character| *character != '│')
                    .collect::<String>()
            })
            .collect::<Vec<_>>()
            .join(" ")
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ")
    }

    fn normalized_validation_panel(screen: &str) -> String {
        normalized_right_panel(screen, "Validation (", "Launch preview")
    }

    fn normalized_preview_panel(screen: &str) -> String {
        normalized_right_panel(screen, "Launch preview", "Conformance probe")
    }

    fn assert_complete_stream_panels(screen: &str) {
        let preview = normalized_preview_panel(screen);
        for (label, command) in [
            (
                "stream",
                r#"zephyr --model <model-without-provider> --system-prompt "<system>" -p --input-format stream-json --output-format stream-json --verbose --session-id <session-id>"#,
            ),
            (
                "stream resume",
                r#"zephyr --model <model-without-provider> --system-prompt "<system>" -p --input-format stream-json --output-format stream-json --verbose --resume <session-id>"#,
            ),
        ] {
            let expected = format!("{label} $ {command}");
            assert!(
                preview.contains(&expected),
                "missing `{expected}` from preview panel:\n{preview}\n\n{screen}"
            );
        }
        assert!(screen.contains("press t to probe"), "{screen}");
    }

    #[test]
    fn renders_scaffold_with_all_panels() {
        let studio = Studio::new(
            scaffold("aria", Flavor::Minimal),
            PathBuf::from("aria.json"),
            false,
        );
        let screen = rendered(&studio);
        assert!(screen.contains("conjure studio"), "{screen}");
        assert!(screen.contains("aria.json"), "{screen}");
        assert!(screen.contains("Manifest"), "{screen}");
        assert!(screen.contains("Identity"), "{screen}");
        assert!(screen.contains("Capabilities"), "{screen}");
        assert!(screen.contains("✓ valid"), "{screen}");
        assert!(screen.contains("Launch preview"), "{screen}");
        assert!(screen.contains("\"<prompt>\""), "{screen}");
        assert!(screen.contains("press t to probe"), "{screen}");
    }

    #[test]
    fn renders_model_transform_as_cycle_selector() {
        let mut studio = Studio::new(
            scaffold("aria", Flavor::Minimal),
            PathBuf::from("aria.json"),
            false,
        );
        studio.cursor = studio
            .fields()
            .iter()
            .position(|field| field.label == "model_id_transform")
            .unwrap();

        let screen = rendered(&studio);
        assert!(screen.contains("‹strip_provider›"), "{screen}");

        studio.apply(Event::Toggle);
        let screen = rendered(&studio);
        assert!(screen.contains("‹preserve›"), "{screen}");
        assert!(!screen.contains("editing — Enter commit"), "{screen}");
    }

    #[test]
    fn renders_errors_and_dirty_marker() {
        let mut studio = Studio::new(
            scaffold("aria", Flavor::Minimal),
            PathBuf::from("aria.json"),
            false,
        );
        // Blank the id → error + dirty.
        studio.apply(Event::Activate);
        for _ in 0..4 {
            studio.apply(Event::Backspace);
        }
        studio.apply(Event::Commit);
        let screen = rendered(&studio);
        assert!(screen.contains("• modified"), "{screen}");
        assert!(screen.contains("Validation (1)"), "{screen}");
        assert!(screen.contains("must not be empty"), "{screen}");
    }

    #[test]
    fn renders_edit_and_confirm_modes() {
        let mut studio = Studio::new(
            scaffold("aria", Flavor::Minimal),
            PathBuf::from("aria.json"),
            false,
        );
        studio.apply(Event::Activate);
        let screen = rendered(&studio);
        assert!(screen.contains("editing — Enter commit"), "{screen}");

        studio.apply(Event::Char('x'));
        studio.apply(Event::Commit);
        studio.apply(Event::Quit);
        let screen = rendered(&studio);
        assert!(screen.contains("quit anyway?"), "{screen}");
    }

    #[test]
    fn renders_complete_stream_commands_at_120x40() {
        let studio = Studio::new(
            scaffold("zephyr", Flavor::Streaming),
            PathBuf::from("zephyr.json"),
            false,
        );
        let screen = rendered(&studio);
        assert_complete_stream_panels(&screen);
        assert!(screen.contains("✓ valid"), "{screen}");
    }

    #[test]
    fn focused_late_error_is_visible_with_complete_stream_panels_at_120x40() {
        let mut manifest = scaffold("zephyr", Flavor::Streaming);
        let adapter = &mut manifest.adapters[0];
        adapter.label.clear();
        adapter.install_hint.clear();
        adapter.version = Some("not-semver".into());
        adapter.prompt_flag = Some(" ".into());
        adapter.interactive_prompt_flag = Some(" ".into());
        let Some(coven_runtime_spec::SandboxMapping::Flag {
            flag,
            full,
            read_only,
        }) = &mut adapter.sandbox
        else {
            panic!("streaming scaffold uses flag-form sandbox");
        };
        flag.clear();
        full.clear();
        read_only.clear();

        let mut studio = Studio::new(manifest, PathBuf::from("zephyr.json"), false);
        assert_eq!(studio.errors.len(), 8, "{:?}", studio.errors);
        studio.cursor = studio
            .fields()
            .iter()
            .position(|field| field.label == "read-only value")
            .expect("sandbox read-only field");

        let screen = rendered(&studio);
        let validation = normalized_validation_panel(&screen);
        let focused_error =
            "adapter `zephyr` [sandbox.read_only]: sandbox `read_only` value must not be empty";
        assert!(
            validation.contains(focused_error),
            "missing focused diagnostic from validation panel:\n{validation}\n\n{screen}"
        );
        assert!(screen.contains("Validation (8)"), "{screen}");
        assert_complete_stream_panels(&screen);
    }
}
