use std::net::TcpListener;

use anyhow::{Result, bail};

pub const MAX_PORT_FALLBACKS: u16 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Local,
    Reverse,
}

/// Application-layer protocol guessed for a forwarded TCP service.
///
/// SSH tunnels are always TCP; this describes what speaks on top of that
/// stream when we can detect it (HTTP(S), WebSocket, or plain TCP).
/// `Wss` is reserved for TLS WebSocket endpoints once a full TLS probe exists.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Protocol {
    Tcp,
    Http,
    Https,
    Ws,
    #[allow(dead_code)]
    Wss,
}

impl Protocol {
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Http => "http",
            Self::Https => "https",
            Self::Ws => "ws",
            Self::Wss => "wss",
        }
    }
}

#[derive(Clone, Debug)]
pub struct Tunnel {
    pub direction: Direction,
    pub source_port: u16,
    pub bind_port: Option<u16>,
    pub enabled: bool,
    pub error: Option<String>,
    pub present: bool,
    pub missing_scans: u8,
    pub manual_off: bool,
    pub requested_port: u16,
    pub label: String,
    pub discovered: bool,
    pub protocol: Protocol,
}

impl Tunnel {
    pub fn local(remote_port: u16) -> Self {
        Self {
            direction: Direction::Local,
            source_port: remote_port,
            bind_port: None,
            enabled: false,
            error: None,
            present: true,
            missing_scans: 0,
            manual_off: false,
            requested_port: remote_port,
            label: String::new(),
            discovered: true,
            protocol: Protocol::Tcp,
        }
    }

    pub fn reverse(local_port: u16) -> Self {
        Self {
            direction: Direction::Reverse,
            source_port: local_port,
            bind_port: None,
            enabled: false,
            error: None,
            present: true,
            missing_scans: 0,
            manual_off: false,
            requested_port: local_port,
            label: String::new(),
            discovered: false,
            protocol: Protocol::Tcp,
        }
    }

    pub fn manual_local(remote_port: u16, local_port: u16, label: String) -> Self {
        Self {
            direction: Direction::Local,
            source_port: remote_port,
            bind_port: None,
            enabled: false,
            error: None,
            present: true,
            missing_scans: 0,
            manual_off: false,
            requested_port: local_port,
            label,
            discovered: false,
            protocol: Protocol::Tcp,
        }
    }

    pub fn manual_reverse(local_port: u16, remote_port: u16, label: String) -> Self {
        let mut tunnel = Self::reverse(local_port);
        tunnel.requested_port = remote_port;
        tunnel.label = label;
        tunnel
    }
}

