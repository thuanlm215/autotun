# autotun

`autotun` is a Linux TUI that discovers TCP listeners on an SSH host and lets
you toggle forwards without reconnecting. It uses an OpenSSH ControlMaster, so
port discovery, `-L` forwards, and `-R` forwards share one SSH transport.

![status](https://img.shields.io/badge/status-production--ready-brightgreen)

## Features

- Detect remote listening TCP ports above 1024 with `ss` (or `netstat` fallback).
- Auto-forward discovered services by default; opt out with `--no-auto-forward`.
- Rescan in the background every 3 seconds (configurable with `--interval`).
- Require two valid missing scans before removing a stopped service's tunnel.
- Toggle, label, edit, delete, and copy addresses directly from the TUI.
- Prefer the requested bind port, then try the next five sequential ports.
- Add reverse forwards explicitly from the TUI or with `--reverse` / `-R`.
- One SSH connection; dynamic forwards use OpenSSH multiplex control commands.
- Reconnect after network loss and restore tunnels that were active.
- Works with SSH config aliases, ProxyJump, keys, and agents.

## Install

Prerequisites: Rust and OpenSSH. On Arch Linux:

```bash
sudo pacman -S --needed rust openssh
cargo install --git https://github.com/thuanlm215/autotun
```

Or build from a checkout:

```bash
cargo build --release
install -Dm755 target/release/autotun ~/.local/bin/autotun
```

## Usage

```bash
autotun user@example.com
autotun my-ssh-config-alias
autotun server -R 3000 -R 8080
autotun server --no-auto-forward --interval 5
autotun server --ssh-arg=-J --ssh-arg=bastion
```

TUI keys:

| Key | Action |
| --- | --- |
| `↑`/`↓`, `j`/`k` | Select a tunnel |
| `Space`, `Enter` | Toggle the selected tunnel |
| `a` | Add a local forward (`-L`) |
| `v` | Add a reverse forward (`-R`) |
| `e` | Edit ports or label |
| `d` | Delete a manual tunnel or ignore a discovered tunnel |
| `c` | Copy the active bind address using OSC 52 |
| `r` | Scan remote listeners immediately |
| `q`, `Esc` | Close every tunnel and the SSH connection |

For local forwards, a row `Service port 3000 / Bind port 3001` means remote
`127.0.0.1:3000` is reachable at local `127.0.0.1:3001`. If ports 3000 through
3005 are all busy, the row reports a conflict instead of choosing a surprising
random port. Reverse forwards are never auto-discovered: adding one is an
explicit request to expose a local service to remote loopback.

Disabling an auto-discovered tunnel sets it to `MANUAL OFF`, so background
scans do not turn it back on. A listener that disappears in two consecutive
successful scans becomes `TARGET DOWN`; scan failures never remove tunnels.
When the SSH transport is recreated after a disconnect, autotun restores the
tunnels that were active before the failure. Configuration is intentionally
session-only and is not persisted per host.

## Security and limitations

- All generated listeners bind to `127.0.0.1`; autotun never exposes a service
  publicly by default.
- The remote SSH server must allow TCP forwarding. Reverse forwarding also
  depends on `AllowTcpForwarding`/`DisableForwarding` server policy.
- Reverse ports try the requested remote port and the next five ports.
- Listener discovery shows ports, not owning process names; no remote sudo is
  required.

## License

MIT
