use std::net::TcpListener;

use anyhow::{Result, bail};

pub const MAX_PORT_FALLBACKS: u16 = 5;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Direction {
    Local,
    Reverse,
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

pub fn parse_ss_ports(output: &str, include_loopback: bool) -> Vec<u16> {
    let mut ports = output
        .lines()
        .filter_map(|line| {
            let addr = line.split_whitespace().last()?;
            if !include_loopback && (addr.starts_with("127.") || addr.starts_with("[::1]")) {
                return None;
            }
            let port = addr.rsplit(':').next()?.parse::<u16>().ok()?;
            (port > 1024).then_some(port)
        })
        .collect::<Vec<_>>();
    ports.sort_unstable();
    ports.dedup();
    ports
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_and_deduplicates_ss() {
        let input = "LISTEN 0 128 127.0.0.1:3000\nLISTEN 0 128 0.0.0.0:22\nLISTEN 0 128 [::]:22\n";
        assert_eq!(parse_ss_ports(input, true), vec![3000]);
        assert!(parse_ss_ports(input, false).is_empty());
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