pub fn available_local_port(preferred: u16) -> Result<(u16, TcpListener)> {
    for offset in 0..=MAX_PORT_FALLBACKS {
        let Some(port) = preferred.checked_add(offset) else {
            break;
        };
        if let Ok(listener) = TcpListener::bind(("127.0.0.1", port)) {
            return Ok((port, listener));
        }
    }
    bail!(
        "ports {preferred} through {} are unavailable",
        preferred.saturating_add(MAX_PORT_FALLBACKS)
    )
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RemoteListener {
    pub port: u16,
    pub process: Option<String>,
}

pub fn parse_ss_listeners(output: &str, include_loopback: bool) -> Vec<RemoteListener> {
    use std::collections::BTreeMap;

    let mut by_port = BTreeMap::<u16, Option<String>>::new();
    for line in output.lines() {
        let Some((port, process)) = parse_ss_line(line, include_loopback) else {
            continue;
        };
        by_port
            .entry(port)
            .and_modify(|existing| {
                if existing.is_none() {
                    *existing = process.clone();
                }
            })
            .or_insert(process);
    }
    by_port
        .into_iter()
        .map(|(port, process)| RemoteListener { port, process })
        .collect()
}

fn parse_ss_line(line: &str, include_loopback: bool) -> Option<(u16, Option<String>)> {
    let mut local_port = None;
    let mut process = None;
    for token in line.split_whitespace() {
        if let Some(name) = parse_ss_process(token) {
            process = Some(name);
            continue;
        }
        if local_port.is_none()
            && let Some(port) = parse_ss_local_port(token, include_loopback)
        {
            local_port = Some(port);
        }
    }
    Some((local_port?, process))
}

fn parse_ss_local_port(token: &str, include_loopback: bool) -> Option<u16> {
    // Require host:port form. Bare numbers are Recv-Q/Send-Q columns from ss
    // (e.g. "LISTEN 0 4096 127.0.0.53%lo:53") and must not be treated as ports.
    if !token.contains(':') {
        return None;
    }
    if !include_loopback && (token.starts_with("127.") || token.starts_with("[::1]")) {
        return None;
    }
    let port = token.rsplit(':').next()?.parse::<u16>().ok()?;
    (port > 1024).then_some(port)
}

fn parse_ss_process(token: &str) -> Option<String> {
    // ss -p appends: users:(("name",pid=123,fd=4))
    let rest = token.strip_prefix("users:((")?;
    let name = rest.strip_prefix('"')?.split('"').next()?;
    (!name.is_empty()).then(|| name.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_deduplicates_ss() {
        let input = "LISTEN 0 128 127.0.0.1:3000\nLISTEN 0 128 0.0.0.0:22\nLISTEN 0 128 [::]:22\n";
        assert_eq!(
            parse_ss_listeners(input, true),
            vec![RemoteListener {
                port: 3000,
                process: None
            }]
        );
        assert!(parse_ss_listeners(input, false).is_empty());
    }

    #[test]
    fn ignores_ss_queue_sizes_that_look_like_ports() {
        // Real ss -lntp line: columns are State Recv-Q Send-Q Local Peer [Process].
        // Send-Q is often 4096; that must not become a discovered listener.
        let input = concat!(
            "LISTEN 0 4096 127.0.0.53%lo:53 0.0.0.0:*\n",
            "LISTEN 0 4096 127.0.0.1:35633 0.0.0.0:* users:((\"agy\",pid=1,fd=2))\n",
            "LISTEN 0 128 0.0.0.0:22 0.0.0.0:*\n",
        );
        assert_eq!(
            parse_ss_listeners(input, true),
            vec![RemoteListener {
                port: 35633,
                process: Some("agy".into()),
            }]
        );
    }

    #[test]
    fn parses_process_names_from_ss_p() {
        let input = concat!(
            "LISTEN 0 128 127.0.0.1:35633 0.0.0.0:* users:((\"agy\",pid=4102,fd=13))\n",
            "LISTEN 0 128 127.0.0.1:35633 0.0.0.0:* users:((\"agy\",pid=4102,fd=11))\n",
            "LISTEN 0 128 0.0.0.0:3000 0.0.0.0:* users:((\"node\",pid=9,fd=23))\n",
        );
        let listeners = parse_ss_listeners(input, true);
        assert_eq!(
            listeners,
            vec![
                RemoteListener {
                    port: 3000,
                    process: Some("node".into()),
                },
                RemoteListener {
                    port: 35633,
                    process: Some("agy".into()),
                },
            ]
        );
    }

    #[test]
    fn changes_port_when_preferred_is_busy() {
        let busy = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let preferred = busy.local_addr().unwrap().port();
        let (actual, _reservation) = available_local_port(preferred).unwrap();
        assert_eq!(actual, preferred + 1);
    }

    #[test]
    fn gives_up_after_five_fallbacks() {
        let base = (20_000..60_000)
            .find(|base| {
                (0..=MAX_PORT_FALLBACKS)
                    .all(|offset| TcpListener::bind(("127.0.0.1", base + offset)).is_ok())
            })
            .expect("six consecutive test ports");
        let listeners = (0..=MAX_PORT_FALLBACKS)
            .map(|offset| TcpListener::bind(("127.0.0.1", base + offset)).unwrap())
            .collect::<Vec<_>>();
        let error = available_local_port(base).unwrap_err();
        assert!(error.to_string().contains("are unavailable"));
        drop(listeners);
    }
}
