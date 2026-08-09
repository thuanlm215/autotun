# autotun

`autotun` is a Linux TUI that discovers TCP listeners on an SSH host and lets
you toggle forwards without reconnecting. It uses an OpenSSH ControlMaster, so
port discovery, `-L` forwards, and `-R` forwards share one SSH transport.

![status](https://img.shields.io/badge/status-MVP-orange)

## Features

- Detect remote listening TCP ports with `ss` (or `netstat` fallback).
- Toggle each local forward independently from the TUI.
- Prefer the same local port; automatically allocate another port when busy.
- Add reverse forwards for local services with `--reverse` / `-R`.
- One SSH connection; dynamic forwards use OpenSSH multiplex control commands.
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
autotun server --ssh-arg=-J --ssh-arg=bastion
```

In the TUI, use `↑`/`↓` (or `j`/`k`) to select, `Space`/`Enter` to toggle,
`r` to refresh discovered listeners, and `q` to quit. Quitting closes all
tunnels and the master connection.

For local forwards, a row `Service port 3000 / Bind port 49152` means remote
`127.0.0.1:3000` is reachable at local `127.0.0.1:49152`. Reverse forwards bind
only remote loopback by default and expose the requested local service on the
same numbered remote port.

## Security and limitations

- All generated listeners bind to `127.0.0.1`; autotun never exposes a service
  publicly by default.
- The remote SSH server must allow TCP forwarding. Reverse forwarding also
  depends on `AllowTcpForwarding`/`DisableForwarding` server policy.
- Reverse ports currently use the same port number on the remote host. If that
  port is occupied, SSH reports the failure in the TUI.
- Listener discovery shows ports, not owning process names; no remote sudo is
  required.

## License

MIT
