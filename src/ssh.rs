use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
    sync::Mutex,
};

use anyhow::{Context, Result, bail};

use crate::ports::{Direction, RemoteListener, parse_ss_listeners};

pub struct SshSession {
    destination: String,
    socket: PathBuf,
    closed: bool,
    extra_args: Vec<String>,
    command_lock: Mutex<()>,
}

impl SshSession {
    pub fn destination(&self) -> &str {
        &self.destination
    }

    pub fn socket(&self) -> &PathBuf {
        &self.socket
    }

    pub fn connect(destination: String, socket: PathBuf, extra_args: Vec<String>) -> Result<Self> {
        Self::start_master(&destination, &socket, &extra_args, false)?;
        Ok(Self {
            destination,
            socket,
            closed: false,
            extra_args,
            command_lock: Mutex::new(()),
        })
    }

    fn start_master(
        destination: &str,
        socket: &PathBuf,
        extra_args: &[String],
        batch_mode: bool,
    ) -> Result<()> {
        let mut command = Command::new("ssh");
        command.args(["-M", "-S"]).arg(socket).args([
            "-o",
            "ControlPersist=yes",
            "-o",
            "ExitOnForwardFailure=yes",
            "-o",
            "ConnectTimeout=10",
            "-o",
            "ServerAliveInterval=10",
            "-o",
            "ServerAliveCountMax=3",
        ]);
        if batch_mode {
            command.args(["-o", "BatchMode=yes"]);
        }
        let status = command
            .args(extra_args)
            .args(["-fnNT", destination])
            .status()
            .context("failed to start ssh; is OpenSSH installed?")?;
        if !status.success() {
            bail!("SSH master connection failed ({status})");
        }
        Ok(())
    }

    fn control(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args(["-S"]).arg(&self.socket);
        command
    }

    pub fn discover_ports(&self, include_loopback: bool) -> Result<Vec<RemoteListener>> {
        let _guard = self.command_lock.lock().expect("SSH command lock poisoned");
        self.discover_ports_unlocked(include_loopback)
    }

    fn discover_ports_unlocked(&self, include_loopback: bool) -> Result<Vec<RemoteListener>> {
        // Prefer ss -p so process names can label tunnels. Fall back without -p
        // when the option is unavailable; process names are best-effort.
        let script = "if command -v ss >/dev/null 2>&1; then ss -H -lntp 2>/dev/null || ss -H -lnt; elif command -v netstat >/dev/null 2>&1; then netstat -lntp 2>/dev/null || netstat -lnt 2>/dev/null; else exit 127; fi";
        let remote_command = format!("sh -lc {}", shell_words::quote(script));
        let output = self
            .control()
            .arg(&self.destination)
            .arg(&remote_command)
            .output()
            .context("failed to query remote listeners")?;
        if !output.status.success() {
            bail!("remote host needs ss (iproute2) or netstat");
        }
        Ok(parse_ss_listeners(
            &String::from_utf8_lossy(&output.stdout),
            include_loopback,
        ))
    }

    pub fn forward(&self, direction: Direction, bind_port: u16, source_port: u16) -> Result<()> {
        let _guard = self.command_lock.lock().expect("SSH command lock poisoned");
        let (flag, spec) = match direction {
            Direction::Local => (
                "-L",
                format!("127.0.0.1:{bind_port}:127.0.0.1:{source_port}"),
            ),
            Direction::Reverse => (
                "-R",
                format!("127.0.0.1:{bind_port}:127.0.0.1:{source_port}"),
            ),
        };
        let status = self
            .control()
            .args(["-O", "forward", flag, &spec, &self.destination])
            .stdin(Stdio::null())
            .status()
            .context("failed to add SSH forward")?;
        if !status.success() {
            bail!("ssh rejected {flag} {spec}");
        }
        Ok(())
    }

    pub fn cancel(&self, direction: Direction, bind_port: u16, source_port: u16) -> Result<()> {
        let _guard = self.command_lock.lock().expect("SSH command lock poisoned");
        let (flag, spec) = match direction {
            Direction::Local => (
                "-L",
                format!("127.0.0.1:{bind_port}:127.0.0.1:{source_port}"),
            ),
            Direction::Reverse => (
                "-R",
                format!("127.0.0.1:{bind_port}:127.0.0.1:{source_port}"),
            ),
        };
        let status = self
            .control()
            .args(["-O", "cancel", flag, &spec, &self.destination])
            .status()?;
        if !status.success() {
            bail!("ssh could not cancel {flag} {spec}");
        }
        Ok(())
    }

    /// Recreate a dead master connection without prompting in the background.
    /// Returns true when a new connection was created.
    pub fn reconnect_if_needed(&self) -> Result<bool> {
        let _guard = self.command_lock.lock().expect("SSH command lock poisoned");
        let alive = self
            .control()
            .args(["-O", "check", &self.destination])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .is_ok_and(|status| status.success());
        if alive {
            return Ok(false);
        }
        let _ = fs::remove_file(&self.socket);
        Self::start_master(&self.destination, &self.socket, &self.extra_args, true)?;
        Ok(true)
    }

    pub fn close(&mut self) {
        if self.closed {
            return;
        }
        let _ = self
            .control()
            .args(["-O", "exit", &self.destination])
            .status();
        let _ = fs::remove_file(&self.socket);
        self.closed = true;
    }
}

impl Drop for SshSession {
    fn drop(&mut self) {
        self.close();
    }
}
