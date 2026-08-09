use std::{
    io,
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

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

const MISSING_THRESHOLD: u8 = 2;

pub fn run(
    session: &mut SshSession,
    reverse_ports: &[u16],
    include_loopback: bool,
    auto_forward: bool,
    interval_seconds: u64,
) -> Result<()> {
    let mut tunnels = session
        .discover_ports(include_loopback)?
        .into_iter()
        .map(Tunnel::local)
        .collect::<Vec<_>>();
    tunnels.extend(reverse_ports.iter().copied().map(Tunnel::reverse));
    let mut state = TableState::default().with_selected((!tunnels.is_empty()).then_some(0));
    let mut message = format!("{} listener(s) found", tunnels.len());

    if auto_forward {
        for tunnel in tunnels
            .iter_mut()
            .filter(|t| t.direction == Direction::Local)
        {
            enable(session, tunnel, &mut message);
        }
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let (scan_tx, scan_rx) = mpsc::channel::<Result<Vec<u16>, String>>();
    let scanning = Arc::new(AtomicBool::new(true));
    let scanner_flag = Arc::clone(&scanning);
    let interval = Duration::from_secs(interval_seconds);
    let scanner = thread::scope(|scope| -> Result<()> {
        let scanner = scope.spawn(|| {
            let mut next_scan = Instant::now() + interval;
            while scanner_flag.load(Ordering::Relaxed) {
                if Instant::now() >= next_scan {
                    let result = session
                        .discover_ports(include_loopback)
                        .map_err(|error| format!("{error:#}"));
                    if scan_tx.send(result).is_err() {
                        break;
                    }
                    next_scan = Instant::now() + interval;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });

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
                let status = t.error.as_deref().unwrap_or(if t.enabled {
                    "ON"
                } else if t.manual_off {
                    "MANUAL OFF"
                } else if !t.present {
                    "TARGET DOWN"
                } else {
                    "off"
                });
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
                    "↑/↓ select  Space toggle  r refresh  q quit  │ scan {interval_seconds}s │ {message}"
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
                    KeyCode::Down | KeyCode::Char('j') => {
                        move_selection(&mut state, tunnels.len(), 1)
                    }
                    KeyCode::Up | KeyCode::Char('k') => {
                        move_selection(&mut state, tunnels.len(), -1)
                    }
                    KeyCode::Char(' ') | KeyCode::Enter => {
                        if let Some(i) = state.selected() {
                            toggle(session, &mut tunnels[i], &mut message);
                        }
                    }
                    KeyCode::Char('r') => match session.discover_ports(include_loopback) {
                        Ok(found) => {
                            reconcile_scan(session, &mut tunnels, found, auto_forward, &mut message)
                        }
                        Err(e) => message = format!("refresh failed: {e:#}"),
                    },
                    _ => {}
                }
            }
            while let Ok(scan) = scan_rx.try_recv() {
                match scan {
                    Ok(found) => {
                        reconcile_scan(session, &mut tunnels, found, auto_forward, &mut message)
                    }
                    Err(error) => message = format!("scan error; tunnels kept: {error}"),
                }
            }
        };
        scanning.store(false, Ordering::Relaxed);
        scanner.join().expect("scanner thread panicked");
        result
    });
    disable_raw_mode()?;
    execute!(terminal.backend_mut(), LeaveAlternateScreen)?;
    terminal.show_cursor()?;
    scanner
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
                tunnel.manual_off = true;
                *message = format!("port {port} disabled");
            }
            Err(e) => tunnel.error = Some(format!("cancel failed: {e:#}")),
        }
        return;
    }

    tunnel.manual_off = false;
    enable(session, tunnel, message);
}

fn enable(session: &SshSession, tunnel: &mut Tunnel, message: &mut String) {
    tunnel.error = None;
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

fn reconcile_scan(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    found: Vec<u16>,
    auto_forward: bool,
    message: &mut String,
) {
    for tunnel in tunnels
        .iter_mut()
        .filter(|t| t.direction == Direction::Local)
    {
        if found.contains(&tunnel.source_port) {
            tunnel.present = true;
            tunnel.missing_scans = 0;
            if auto_forward && !tunnel.enabled && !tunnel.manual_off {
                enable(session, tunnel, message);
            }
        } else {
            tunnel.missing_scans = tunnel.missing_scans.saturating_add(1);
            if tunnel.missing_scans >= MISSING_THRESHOLD {
                tunnel.present = false;
                if tunnel.enabled {
                    let port = tunnel.bind_port.expect("enabled tunnel has bind port");
                    match session.cancel(Direction::Local, port, tunnel.source_port) {
                        Ok(()) => {
                            tunnel.enabled = false;
                            *message = format!("remote port {} stopped", tunnel.source_port);
                        }
                        Err(error) => tunnel.error = Some(format!("cancel failed: {error:#}")),
                    }
                }
            }
        }
    }

    for port in found {
        if tunnels
            .iter()
            .any(|t| t.direction == Direction::Local && t.source_port == port)
        {
            continue;
        }
        let mut tunnel = Tunnel::local(port);
        if auto_forward {
            enable(session, &mut tunnel, message);
        }
        tunnels.push(tunnel);
    }
}
