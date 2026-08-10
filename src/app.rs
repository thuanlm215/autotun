use std::{
    io::{self, Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

use anyhow::{Context, Result};
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
    ports::{
        Direction, MAX_PORT_FALLBACKS, Protocol, RemoteListener, Tunnel, available_local_port,
    },
    ssh::SshSession,
};

const MISSING_THRESHOLD: u8 = 2;

enum ScanEvent {
    Ports(Vec<RemoteListener>),
    Reconnected(Vec<RemoteListener>),
    Error(String),
}

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
        "Tab/Shift+Tab: field  Enter: next/save  Esc: cancel"
    }

    fn tunnel(&self) -> Result<Tunnel> {
        let source_port = parse_form_port(&self.fields[0], self.labels()[0])?;
        let requested_port = if self.fields[1].is_empty() {
            source_port
        } else {
            parse_form_port(&self.fields[1], self.labels()[1])?
        };
        let label = self.fields[2].trim().to_owned();
        Ok(match self.direction {
            Direction::Local => Tunnel::manual_local(source_port, requested_port, label),
            Direction::Reverse => Tunnel::manual_reverse(source_port, requested_port, label),
        })
    }
}

struct ScanGuard(Arc<AtomicBool>);

impl Drop for ScanGuard {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
    }
}

struct TerminalGuard;

