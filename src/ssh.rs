use std::{
    fs,
    path::PathBuf,
    process::{Command, Stdio},
};

use anyhow::{Context, Result, bail};

use crate::ports::{Direction, parse_ss_ports};

pub struct SshSession {
    destination: String,
    socket: PathBuf,
    closed: bool,
}

impl SshSession {
    pub fn connect(destination: String, socket: PathBuf, extra_args: Vec<String>) -> Result<Self> {
        let status = Command::new("ssh")
            .args(["-M", "-S"])
            .arg(&socket)
            .args(["-o", "ControlPersist=yes", "-o", "ExitOnForwardFailure=yes"])
            .args(extra_args)
            .args(["-fnNT", &destination])
            .status()
            .context("failed to start ssh; is OpenSSH installed?")?;
        if !status.success() {
            bail!("SSH master connection failed ({status})");
        }
        Ok(Self {
            destination,
            socket,
            closed: false,
        })
    }

    fn control(&self) -> Command {
        let mut command = Command::new("ssh");
        command.args(["-S"]).arg(&self.socket);
        command
    }

    pub fn discover_ports(&self, include_loopback: bool) -> Result<Vec<u16>> {
        let script = "if command -v ss >/dev/null 2>&1; then ss -H -lnt | awk '{print $4}'; elif command -v netstat >/dev/null 2>&1; then netstat -lnt 2>/dev/null | awk 'NR>2 {print $4}'; else exit 127; fi";
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
        Ok(parse_ss_ports(
            &String::from_utf8_lossy(&output.stdout),
            include_loopback,
        ))
    }

    pub fn forward(&self, direction: Direction, bind_port: u16, source_port: u16) -> Result<()> {
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
