use std::{io, time::Duration};

use anyhow::Result;
use crossterm::{
    event::{self, Event, KeyCode, KeyEventKind},
    execute,
    terminal::{EnterAlternateScreen, LeaveAlternateScreen, disable_raw_mode, enable_raw_mode},
};
use ratatui::{
    Terminal,
    backend::CrosstermBackend,
    layout::{Constraint, Layout},
    style::{Color, Modifier, Style},
    text::Line,
    widgets::{Block, Borders, Cell, Paragraph, Row, Table, TableState},
};

use crate::{
    ports::{Direction, Tunnel, available_local_port},
    ssh::SshSession,
};

pub fn run(session: &mut SshSession, reverse_ports: &[u16], include_loopback: bool) -> Result<()> {
    let mut tunnels = session
        .discover_ports(include_loopback)?
        .into_iter()
        .map(Tunnel::local)
        .collect::<Vec<_>>();
    tunnels.extend(reverse_ports.iter().copied().map(Tunnel::reverse));
    let mut state = TableState::default().with_selected((!tunnels.is_empty()).then_some(0));
    let mut message = format!("{} listener(s) found", tunnels.len());

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let result = loop {
        terminal.draw(|frame| {
            let areas = Layout::vertical([
                Constraint::Length(3),
                Constraint::Min(3),
                Constraint::Length(3),
            ])
            .split(frame.area());
            frame.render_widget(
                Paragraph::new("autotun  •  one SSH connection")
                    .block(Block::default().borders(Borders::ALL)),
                areas[0],
            );
            let rows = tunnels.iter().map(|t| {
                let direction = if t.direction == Direction::Local {
                    "LOCAL  -L"
                } else {
                    "REVERSE -R"
                };
                let bind = t
                    .bind_port
                    .map(|p| p.to_string())
                    .unwrap_or_else(|| "auto".into());
                let status = t
                    .error
                    .as_deref()
                    .unwrap_or(if t.enabled { "ON" } else { "off" });
                Row::new(vec![
                    Cell::from(direction),
                    Cell::from(t.source_port.to_string()),
                    Cell::from(bind),
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
                    Constraint::Length(12),
                    Constraint::Length(12),
                    Constraint::Length(12),
                    Constraint::Min(12),
                ],
            )
            .header(
                Row::new(["Direction", "Service port", "Bind port", "Status"])
                    .style(Style::default().add_modifier(Modifier::BOLD)),
            )
            .block(
                Block::default()
                    .title(" Discovered tunnels ")
                    .borders(Borders::ALL),
            )
            .row_highlight_style(Style::default().bg(Color::DarkGray))
            .highlight_symbol("› ");
            frame.render_stateful_widget(table, areas[1], &mut state);
            frame.render_widget(
                Paragraph::new(Line::from(format!(
                    "↑/↓ select  Space toggle  r refresh  q quit  │  {message}"
                )))
                .block(Block::default().borders(Borders::ALL)),
                areas[2],
            );
        })?;

        if event::poll(Duration::from_millis(250))? {
            let Event::Key(key) = event::read()? else {
                continue;
            };
            if key.kind != KeyEventKind::Press {
                continue;
            }
            match key.code {
                KeyCode::Char('q') | KeyCode::Esc => break Ok(()),
                KeyCode::Down | KeyCode::Char('j') => move_selection(&mut state, tunnels.len(), 1),
                KeyCode::Up | KeyCode::Char('k') => move_selection(&mut state, tunnels.len(), -1),
                KeyCode::Char(' ') | KeyCode::Enter => {
                    if let Some(i) = state.selected() {
                        toggle(session, &mut tunnels[i], &mut message);
                    }
                }
                KeyCode::Char('r') => match session.discover_ports(include_loopback) {
                    Ok(found) => {
                        for port in found {
                            if !tunnels
                                .iter()
                                .any(|t| t.direction == Direction::Local && t.source_port == port)
                            {
                                tunnels.push(Tunnel::local(port));
                            }
                        }
                        message = "remote listeners refreshed".into();
                    }
                    Err(e) => message = format!("refresh failed: {e:#}"),
                },
                _ => {}
            }
        }
    };
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    result
}

fn move_selection(state: &mut TableState, len: usize, delta: isize) {
    if len == 0 {
        return;
    }
    let current = state.selected().unwrap_or(0) as isize;
    state.select(Some((current + delta).rem_euclid(len as isize) as usize));
}

fn toggle(session: &SshSession, tunnel: &mut Tunnel, message: &mut String) {
    tunnel.error = None;
    if tunnel.enabled {
        let port = tunnel.bind_port.expect("enabled tunnel has a bind port");
        match session.cancel(tunnel.direction, port, tunnel.source_port) {
            Ok(()) => {
                tunnel.enabled = false;
                *message = format!("port {port} disabled");
            }
            Err(e) => tunnel.error = Some(format!("cancel failed: {e:#}")),
        }
        return;
    }

    let preferred = tunnel.source_port;
    if tunnel.direction == Direction::Reverse {
        match session.forward(Direction::Reverse, preferred, tunnel.source_port) {
            Ok(()) => {
                tunnel.bind_port = Some(preferred);
                tunnel.enabled = true;
                *message = format!("reverse port {preferred} enabled");
            }
            Err(e) => tunnel.error = Some(format!("forward failed: {e:#}")),
        }
        return;
    }

    match available_local_port(preferred) {
        Ok((port, reservation)) => {
            drop(reservation);
            match session.forward(Direction::Local, port, tunnel.source_port) {
                Ok(()) => {
                    tunnel.bind_port = Some(port);
                    tunnel.enabled = true;
                    *message = if port == preferred {
                        format!("port {port} enabled")
                    } else {
                        format!("local {preferred} busy; using {port}")
                    };
                }
                Err(e) => tunnel.error = Some(format!("forward failed: {e:#}")),
            }
        }
        Err(e) => tunnel.error = Some(format!("allocation failed: {e:#}")),
    }
}