impl Drop for TerminalGuard {
    fn drop(&mut self) {
        let _ = disable_raw_mode();
        let _ = execute!(io::stdout(), LeaveAlternateScreen, Show);
    }
}

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
        .map(tunnel_from_listener)
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
    for tunnel in tunnels
        .iter_mut()
        .filter(|t| t.direction == Direction::Reverse)
    {
        enable(session, tunnel, &mut message);
    }

    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen)?;
    let _terminal_guard = TerminalGuard;
    let mut terminal = Terminal::new(CrosstermBackend::new(stdout))?;
    let (scan_tx, scan_rx) = mpsc::channel::<ScanEvent>();
    let scanning = Arc::new(AtomicBool::new(true));
    let scanner_flag = Arc::clone(&scanning);
    let interval = Duration::from_secs(interval_seconds);
    thread::scope(|scope| -> Result<()> {
        let _scan_guard = ScanGuard(Arc::clone(&scanning));
        let scanner = scope.spawn(|| {
            let mut next_scan = Instant::now() + interval;
            while scanner_flag.load(Ordering::Relaxed) {
                if Instant::now() >= next_scan {
                    let event = match session.discover_ports(include_loopback) {
                        Ok(ports) => ScanEvent::Ports(ports),
                        Err(scan_error) => match session.reconnect_if_needed() {
                            Ok(true) => match session.discover_ports(include_loopback) {
                                Ok(ports) => ScanEvent::Reconnected(ports),
                                Err(error) => ScanEvent::Error(format!(
                                    "reconnected but scan failed: {error:#}"
                                )),
                            },
                            Ok(false) => ScanEvent::Error(format!("{scan_error:#}")),
                            Err(error) => ScanEvent::Error(format!(
                                "disconnected; reconnect failed: {error:#}"
                            )),
                        },
                    };
                    if scan_tx.send(event).is_err() {
                        break;
                    }
                    next_scan = Instant::now() + interval;
                }
                thread::sleep(Duration::from_millis(100));
            }
        });

        let mut form = None::<InlineForm>;
        let mut show_help = false;
        let result = loop {
            terminal.draw(|frame| {
            let form_height = if form.is_some() { 5 } else { 0 };
            let areas = Layout::vertical([
                Constraint::Min(3),
                Constraint::Length(form_height),
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

            let rows = tunnels.iter().map(|t| {
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
                let url = tunnel_url(t);
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
                    Constraint::Length(12), // Remote port
                    Constraint::Length(12), // Local port
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
                    .title(format!("{title_leader}autotun"))
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

            let footer = if form.is_some() {
                format!("{gutter_pad}{}  │  {message}", InlineForm::help_line())
            } else if show_help {
                format!(
                    "{gutter_pad}↑↓ Select  Space Toggle  a Forward  v Reverse  e Edit  d Remove  r Rescan  ? Help  q Quit  │  {message}"
                )
            } else {
                format!("{gutter_pad}? Help  │  {message}")
            };
            frame.render_widget(
                Paragraph::new(Line::from(footer))
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
                if key.code == KeyCode::Char('c')
                    && key.modifiers.contains(KeyModifiers::CONTROL)
                {
                    break Ok(());
                }
                if let Some(active_form) = form.as_mut() {
                    match handle_inline_form(key.code, active_form) {
                        Ok(Some(mut tunnel)) => {
                            if let Some(index) = active_form.edit_index {
                                apply_edit(
                                    session,
                                    &mut tunnels,
                                    index,
                                    tunnel,
                                    active_form.was_enabled,
                                    &mut message,
                                );
                            } else {
                                enable(session, &mut tunnel, &mut message);
                                tunnels.push(tunnel);
                                state.select(Some(tunnels.len() - 1));
                            }
                            form = None;
                        }
                        Ok(None) => {}
                        Err(error) => message = format!("form error: {error:#}"),
                    }
                    if matches!(key.code, KeyCode::Esc) {
                        let editing = form.as_ref().is_some_and(|f| f.edit_index.is_some());
                        form = None;
                        message = if editing {
                            "edit cancelled".into()
                        } else {
                            "add cancelled".into()
                        };
                    }
                } else {
                    match key.code {
                        KeyCode::Char('q') => break Ok(()),
                        KeyCode::Esc => {}
                        KeyCode::Down | KeyCode::Char('j') => {
                            move_selection(&mut state, tunnels.len(), 1)
                        }
                        KeyCode::Up | KeyCode::Char('k') => {
                            move_selection(&mut state, tunnels.len(), -1)
                        }
                        KeyCode::Char(' ') => {
                            if let Some(i) = state.selected() {
                                toggle(session, &mut tunnels[i], &mut message);
                            }
                        }
                        KeyCode::Enter | KeyCode::Char('e') => {
                            if let Some(i) = state.selected() {
                                form = Some(InlineForm::edit(&tunnels[i], i));
                            }
                        }
                        KeyCode::Char('a') => form = Some(InlineForm::new(Direction::Local)),
                        KeyCode::Char('v') => form = Some(InlineForm::new(Direction::Reverse)),
                        KeyCode::Char('d') => {
                            if let Some(i) = state.selected() {
                                delete_tunnel(session, &mut tunnels, i, &mut message);
                                state.select(
                                    (!tunnels.is_empty()).then_some(i.min(tunnels.len() - 1)),
                                );
                            }
                        }
                        KeyCode::Char('r') => match session.discover_ports(include_loopback) {
                            Ok(found) => reconcile_scan(
                                session,
                                &mut tunnels,
                                found,
                                auto_forward,
                                &mut message,
                            ),
                            Err(e) => message = format!("refresh failed: {e:#}"),
                        },
                        KeyCode::Char('?') => {
                            show_help = !show_help;
                            message = if show_help {
                                "help shown".into()
                            } else {
                                "help hidden".into()
                            };
                        }
                        _ => {}
                    }
                }
            }
            while let Ok(scan) = scan_rx.try_recv() {
                match scan {
                    ScanEvent::Ports(found) => {
                        reconcile_scan(session, &mut tunnels, found, auto_forward, &mut message)
                    }
                    ScanEvent::Reconnected(found) => restore_after_reconnect(
                        session,
                        &mut tunnels,
                        found,
                        auto_forward,
                        &mut message,
                    ),
                    ScanEvent::Error(error) => {
                        message = format!("scan error; tunnels kept: {error}")
                    }
                }
            }
        };
        scanning.store(false, Ordering::Relaxed);
        scanner.join().expect("scanner thread panicked");
        result
    })
}

fn handle_inline_form(key: KeyCode, form: &mut InlineForm) -> Result<Option<Tunnel>> {
    match key {
        KeyCode::Tab | KeyCode::Down => form.selected = (form.selected + 1) % form.fields.len(),
        KeyCode::BackTab | KeyCode::Up => {
            form.selected = (form.selected + form.fields.len() - 1) % form.fields.len()
        }
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

fn parse_form_port(value: &str, name: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| format!("{name} must be a TCP port from 1 to 65535"))?;
    if port == 0 {
        anyhow::bail!("{name} must be a TCP port from 1 to 65535");
    }
    Ok(port)
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
    let preferred = tunnel.requested_port;
    if tunnel.direction == Direction::Reverse {
        let mut last_error = None;
        for offset in 0..=MAX_PORT_FALLBACKS {
            let Some(port) = preferred.checked_add(offset) else {
                break;
            };
            match session.forward(Direction::Reverse, port, tunnel.source_port) {
                Ok(()) => {
                    tunnel.bind_port = Some(port);
                    tunnel.enabled = true;
                    *message = format!("reverse port {port} enabled");
                    return;
                }
                Err(error) => last_error = Some(error),
            }
        }
        tunnel.error = Some(format!(
            "ports {preferred}..{} failed: {:#}",
            preferred.saturating_add(MAX_PORT_FALLBACKS),
            last_error.expect("at least one reverse port attempt")
        ));
        return;
    }

    match available_local_port(preferred) {
        Ok((port, reservation)) => {
            drop(reservation);
            match session.forward(Direction::Local, port, tunnel.source_port) {
                Ok(()) => {
                    tunnel.bind_port = Some(port);
                    tunnel.enabled = true;
                    tunnel.protocol = detect_protocol(port);
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

fn tunnel_url(tunnel: &Tunnel) -> String {
    if tunnel.direction == Direction::Local
        && tunnel.enabled
        && let Some(port) = tunnel.bind_port
    {
        format!("{}://127.0.0.1:{port}", tunnel.protocol.as_str())
    } else {
        "—".into()
    }
}

fn detect_protocol(port: u16) -> Protocol {
    // TLS endpoints are reported as https. Distinguishing wss would need a full
    // TLS handshake, which is intentionally out of scope for this probe.
    if probe_tls(port) {
        return Protocol::Https;
    }
    if probe_websocket(port) {
        return Protocol::Ws;
    }
    if probe_http(port) {
        return Protocol::Http;
    }
    Protocol::Tcp
}

fn open_probe_stream(port: u16) -> Option<TcpStream> {
    let address = SocketAddr::from(([127, 0, 0, 1], port));
    let timeout = Duration::from_millis(250);
    let stream = TcpStream::connect_timeout(&address, timeout).ok()?;
    stream.set_read_timeout(Some(timeout)).ok()?;
    stream.set_write_timeout(Some(timeout)).ok()?;
    Some(stream)
}

fn probe_tls(port: u16) -> bool {
    // A minimal TLS 1.2 ClientHello. A TLS ServerHello or TLS alert is enough
    // to classify the endpoint; no certificate is trusted or retained.
    let client_hello = [
        0x16, 0x03, 0x01, 0x00, 0x2f, 0x01, 0x00, 0x00, 0x2b, 0x03, 0x03, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00,
        0x00, 0x04, 0x00, 0x2f, 0x00, 0x35, 0x01, 0x00,
    ];
    let Some(mut stream) = open_probe_stream(port) else {
        return false;
    };
    if stream.write_all(&client_hello).is_err() {
        return false;
    }
    let mut record = [0_u8; 5];
    stream.read_exact(&mut record).is_ok() && matches!(record[0], 0x15 | 0x16) && record[1] == 0x03
}

fn probe_http(port: u16) -> bool {
    let Some(mut stream) = open_probe_stream(port) else {
        return false;
    };
    if stream
        .write_all(b"HEAD / HTTP/1.0\r\nHost: 127.0.0.1\r\n\r\n")
        .is_err()
    {
        return false;
    }
    let mut prefix = [0_u8; 5];
    stream.read_exact(&mut prefix).is_ok() && prefix == *b"HTTP/"
}

fn probe_websocket(port: u16) -> bool {
    let Some(mut stream) = open_probe_stream(port) else {
        return false;
    };
    let request = concat!(
        "GET / HTTP/1.1\r\n",
        "Host: 127.0.0.1\r\n",
        "Upgrade: websocket\r\n",
        "Connection: Upgrade\r\n",
        "Sec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\n",
        "Sec-WebSocket-Version: 13\r\n",
        "\r\n",
    );
    if stream.write_all(request.as_bytes()).is_err() {
        return false;
    }
    let mut buf = [0_u8; 32];
    match stream.read(&mut buf) {
        Ok(n) if n >= 12 => {
            let text = String::from_utf8_lossy(&buf[..n]);
            text.starts_with("HTTP/1.1 101") || text.starts_with("HTTP/1.0 101")
        }
        _ => false,
    }
}

fn tunnel_from_listener(listener: RemoteListener) -> Tunnel {
    let mut tunnel = Tunnel::local(listener.port);
    if let Some(process) = listener.process {
        tunnel.label = process;
    }
    tunnel
}

fn apply_auto_label(tunnel: &mut Tunnel, process: Option<&str>) {
    if tunnel.label.is_empty()
        && let Some(process) = process
        && !process.is_empty()
    {
        tunnel.label = process.to_owned();
    }
}

fn apply_edit(
    session: &SshSession,
    tunnels: &mut [Tunnel],
    index: usize,
    mut replacement: Tunnel,
    was_enabled: bool,
    message: &mut String,
) {
    let Some(existing) = tunnels.get(index) else {
        *message = "edit failed: tunnel disappeared".into();
        return;
    };
    if was_enabled {
        let Some(bind) = existing.bind_port else {
            *message = "edit failed: enabled tunnel has no bind port".into();
            return;
        };
        if let Err(error) = session.cancel(existing.direction, bind, existing.source_port) {
            tunnels[index].error = Some(format!("edit cancel failed: {error:#}"));
            return;
        }
    }
    replacement.discovered = existing.discovered;
    replacement.present = existing.present;
    if was_enabled {
        enable(session, &mut replacement, message);
        if replacement.error.is_none() {
            *message = "tunnel updated".into();
        }
    } else {
        replacement.manual_off = existing.manual_off;
        *message = "tunnel updated".into();
    }
    tunnels[index] = replacement;
}

fn delete_tunnel(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    index: usize,
    message: &mut String,
) {
    if tunnels[index].enabled {
        let tunnel = &tunnels[index];
        let bind = tunnel.bind_port.expect("enabled tunnel has bind port");
        if let Err(error) = session.cancel(tunnel.direction, bind, tunnel.source_port) {
            tunnels[index].error = Some(format!("delete failed: {error:#}"));
            return;
        }
    }
    if tunnels[index].discovered {
        tunnels[index].enabled = false;
        tunnels[index].manual_off = true;
        tunnels[index].error = None;
        *message = "discovered tunnel ignored for this session".into();
    } else {
        tunnels.remove(index);
        *message = "manual tunnel deleted".into();
    }
}

fn reconcile_scan(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    found: Vec<RemoteListener>,
    auto_forward: bool,
    message: &mut String,
) {
    for tunnel in tunnels
        .iter_mut()
        .filter(|t| t.direction == Direction::Local)
    {
        if let Some(listener) = found.iter().find(|l| l.port == tunnel.source_port) {
            tunnel.present = true;
            tunnel.missing_scans = 0;
            apply_auto_label(tunnel, listener.process.as_deref());
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

    for listener in found {
        if tunnels
            .iter()
            .any(|t| t.direction == Direction::Local && t.source_port == listener.port)
        {
            continue;
        }
        let mut tunnel = tunnel_from_listener(listener);
        if auto_forward {
            enable(session, &mut tunnel, message);
        }
        tunnels.push(tunnel);
    }
}

fn restore_after_reconnect(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    found: Vec<RemoteListener>,
    auto_forward: bool,
    message: &mut String,
) {
    let wanted = tunnels.iter().map(|t| t.enabled).collect::<Vec<_>>();
    for tunnel in tunnels.iter_mut() {
        tunnel.enabled = false;
        tunnel.bind_port = None;
        tunnel.error = None;
    }
    reconcile_scan(session, tunnels, found, auto_forward, message);
    for (index, was_enabled) in wanted.into_iter().enumerate() {
        if was_enabled
            && let Some(tunnel) = tunnels.get_mut(index)
            && !tunnel.enabled
            && (tunnel.direction == Direction::Reverse || tunnel.present)
        {
            enable(session, tunnel, message);
        }
    }
    *message = "SSH reconnected; active tunnels restored".into();
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

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

    #[test]
    fn detects_http_tls_and_websocket_protocols() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            // Probes: TLS (fail), WebSocket (fail), HTTP (match).
            for _ in 0..3 {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 256];
                let _ = stream.read(&mut request);
                stream.write_all(b"HTTP/1.0 200 OK\r\n\r\n").unwrap();
            }
        });
        assert_eq!(detect_protocol(port), Protocol::Http);
        server.join().unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = [0_u8; 128];
            let _ = stream.read(&mut request);
            stream.write_all(&[0x16, 0x03, 0x03, 0x00, 0x00]).unwrap();
        });
        assert_eq!(detect_protocol(port), Protocol::Https);
        server.join().unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        let server = thread::spawn(move || {
            // TLS ClientHello first (ignored), then WebSocket upgrade.
            for response in [
                &b"HTTP/1.1 400 Bad Request\r\n\r\n"[..],
                &b"HTTP/1.1 101 Switching Protocols\r\n\r\n"[..],
            ] {
                let (mut stream, _) = listener.accept().unwrap();
                let mut request = [0_u8; 512];
                let _ = stream.read(&mut request);
                stream.write_all(response).unwrap();
            }
        });
        assert_eq!(detect_protocol(port), Protocol::Ws);
        server.join().unwrap();

        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let port = listener.local_addr().unwrap().port();
        // No accept — connection fails / times out → plain TCP.
        assert_eq!(detect_protocol(port), Protocol::Tcp);
        drop(listener);
    }

    #[test]
    fn tunnel_url_includes_protocol_for_enabled_local() {
        let mut tunnel = Tunnel::local(3000);
        tunnel.enabled = true;
        tunnel.bind_port = Some(3000);
        tunnel.protocol = Protocol::Https;
        assert_eq!(tunnel_url(&tunnel), "https://127.0.0.1:3000");
        tunnel.enabled = false;
        assert_eq!(tunnel_url(&tunnel), "—");
    }
}
