//! Live SSH lifecycle checks against a reachable destination.
//!
//! Opt-in only (needs network + credentials):
//!
//! ```sh
//! AUTOTUN_SSH_TEST=1 cargo test --test ssh_lifecycle -- --nocapture
//! # optional override (default: thuanlee@vm)
//! AUTOTUN_SSH_DEST=user@host cargo test --test ssh_lifecycle -- --nocapture
//! ```

use std::{
    io::{Read, Write},
    net::{SocketAddr, TcpStream},
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
    thread,
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use autotun::ports::{Direction, Tunnel, parse_ss_listeners};
use autotun::scan::{ScanAction, apply_scan_in_memory, plan_scan, tunnel_from_listener};
use autotun::ssh::SshSession;

/// Serialize live tests so temporary remote listeners do not race each other.
static LIVE_LOCK: Mutex<()> = Mutex::new(());

fn enabled() -> bool {
    matches!(
        std::env::var("AUTOTUN_SSH_TEST").as_deref(),
        Ok("1") | Ok("true") | Ok("yes")
    )
}

fn live_guard() -> std::sync::MutexGuard<'static, ()> {
    LIVE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

fn destination() -> String {
    std::env::var("AUTOTUN_SSH_DEST").unwrap_or_else(|_| "thuanlee@vm".into())
}

fn unique_socket() -> PathBuf {
    let nonce = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_nanos();
    std::env::temp_dir().join(format!(
        "autotun-itest-{}-{nonce}.sock",
        std::process::id()
    ))
}

/// Pick a free high port on the remote and start a short-lived TCP listener.
fn start_remote_listener(dest: &str) -> (u16, String /* marker path for cleanup */) {
    // Encode the helper in base64 so newlines/quotes survive the SSH argv path.
    let script = r#"
import socket, time, os
s = socket.socket()
s.setsockopt(socket.SOL_SOCKET, socket.SO_REUSEADDR, 1)
s.bind(("127.0.0.1", 0))
port = s.getsockname()[1]
marker = f"/tmp/autotun-itest-{os.getpid()}-{port}"
open(marker, "w").write(str(os.getpid()))
print(f"PORT={port}", flush=True)
print(f"MARKER={marker}", flush=True)
s.listen(1)
deadline = time.time() + 60
while time.time() < deadline and os.path.exists(marker):
    s.settimeout(0.5)
    try:
        c, _ = s.accept()
        c.sendall(b"HTTP/1.0 200 OK\r\nContent-Length: 2\r\n\r\nok")
        c.close()
    except Exception:
        pass
s.close()
"#;
    let b64 = base64_encode(script.as_bytes());
    // Pass the pipeline as a single remote command string. Using `bash -lc`
    // here swallows stdout on some OpenSSH/login-shell combos.
    let remote = format!("echo {b64} | base64 -d | python3 -u");
    let mut child = Command::new("ssh")
        .args([dest, &remote])
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("ssh spawn");

    let mut port = None;
    let mut marker = None;
    let start = std::time::Instant::now();
    let stdout = child.stdout.take().expect("stdout");
    let mut reader = std::io::BufReader::new(stdout);
    use std::io::BufRead;
    while start.elapsed() < Duration::from_secs(10) {
        let mut line = String::new();
        if reader.read_line(&mut line).ok().filter(|n| *n > 0).is_none() {
            if child.try_wait().ok().flatten().is_some() {
                break;
            }
            thread::sleep(Duration::from_millis(50));
            continue;
        }
        let line = line.trim();
        if let Some(p) = line.strip_prefix("PORT=") {
            port = Some(p.parse::<u16>().expect("port"));
        }
        if let Some(m) = line.strip_prefix("MARKER=") {
            marker = Some(m.to_string());
        }
        if port.is_some() && marker.is_some() {
            break;
        }
    }
    let port = port.expect("remote listener did not print PORT=");
    let marker = marker.expect("remote listener did not print MARKER=");
    // Detach: leave the remote python running; we clean via marker removal.
    std::mem::forget(child);
    (port, marker)
}

fn base64_encode(input: &[u8]) -> String {
    const TABLE: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::new();
    for chunk in input.chunks(3) {
        let mut buf = [0u8; 3];
        for (i, b) in chunk.iter().enumerate() {
            buf[i] = *b;
        }
        let n = chunk.len();
        let x = u32::from(buf[0]) << 16 | u32::from(buf[1]) << 8 | u32::from(buf[2]);
        out.push(TABLE[((x >> 18) & 63) as usize] as char);
        out.push(TABLE[((x >> 12) & 63) as usize] as char);
        if n > 1 {
            out.push(TABLE[((x >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if n > 2 {
            out.push(TABLE[(x & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

fn stop_remote_listener(dest: &str, marker: &str) {
    let _ = Command::new("ssh")
        .args([dest, "rm", "-f", marker])
        .status();
    // Give the python loop a moment to exit.
    thread::sleep(Duration::from_millis(300));
}

fn wait_until(pred: impl Fn() -> bool, timeout: Duration) -> bool {
    let start = std::time::Instant::now();
    while start.elapsed() < timeout {
        if pred() {
            return true;
        }
        thread::sleep(Duration::from_millis(100));
    }
    false
}

#[test]
fn live_discover_and_forward_roundtrip() {
    if !enabled() {
        eprintln!("skip: set AUTOTUN_SSH_TEST=1 to run");
        return;
    }
    let _guard = live_guard();
    let dest = destination();
    let (port, marker) = start_remote_listener(&dest);

    let socket = unique_socket();
    let mut session = SshSession::connect(dest.clone(), socket, vec![]).expect("ssh connect");

    let found = session.discover_ports(true).expect("discover");
    assert!(
        found.iter().any(|l| l.port == port),
        "expected remote port {port} in discover results: {found:?}"
    );

    // Plan auto-enable for a fresh tunnel list (all discovered, none enabled yet).
    let mut tunnels: Vec<Tunnel> = found.iter().cloned().map(tunnel_from_listener).collect();
    let actions = plan_scan(&mut tunnels, &found, true);
    let enable_idx = tunnels
        .iter()
        .position(|t| t.source_port == port)
        .expect("tunnel row for test port");
    assert!(
        actions.iter().any(|a| matches!(a, ScanAction::Enable(i) if *i == enable_idx)),
        "expected Enable({enable_idx}) for port {port}, got {actions:?}"
    );

    // Real forward.
    session
        .forward(Direction::Local, port, port)
        .expect("forward");
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    assert!(
        wait_until(
            || TcpStream::connect_timeout(&addr, Duration::from_millis(200)).is_ok(),
            Duration::from_secs(3)
        ),
        "local bind {port} never became reachable"
    );

    let mut stream = TcpStream::connect(addr).expect("connect through tunnel");
    stream.write_all(b"GET / HTTP/1.0\r\n\r\n").unwrap();
    let mut buf = [0u8; 64];
    let n = stream.read(&mut buf).unwrap_or(0);
    let body = String::from_utf8_lossy(&buf[..n]);
    assert!(
        body.contains("200") || body.contains("ok"),
        "unexpected response through tunnel: {body:?}"
    );

    session.cancel(Direction::Local, port, port).expect("cancel");
    stop_remote_listener(&dest, &marker);
    session.close();
}

#[test]
fn live_ss_output_does_not_invent_queue_ports() {
    if !enabled() {
        eprintln!("skip: set AUTOTUN_SSH_TEST=1 to run");
        return;
    }
    let _guard = live_guard();
    let dest = destination();
    let socket = unique_socket();
    let mut session = SshSession::connect(dest, socket, vec![]).expect("ssh connect");
    let found = session.discover_ports(true).expect("discover");
    // Regression: Send-Q 4096 on DNS must never appear as a listener.
    assert!(
        !found.iter().any(|l| l.port == 4096),
        "bogus port 4096 from ss Send-Q still present: {found:?}"
    );
    // Infrastructure privileged ports stay filtered; 80/443 may appear.
    assert!(
        !found
            .iter()
            .any(|l| l.port <= 1024 && !matches!(l.port, 80 | 443)),
        "unexpected privileged non-app port in discover: {found:?}"
    );
    session.close();
}

#[test]
fn live_manual_off_plan_matches_restart_semantics() {
    if !enabled() {
        eprintln!("skip: set AUTOTUN_SSH_TEST=1 to run");
        return;
    }
    let _guard = live_guard();
    let dest = destination();
    let (port, marker) = start_remote_listener(&dest);

    // Simulate: discovered + user manual-off, then service restart.
    let mut tunnel = Tunnel::local(port);
    tunnel.manual_off = true;
    tunnel.enabled = false;
    let mut tunnels = vec![tunnel];

    let socket = unique_socket();
    let mut session = SshSession::connect(dest.clone(), socket, vec![]).expect("ssh");
    let found = session.discover_ports(true).expect("discover");
    assert!(found.iter().any(|l| l.port == port));

    apply_scan_in_memory(&mut tunnels, &found, true);
    assert!(
        !tunnels[0].enabled,
        "manual_off tunnel must stay off while service is up"
    );

    stop_remote_listener(&dest, &marker);
    // Two empty scans after stop.
    apply_scan_in_memory(&mut tunnels, &[], true);
    apply_scan_in_memory(&mut tunnels, &[], true);
    assert!(!tunnels[0].present);

    // Restart service.
    let (port2, marker2) = start_remote_listener(&dest);
    // If the OS reused a different free port, re-bind the scenario to the same
    // logical test by checking manual_off still blocks auto enable for `port`
    // and that a *new* port would be discovered separately.
    let found2 = session.discover_ports(true).expect("discover after restart");
    apply_scan_in_memory(&mut tunnels, &found2, true);

    if port2 == port {
        assert!(
            !tunnels[0].enabled && tunnels[0].manual_off,
            "same port restart must not auto-enable manual_off tunnel"
        );
    } else {
        // Original stays manual_off / absent or present if something else took the port.
        assert!(tunnels[0].manual_off);
        assert!(
            tunnels.iter().any(|t| t.source_port == port2 && t.enabled),
            "new port should be auto-discovered and enabled"
        );
    }

    stop_remote_listener(&dest, &marker2);
    session.close();
    let _ = port2;
}

#[test]
fn live_parse_remote_ss_matches_discover() {
    if !enabled() {
        eprintln!("skip: set AUTOTUN_SSH_TEST=1 to run");
        return;
    }
    let _guard = live_guard();
    let dest = destination();
    // Same remote command the session uses (pipeline must be one argv so the
    // remote shell keeps the `||` fallback).
    let raw = Command::new("ssh")
        .args([
            dest.as_str(),
            "sh -lc 'ss -H -lntp 2>/dev/null || ss -H -lnt'",
        ])
        .output()
        .expect("ssh ss");
    assert!(
        raw.status.success(),
        "remote ss failed: {}",
        String::from_utf8_lossy(&raw.stderr)
    );
    let text = String::from_utf8_lossy(&raw.stdout);
    let parsed = parse_ss_listeners(&text, true);

    let socket = unique_socket();
    let mut session = SshSession::connect(dest, socket, vec![]).expect("ssh");
    let found = session.discover_ports(true).expect("discover");
    session.close();

    let mut a: Vec<u16> = parsed.iter().map(|l| l.port).collect();
    let mut b: Vec<u16> = found.iter().map(|l| l.port).collect();
    a.sort_unstable();
    b.sort_unstable();
    assert_eq!(a, b, "discover_ports must match local parse of remote ss\nraw:\n{text}");
}
