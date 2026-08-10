# autotun

[![CI](https://github.com/thuanlm215/autotun/actions/workflows/ci.yml/badge.svg)](https://github.com/thuanlm215/autotun/actions/workflows/ci.yml)
[![Release](https://img.shields.io/github/v/release/thuanlm215/autotun)](https://github.com/thuanlm215/autotun/releases/latest)
[![License](https://img.shields.io/github/license/thuanlm215/autotun)](LICENSE)

`autotun` is a terminal interface for discovering and managing SSH port
forwarding. It watches TCP services on a remote host, forwards them to your
machine automatically, and lets you manage local and reverse tunnels without
reconnecting.

All operations share one OpenSSH ControlMaster connection. Existing SSH
configuration—including aliases, keys, agents, `ProxyJump`, and custom
options—continues to work.

## Features

- Automatically discovers remote TCP listeners above port 1024.
- Forwards discovered services to local loopback by default.
- Supports local (`-L`) and explicitly configured reverse (`-R`) tunnels.
- Adds and removes forwards without opening another SSH transport.
- Rescans remote listeners in the background and tracks service lifecycle.
- Restores active tunnels after an SSH reconnect.
- Resolves port conflicts predictably by trying the next five ports.
- Provides labels, manual port editing, per-tunnel controls, and address copy.
- Ships as static Linux binaries for x86-64 and ARM64.

## Installation

### Install script

The installer detects the host architecture, downloads the latest release,
verifies its SHA-256 checksum, and installs `autotun` to
`$HOME/.local/bin/autotun`:

```sh
curl -fsSL https://raw.githubusercontent.com/thuanlm215/autotun/main/install.sh | sh
```

Many Linux environments include `$HOME/.local/bin` in `PATH`. If yours does
not, add that directory using your shell's normal `PATH` configuration or run
the binary as `$HOME/.local/bin/autotun`.

Install a specific version or select another destination:

```sh
curl -fsSL https://raw.githubusercontent.com/thuanlm215/autotun/main/install.sh \
  | AUTOTUN_VERSION=1.0.0 AUTOTUN_INSTALL_DIR="$HOME/bin" sh
```

For environments where piping a remote script is undesirable, download and
inspect it first:

```sh
curl -fsSLO https://raw.githubusercontent.com/thuanlm215/autotun/main/install.sh
less install.sh
sh install.sh
```

Release archives and checksums are also available from the
[GitHub Releases page](https://github.com/thuanlm215/autotun/releases/latest).

### Build from source

Rust and OpenSSH are required:

```sh
cargo install --git https://github.com/thuanlm215/autotun --locked
```

Or build a checkout directly:

```sh
cargo build --release --locked
install -Dm755 target/release/autotun "$HOME/.local/bin/autotun"
```

## Usage

Connect using an SSH destination or an alias from `~/.ssh/config`:

```sh
autotun user@example.com
autotun development-server
```

Remote services are discovered and forwarded automatically. To start in
manual mode instead:

```sh
autotun development-server --no-auto-forward
```

Change the default three-second scan interval:

```sh
autotun development-server --interval 5
```

Add reverse forwards at startup or pass additional OpenSSH options:

```sh
autotun development-server -R 3000 -R 8080
autotun development-server --ssh-arg=-J --ssh-arg=bastion.example.com
```

Run `autotun --help` for the complete command-line reference.

## Controls

| Key | Action |
| --- | --- |
| `↑` / `↓`, `j` / `k` | Select a tunnel |
| `Space`, `Enter` | Enable or disable the selected tunnel |
| `a` | Add a remote service to local loopback (`-L`) |
| `v` | Expose a local service on remote loopback (`-R`) |
| `e` | Edit the selected tunnel's ports or label |
| `d` | Delete a manual tunnel or ignore a discovered tunnel |
| `r` | Scan remote listeners immediately |
| `q`, `Esc` | Close all tunnels and exit |

The add form appears directly below the tunnel table. Use `Tab` or
`Shift+Tab` to change fields, `Enter` to advance or save, and `Esc` to cancel.

The `URL` column is populated only for services that respond as HTTP or TLS.
It is plain `http://` or `https://` text so terminals with URL detection can
open it with their usual Ctrl/Cmd-click gesture.

## Forwarding behavior

For a remote service listening on port `3000`, autotun first requests this
mapping:

```text
local 127.0.0.1:3000  →  remote 127.0.0.1:3000
```

If local port `3000` is unavailable, it tries `3001` through `3005` in order.
If none can be bound, the tunnel remains visible with a conflict status.

Reverse tunnels are never discovered or created implicitly. They must be added
with `v` in the TUI or explicitly with `--reverse` / `-R`. A reverse tunnel is
bound to remote loopback and targets the requested local service:

```text
remote 127.0.0.1:8080  →  local 127.0.0.1:8080
```

Autotun scans every three seconds by default. A remote service must be absent
from two consecutive successful scans before its local tunnel is removed.
Failed scans do not remove tunnels. If the service returns, its tunnel is
restored unless it was disabled manually.

Configuration and manual overrides are intentionally scoped to the current
session; autotun does not persist settings per host.

## How it works

Autotun starts one background OpenSSH ControlMaster connection. Listener
discovery and tunnel changes run as multiplexed channels through its control
socket:

```text
autotun
   └── one SSH transport
       ├── remote listener scans
       ├── local forwards (-L)
       └── reverse forwards (-R)
```

The remote scan uses `ss`, with `netstat` as a fallback. No remote agent,
daemon, elevated privileges, or configuration file is installed.

## Requirements and compatibility

- Linux on x86-64 or ARM64 for the prebuilt binaries.
- OpenSSH client available as `ssh` on the local machine.
- `ss` or `netstat` available on the remote host.
- TCP forwarding permitted by the remote SSH server.
- A terminal with OSC 52 support is recommended for clipboard integration.

Other architectures can build autotun from source when supported by Rust.

## Security

- Generated forwards bind to `127.0.0.1`; services are not exposed publicly by
  default.
- Reverse forwarding is always an explicit user action.
- Release installers verify published SHA-256 checksums before installation.
- SSH authentication and host verification are delegated to OpenSSH.

Review the installer and release checksums before use if required by your
environment's security policy.

## Contributing

Issues and pull requests are welcome. Before submitting a change, run:

```sh
cargo fmt --check
cargo test --locked
cargo clippy --locked --all-targets -- -D warnings
tests/install.sh
```

## License

Licensed under the [MIT License](LICENSE).
