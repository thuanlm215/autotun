use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    cursor::Show,
    event::{self, Event, KeyCode, KeyEventKind, KeyModifiers},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Cell, HighlightSpacing, Paragraph, Row, Table, TableState},
};

use crate::{
    engine::{self, Engine},
    ports::{Direction, Tunnel},
};

struct InlineForm {
    direction: Direction,
    fields: [String; 3],
    selected: usize,
    /// When set, the form updates an existing tunnel instead of creating one.
    edit_index: Option<usize>,
    was_enabled: bool,
}

impl InlineForm {
    fn new(direction: Direction) -> Self {
        Self {
            direction,
            fields: [String::new(), String::new(), String::new()],
            selected: 0,
            edit_index: None,
            was_enabled: false,
        }
    }

    fn edit(tunnel: &Tunnel, index: usize) -> Self {
        Self {
            direction: tunnel.direction,
            fields: [
                tunnel.source_port.to_string(),
                tunnel.requested_port.to_string(),
                tunnel.label.clone(),
            ],
            selected: 0,
            edit_index: Some(index),
            was_enabled: tunnel.enabled,
        }
    }

    fn labels(&self) -> [&'static str; 3] {
        match self.direction {
            Direction::Local => ["Remote port", "Local port", "Label"],
            Direction::Reverse => ["Local port", "Remote port", "Label"],
        }
    }

