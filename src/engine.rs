//! Session controller shared by the TUI and GUI.
//!
//! Owns the SSH ControlMaster, tunnel list, and background listener scan.
//! Frontends only render state and call the methods here.

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result};

use crate::{
    ports::{
        Direction, MAX_PORT_FALLBACKS, Protocol, RemoteListener, Tunnel, available_local_port,
    },
    scan::{ScanAction, plan_scan, tunnel_from_listener},
    ssh::SshSession,
};

enum ScanEvent {
    Ports(Vec<RemoteListener>),
    Reconnected(Vec<RemoteListener>),
    Error,
}

/// Live SSH session plus the tunnels discovered or created on it.
pub struct Engine {
    session: Arc<SshSession>,
    tunnels: Vec<Tunnel>,
    connected: bool,
    auto_forward: bool,
    include_loopback: bool,
    scan_rx: mpsc::Receiver<ScanEvent>,
    scanning: Arc<AtomicBool>,
    scanner: Option<thread::JoinHandle<()>>,
}

impl Engine {
    pub fn connect(
        destination: String,
        extra_args: Vec<String>,
        reverse_ports: &[u16],
        include_loopback: bool,
        auto_forward: bool,
        interval_seconds: u64,
    ) -> Result<Self> {
        let nonce = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos();
        let socket =
            std::env::temp_dir().join(format!("autotun-{}-{nonce}.sock", std::process::id()));
        let session = SshSession::connect(destination, socket, extra_args)?;
        Self::start(
            session,
            reverse_ports,
            include_loopback,
            auto_forward,
            interval_seconds,
        )
    }