    fn title(&self) -> &'static str {
        match (self.edit_index.is_some(), self.direction) {
            (false, Direction::Local) => " Add forward (remote → local) ",
            (false, Direction::Reverse) => " Add reverse (local → remote) ",
            (true, Direction::Local) => " Edit forward (remote → local) ",
            (true, Direction::Reverse) => " Edit reverse (local → remote) ",
        }
    }

    fn help_line() -> &'static str {
        "↑↓ Select  Enter Next/Save  Esc Cancel"
    }

    fn tunnel(&self) -> Result<Tunnel> {
        engine::tunnel_from_form(
            self.direction,
            &self.fields[0],
            &self.fields[1],
            &self.fields[2],
        )
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

pub fn run(mut engine: Engine) -> Result<()> {
    let result = run_tui(&mut engine);
    engine.shutdown();
    result
}

fn run_tui(engine: &mut Engine) -> Result<()> {
    let mut state =
        TableState::default().with_selected((!engine.tunnels().is_empty()).then_some(0));

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _terminal_guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;

    let mut form = None::<InlineForm>;
    let mut show_help = false;
    let mut filter = None::<String>;

    loop {
        engine.poll();

        let tunnels = engine.tunnels();
        let visible_indices: Vec<usize> = tunnels
            .iter()
            .enumerate()
            .filter(|(_, t)| {
                if let Some(query) = &filter {
                    let q = query.to_lowercase();
                    t.label.to_lowercase().contains(&q)
                        || t.source_port.to_string().contains(&q)
                        || t.bind_port
                            .map(|p| p.to_string().contains(&q))
                            .unwrap_or(false)
                } else {
                    true
                }
            })
            .map(|(i, _)| i)
            .collect();
        if visible_indices.is_empty() {
            state.select(None);
        } else if let Some(sel) = state.selected() {
            if sel >= visible_indices.len() {
                state.select(Some(visible_indices.len() - 1));
            }
        } else {
            state.select(Some(0));
        }

        terminal.draw(|frame| {
            let form_height = if form.is_some() { 5 } else { 0 };
            let filter_height = if filter.is_some() { 3 } else { 0 };
            let areas = Layout::vertical([
                Constraint::Min(3),
                Constraint::Length(form_height),
                Constraint::Length(filter_height),
                Constraint::Length(3),
            ])
            .split(frame.area());

            // Selection gutter (› + space). Help text uses spaces so it lines
            // up with Direction. The border title uses the same width in ─
            // glyphs so the top line stays continuous: ┌──autotun────
            // (spaces would punch holes: ┌  autotun─).
            const HIGHLIGHT: &str = "› ";
            let gutter_width = HIGHLIGHT.chars().count();
            let gutter_pad = " ".repeat(gutter_width);
            let title_leader: String = "─".repeat(gutter_width);

            let destination = engine.destination();
            let status_indicator = if engine.connected() {
                "●"
            } else {
                "○ reconnecting"
            };
            let status_style = if engine.connected() {
                Color::Green
            } else {
                Color::Yellow
            };

            let rows = visible_indices.iter().map(|&i| {
                let t = &engine.tunnels()[i];
                let direction = if t.direction == Direction::Local {
                    "Forward"
                } else {
                    "Reverse"
                };
                let (remote_port, local_port) = match t.direction {
                    Direction::Local => (
                        t.source_port.to_string(),
                        t.bind_port
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| "auto".into()),
                    ),
                    Direction::Reverse => (
                        t.bind_port
                            .map(|p| p.to_string())
                            .unwrap_or_else(|| t.requested_port.to_string()),
                        t.source_port.to_string(),
                    ),
                };
                let status = t.error.as_deref().unwrap_or(if t.enabled {
                    "ON"
                } else if t.manual_off {
                    "MANUAL OFF"
                } else if !t.present {
                    "TARGET DOWN"
                } else {
                    "off"
                });
                let url = engine::tunnel_url(t);
                Row::new(vec![
                    Cell::from(direction),
                    Cell::from(if t.label.is_empty() { "—" } else { &t.label }),
                    Cell::from(remote_port),
                    Cell::from(local_port),
                    Cell::from(url),
                    Cell::from(status),
                ])
                .style(if t.enabled {
                    Style::default().fg(Color::Green)
                } else {
                    Style::default()
                })
            });
            let table = Table::new(
                rows,
                [
                    Constraint::Length(12), // Direction
                    Constraint::Length(15), // Label
                    Constraint::Length(15), // Remote port
                    Constraint::Length(15), // Local port
                    // Fixed width: min == max == 25 (Length is the ratatui form).
                    Constraint::Length(25), // URL
                    Constraint::Min(12),    // Status
                ],
            )
            .header(
                Row::new([
                    "Direction",
                    "Label",
                    "Remote port",
                    "Local port",
                    "URL",
                    "Status",
                ])
                .style(Style::default().add_modifier(Modifier::BOLD))
                .top_margin(1),
            )
            .block(
                Block::default()
                    .title(Line::from(vec![
                        Span::raw(format!("{title_leader}autotun─{destination} ")),
                        Span::styled(status_indicator, Style::default().fg(status_style)),
                    ]))
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol(HIGHLIGHT)
            .highlight_spacing(HighlightSpacing::Always);
            frame.render_stateful_widget(table, areas[0], &mut state);

            if let Some(form) = &form {
                let labels = form.labels();
                let lines = labels
                    .iter()
                    .zip(form.fields.iter())
                    .enumerate()
                    .map(|(index, (label, value))| {
                        let placeholder = match (form.direction, index) {
                            (Direction::Local, 1) if value.is_empty() => "same as remote port",
                            (Direction::Reverse, 1) if value.is_empty() => "same as local port",
                            _ => "",
                        };
                        let display = if value.is_empty() {
                            placeholder
                        } else {
                            value.as_str()
                        };
                        let content = format!("{label}: {display}");
                        if index == form.selected {
                            Line::from(Span::styled(
                                content,
                                Style::default()
                                    .fg(Color::Yellow)
                                    .add_modifier(Modifier::BOLD),
                            ))
                        } else {
                            Line::from(content)
                        }
                    })
                    .collect::<Vec<_>>();
                frame.render_widget(
                    Paragraph::new(lines)
                        .block(Block::default().title(form.title()).borders(Borders::ALL)),
                    areas[1],
                );
            }

            if let Some(query) = &filter {
                let filter_line = Line::from(vec![
                    Span::styled("/ ", Style::default().fg(Color::Yellow)),
                    Span::raw(query.as_str()),
                    Span::styled("█", Style::default().fg(Color::Yellow)),
                ]);
                frame.render_widget(
                    Paragraph::new(filter_line)
                        .block(Block::default().title(" Filter ").borders(Borders::ALL)),
                    areas[2],
                );
            }

            let footer = if form.is_some() {
                format!("{gutter_pad}{}", InlineForm::help_line())
            } else if filter.is_some() {
                format!("{gutter_pad}Type to filter  Esc Clear  Enter Confirm")
            } else if let Some(notice) = engine.notice() {
                format!("{gutter_pad}{notice}")
            } else if show_help {
                format!(
                    "{gutter_pad}↑↓ Select  Space Toggle  a Forward  v Reverse  e Edit  d Remove  r Rescan  p Fwd image  c Copy URL  / Filter  ? Help  q Quit"
                )
            } else {
                format!("{gutter_pad}? Help")
            };
            frame.render_widget(
                Paragraph::new(Line::from(footer)).block(Block::default().borders(Borders::ALL)),
                areas[3],
            );
        })?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            if key.code == KeyCode::Char('c') && key.modifiers.contains(KeyModifiers::CONTROL) {
                return Ok(());
            }
            if let Some(active_form) = form.as_mut() {
                match handle_inline_form(key.code, active_form) {
                    Ok(Some(tunnel)) => {
                        if let Some(index) = active_form.edit_index {
                            engine.edit(index, tunnel, active_form.was_enabled);
                        } else {
                            let added = engine.add(tunnel);
                            // Select the new row in the unfiltered list if visible.
                            if let Some(pos) = visible_indices.iter().position(|&i| i == added) {
                                state.select(Some(pos));
                            } else {
                                state.select(Some(added));
                            }
                        }
                        form = None;
                    }
                    Ok(None) => {}
                    Err(_) => {}
                }
                if matches!(key.code, KeyCode::Esc) {
                    form = None;
                }
            } else if let Some(query) = filter.as_mut() {
                match key.code {
                    KeyCode::Esc => {
                        filter = None;
                    }
                    KeyCode::Enter => {
                        if query.is_empty() {
                            filter = None;
                        }
                    }
                    KeyCode::Backspace => {
                        query.pop();
                        if query.is_empty() {
                            filter = None;
                        }
                    }
                    KeyCode::Char(c) => {
                        query.push(c);
                    }
                    _ => {}
                }
            } else {
                match key.code {
                    KeyCode::Char('q') => return Ok(()),
                    KeyCode::Esc => engine.clear_notice(),
                    KeyCode::Down | KeyCode::Char('j') => {
                        move_selection(&mut state, visible_indices.len(), 1)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        move_selection(&mut state, visible_indices.len(), -1)
                    }
                    KeyCode::Char(' ') => {
                        if let Some(vi) = state.selected()
                            && let Some(&i) = visible_indices.get(vi)
                        {
                            engine.toggle(i);
                        }
                    }
                    KeyCode::Enter | KeyCode::Char('e') => {
                        if let Some(vi) = state.selected()
                            && let Some(&i) = visible_indices.get(vi)
                        {
                            form = Some(InlineForm::edit(&engine.tunnels()[i], i));
                        }
                    }
                    KeyCode::Char('a') => form = Some(InlineForm::new(Direction::Local)),
                    KeyCode::Char('v') => form = Some(InlineForm::new(Direction::Reverse)),
                    KeyCode::Char('d') => {
                        if let Some(vi) = state.selected()
                            && let Some(&i) = visible_indices.get(vi)
                        {
                            engine.delete(i);
                            if engine.tunnels().is_empty() {
                                state.select(None);
                            }
                        }
                    }
                    KeyCode::Char('r') => engine.rescan(),
                    KeyCode::Char('p') => {
                        let _ = engine.push_clipboard_image();
                    }
                    KeyCode::Char('/') => {
                        filter = Some(String::new());
                    }
                    KeyCode::Char('c') => {
                        if let Some(vi) = state.selected()
                            && let Some(&i) = visible_indices.get(vi)
                        {
                            let url = engine::tunnel_url(&engine.tunnels()[i]);
                            if url != "—" {
                                crate::clip::copy_text_to_clipboard(&url);
                            }
                        }
                    }
                    KeyCode::Char('?') => {
                        engine.clear_notice();
                        show_help = !show_help;
                    }
                    _ => {}
                }
            }
        }
    }
}

fn handle_inline_form(key: KeyCode, form: &mut InlineForm) -> Result<Option<Tunnel>> {
    match key {
        KeyCode::Down => form.selected = (form.selected + 1) % form.fields.len(),
        KeyCode::Up => form.selected = (form.selected + form.fields.len() - 1) % form.fields.len(),
        KeyCode::Backspace => {
            form.fields[form.selected].pop();
        }
        KeyCode::Enter => {
            if form.selected + 1 == form.fields.len() {
                return form.tunnel().map(Some);
            }
            form.selected += 1;
        }
        KeyCode::Char(character) => form.fields[form.selected].push(character),
        _ => {}
    }
    Ok(None)
}

fn move_selection(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    state.select(Some((current + delta).rem_euclid(len as isize) as usize));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inline_local_form_uses_service_port_as_default_bind_port() {
        let mut form = InlineForm::new(Direction::Local);
        form.fields = ["3000".into(), String::new(), "frontend".into()];
        let tunnel = form.tunnel().unwrap();
        assert_eq!(tunnel.direction, Direction::Local);
        assert_eq!(tunnel.source_port, 3000);
        assert_eq!(tunnel.requested_port, 3000);
        assert_eq!(tunnel.label, "frontend");
    }

    #[test]
    fn inline_form_rejects_zero_port() {
        let mut form = InlineForm::new(Direction::Reverse);
        form.fields[0] = "0".into();
        assert!(form.tunnel().is_err());
    }
}