    pub fn start(
        session: SshSession,
        reverse_ports: &[u16],
        include_loopback: bool,
        auto_forward: bool,
        interval_seconds: u64,
    ) -> Result<Self> {
        let mut tunnels = session
            .discover_ports(include_loopback)?
            .into_iter()
            .map(tunnel_from_listener)
            .collect::<Vec<_>>();
        tunnels.extend(reverse_ports.iter().copied().map(Tunnel::reverse));

        if auto_forward {
            for tunnel in tunnels
                .iter_mut()
                .filter(|t| t.direction == Direction::Local)
            {
                enable(&session, tunnel);
            }
        }
        for tunnel in tunnels
            .iter_mut()
            .filter(|t| t.direction == Direction::Reverse)
        {
            enable(&session, tunnel);
        }

        let session = Arc::new(session);
        let (scan_tx, scan_rx) = mpsc::channel::<ScanEvent>();
        let scanning = Arc::new(AtomicBool::new(true));
        let scanner_flag = Arc::clone(&scanning);
        let scanner_session = Arc::clone(&session);
        let interval = Duration::from_secs(interval_seconds);

        let scanner = thread::spawn(move || {
            let mut next_scan = Instant::now() + interval;
            while scanner_flag.load(Ordering::Relaxed) {
                if Instant::now() >= next_scan {
                    let event = match scanner_session.discover_ports(include_loopback) {
                        Ok(ports) => ScanEvent::Ports(ports),
                        Err(_) => match scanner_session.reconnect_if_needed() {
                            Ok(true) => match scanner_session.discover_ports(include_loopback) {
                                Ok(ports) => ScanEvent::Reconnected(ports),
                                Err(_) => ScanEvent::Error,
                            },
                            Ok(false) => ScanEvent::Error,
                            Err(_) => ScanEvent::Error,
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

        Ok(Self {
            session,
            tunnels,
            connected: true,
            auto_forward,
            include_loopback,
            scan_rx,
            scanning,
            scanner: Some(scanner),
        })
    }

    pub fn destination(&self) -> &str {
        self.session.destination()
    }

    pub fn connected(&self) -> bool {
        self.connected
    }

    pub fn tunnels(&self) -> &[Tunnel] {
        &self.tunnels
    }

    /// Apply any background scan / reconnect results. Call from the UI loop.
    pub fn poll(&mut self) {
        while let Ok(scan) = self.scan_rx.try_recv() {
            match scan {
                ScanEvent::Ports(found) => {
                    self.connected = true;
                    reconcile_scan(&self.session, &mut self.tunnels, found, self.auto_forward);
                }
                ScanEvent::Reconnected(found) => {
                    self.connected = true;
                    restore_after_reconnect(
                        &self.session,
                        &mut self.tunnels,
                        found,
                        self.auto_forward,
                    );
                }
                ScanEvent::Error => {
                    self.connected = false;
                }
            }
        }
    }

    pub fn toggle(&mut self, index: usize) {
        if let Some(tunnel) = self.tunnels.get_mut(index) {
            toggle(&self.session, tunnel);
        }
    }

    pub fn add(&mut self, mut tunnel: Tunnel) -> usize {
        enable(&self.session, &mut tunnel);
        self.tunnels.push(tunnel);
        self.tunnels.len() - 1
    }

    pub fn edit(&mut self, index: usize, replacement: Tunnel, was_enabled: bool) {
        apply_edit(
            &self.session,
            &mut self.tunnels,
            index,
            replacement,
            was_enabled,
        );
    }

    pub fn delete(&mut self, index: usize) {
        if index < self.tunnels.len() {
            delete_tunnel(&self.session, &mut self.tunnels, index);
        }
    }

    pub fn rescan(&mut self) {
        if let Ok(found) = self.session.discover_ports(self.include_loopback) {
            reconcile_scan(&self.session, &mut self.tunnels, found, self.auto_forward);
        }
    }

    pub fn shutdown(&mut self) {
        self.scanning.store(false, Ordering::Relaxed);
        if let Some(scanner) = self.scanner.take() {
            let _ = scanner.join();
        }
    }
}

impl Drop for Engine {
    fn drop(&mut self) {
        self.shutdown();
    }
}

pub fn tunnel_url(tunnel: &Tunnel) -> String {
    if tunnel.direction == Direction::Local
        && tunnel.enabled
        && let Some(port) = tunnel.bind_port
    {
        format!("{}://127.0.0.1:{port}", tunnel.protocol.as_str())
    } else {
        "—".into()
    }
}

pub fn parse_form_port(value: &str, name: &str) -> Result<u16> {
    let port = value
        .parse::<u16>()
        .with_context(|| format!("{name} must be a TCP port from 1 to 65535"))?;
    if port == 0 {
        anyhow::bail!("{name} must be a TCP port from 1 to 65535");
    }
    Ok(port)
}

pub fn tunnel_from_form(
    direction: Direction,
    source: &str,
    requested: &str,
    label: &str,
) -> Result<Tunnel> {
    let (source_name, requested_name) = match direction {
        Direction::Local => ("Remote port", "Local port"),
        Direction::Reverse => ("Local port", "Remote port"),
    };
    let source_port = parse_form_port(source, source_name)?;
    let requested_port = if requested.is_empty() {
        source_port
    } else {
        parse_form_port(requested, requested_name)?
    };
    let label = label.trim().to_owned();
    Ok(match direction {
        Direction::Local => Tunnel::manual_local(source_port, requested_port, label),
        Direction::Reverse => Tunnel::manual_reverse(source_port, requested_port, label),
    })
}

fn toggle(session: &SshSession, tunnel: &mut Tunnel) {
    tunnel.error = None;
    if tunnel.enabled {
        let port = tunnel.bind_port.expect("enabled tunnel has a bind port");
        match session.cancel(tunnel.direction, port, tunnel.source_port) {
            Ok(()) => {
                tunnel.enabled = false;
                tunnel.manual_off = true;
            }
            Err(e) => tunnel.error = Some(format!("cancel failed: {e:#}")),
        }
        return;
    }

    tunnel.manual_off = false;
    enable(session, tunnel);
}

fn enable(session: &SshSession, tunnel: &mut Tunnel) {
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
                }
                Err(e) => tunnel.error = Some(format!("forward failed: {e:#}")),
            }
        }
        Err(e) => tunnel.error = Some(format!("allocation failed: {e:#}")),
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

fn apply_edit(
    session: &SshSession,
    tunnels: &mut [Tunnel],
    index: usize,
    mut replacement: Tunnel,
    was_enabled: bool,
) {
    let Some(existing) = tunnels.get(index) else {
        return;
    };
    if was_enabled {
        let Some(bind) = existing.bind_port else {
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
        enable(session, &mut replacement);
    } else {
        replacement.manual_off = existing.manual_off;
    }
    tunnels[index] = replacement;
}

fn delete_tunnel(session: &SshSession, tunnels: &mut Vec<Tunnel>, index: usize) {
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
    } else {
        tunnels.remove(index);
    }
}

fn reconcile_scan(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    found: Vec<RemoteListener>,
    auto_forward: bool,
) {
    let actions = plan_scan(tunnels, &found, auto_forward);
    for action in actions {
        match action {
            ScanAction::Enable(index) => {
                if let Some(tunnel) = tunnels.get_mut(index) {
                    enable(session, tunnel);
                }
            }
            ScanAction::Cancel {
                index,
                bind,
                source,
            } => match session.cancel(Direction::Local, bind, source) {
                Ok(()) => {
                    if let Some(tunnel) = tunnels.get_mut(index) {
                        tunnel.enabled = false;
                    }
                }
                Err(error) => {
                    if let Some(tunnel) = tunnels.get_mut(index) {
                        tunnel.error = Some(format!("cancel failed: {error:#}"));
                    }
                }
            },
            ScanAction::Discover {
                mut tunnel,
                enable: should_enable,
            } => {
                if should_enable {
                    enable(session, &mut tunnel);
                }
                tunnels.push(tunnel);
            }
        }
    }
}

fn restore_after_reconnect(
    session: &SshSession,
    tunnels: &mut Vec<Tunnel>,
    found: Vec<RemoteListener>,
    auto_forward: bool,
) {
    let wanted = tunnels.iter().map(|t| t.enabled).collect::<Vec<_>>();
    for tunnel in tunnels.iter_mut() {
        tunnel.enabled = false;
        tunnel.bind_port = None;
        tunnel.error = None;
    }
    reconcile_scan(session, tunnels, found, auto_forward);
    for (index, was_enabled) in wanted.into_iter().enumerate() {
        if was_enabled
            && let Some(tunnel) = tunnels.get_mut(index)
            && !tunnel.enabled
            && (tunnel.direction == Direction::Reverse || tunnel.present)
        {
            enable(session, tunnel);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{net::TcpListener, thread};

    #[test]
    fn form_local_uses_service_port_as_default_bind_port() {
        let tunnel = tunnel_from_form(Direction::Local, "3000", "", "frontend").unwrap();
        assert_eq!(tunnel.direction, Direction::Local);
        assert_eq!(tunnel.source_port, 3000);
        assert_eq!(tunnel.requested_port, 3000);
        assert_eq!(tunnel.label, "frontend");
    }

    #[test]
    fn form_rejects_zero_port() {
        assert!(tunnel_from_form(Direction::Reverse, "0", "", "").is_err());
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
