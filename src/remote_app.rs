//! Waypipe-backed remote Wayland application lifecycle management.
//!
//! This module intentionally has no GUI dependency. Launch checks run on a
//! worker thread, while the frontend periodically calls [`RemoteAppManager::poll`].

use std::{
    collections::HashSet,
    env,
    ffi::OsStr,
    io::Read,
    path::{Path, PathBuf},
    process::{Child, ChildStderr, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc::{self, Receiver, Sender},
    },
    thread,
};

use anyhow::{Context, Result, bail};

use crate::ssh::SshControl;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RemoteAppStatus {
    Starting,
    Running,
    Exited(String),
    Failed(String),
}

impl RemoteAppStatus {
    pub fn label(&self) -> &str {
        match self {
            Self::Starting => "starting",
            Self::Running => "running",
            Self::Exited(_) => "exited",
            Self::Failed(_) => "failed",
        }
    }

    pub fn details(&self) -> Option<&str> {
        match self {
            Self::Exited(details) | Self::Failed(details) if !details.is_empty() => Some(details),
            _ => None,
        }
    }

    pub fn can_stop(&self) -> bool {
        matches!(self, Self::Starting | Self::Running)
    }

    pub fn is_finished(&self) -> bool {
        matches!(self, Self::Exited(_) | Self::Failed(_))
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RemoteAppInfo {
    pub id: u64,
    pub command: String,
    pub status: RemoteAppStatus,
}

struct RemoteApp {
    info: RemoteAppInfo,
    child: Option<Child>,
    stderr: Option<StderrCapture>,
    pending_exit: Option<ExitStatus>,
}

enum LaunchEvent {
    Ready {
        id: u64,
        child: Child,
        stderr: StderrCapture,
    },
    Failed {
        id: u64,
        error: String,
    },
}

/// Drains a child stderr pipe continuously while retaining only the tail used
/// for final diagnostics. The reader owns the pipe, so it terminates at EOF
/// when the child exits or is stopped. UI polling only joins a reader after it
/// has already finished, so a chatty child can never block the event loop.
struct StderrCapture {
    tail: Arc<std::sync::Mutex<Vec<u8>>>,
    reader: Option<thread::JoinHandle<()>>,
}

impl StderrCapture {
    const TAIL_LIMIT: usize = 4_096;

    fn start(stderr: ChildStderr) -> Self {
        let tail = Arc::new(std::sync::Mutex::new(Vec::new()));
        let reader_tail = Arc::clone(&tail);
        let reader = thread::spawn(move || drain_stderr(stderr, reader_tail));
        Self {
            tail,
            reader: Some(reader),
        }
    }

    fn finish(&mut self) -> String {
        if let Some(reader) = self.reader.take() {
            // The child was reaped before this method is called, so its stderr
            // pipe is at EOF and the reader has a bounded amount of work left.
            let _ = reader.join();
        }
        stderr_text(&self.tail.lock().unwrap_or_else(|error| error.into_inner()))
    }

    fn finish_if_ready(&mut self) -> Option<String> {
        self.reader
            .as_ref()
            .is_none_or(thread::JoinHandle::is_finished)
            .then(|| self.finish())
    }
}

/// Owns all Waypipe children created for one GUI SSH session.
pub struct RemoteAppManager {
    apps: Vec<RemoteApp>,
    launch_tx: Sender<LaunchEvent>,
    launch_rx: Receiver<LaunchEvent>,
    cancelled: HashSet<u64>,
    next_id: u64,
    closed: Arc<AtomicBool>,
}

impl Default for RemoteAppManager {
    fn default() -> Self {
        let (launch_tx, launch_rx) = mpsc::channel();
        Self {
            apps: Vec::new(),
            launch_tx,
            launch_rx,
            cancelled: HashSet::new(),
            next_id: 1,
            closed: Arc::new(AtomicBool::new(false)),
        }
    }
}

impl RemoteAppManager {
    pub fn launch(&mut self, command: &str, control: SshControl) -> Result<()> {
        let argv = parse_remote_command(command)?;
        // Keep active apps, but do not let diagnostics from previous attempts
        // accumulate forever in the compact GUI panel.
        self.clear_finished();
        let id = self.next_id;
        self.next_id += 1;
        self.apps.push(RemoteApp {
            info: RemoteAppInfo {
                id,
                command: command.trim().to_owned(),
                status: RemoteAppStatus::Starting,
            },
            child: None,
            stderr: None,
            pending_exit: None,
        });

        let tx = self.launch_tx.clone();
        let closed = Arc::clone(&self.closed);
        thread::spawn(move || {
            let result = preflight(&control).and_then(|waypipe| {
                if closed.load(Ordering::Relaxed) {
                    bail!("remote app launch was cancelled")
                }
                let mut process = waypipe_command(&waypipe, &control, &argv);
                process
                    .stdin(Stdio::null())
                    .stdout(Stdio::null())
                    .stderr(Stdio::piped());
                let mut child = process.spawn().context("failed to start local waypipe")?;
                let stderr = child
                    .stderr
                    .take()
                    .map(StderrCapture::start)
                    .expect("waypipe stderr was configured as piped");
                Ok((child, stderr))
            });
            match result {
                Ok((mut child, mut stderr)) => {
                    if closed.load(Ordering::Relaxed) {
                        stop_child(&mut child);
                        let _ = stderr.finish();
                    } else if let Err(error) = tx.send(LaunchEvent::Ready { id, child, stderr })
                        && let LaunchEvent::Ready {
                            mut child,
                            mut stderr,
                            ..
                        } = error.0
                    {
                        stop_child(&mut child);
                        let _ = stderr.finish();
                    }
                }
                Err(error) => {
                    let _ = tx.send(LaunchEvent::Failed {
                        id,
                        error: format!("{error:#}"),
                    });
                }
            }
        });
        Ok(())
    }

    /// Apply completed preflight checks and observe children without blocking.
    pub fn poll(&mut self) {
        while let Ok(event) = self.launch_rx.try_recv() {
            match event {
                LaunchEvent::Ready {
                    id,
                    mut child,
                    mut stderr,
                } => {
                    let Some(app) = self.apps.iter_mut().find(|app| app.info.id == id) else {
                        stop_child(&mut child);
                        let _ = stderr.finish();
                        continue;
                    };
                    if self.cancelled.remove(&id) {
                        stop_child(&mut child);
                        let _ = stderr.finish();
                        app.info.status = RemoteAppStatus::Exited("stopped".into());
                    } else {
                        app.info.status = RemoteAppStatus::Running;
                        app.child = Some(child);
                        app.stderr = Some(stderr);
                        app.pending_exit = None;
                    }
                }
                LaunchEvent::Failed { id, error } => {
                    if let Some(app) = self.apps.iter_mut().find(|app| app.info.id == id)
                        && !self.cancelled.remove(&id)
                    {
                        app.info.status = RemoteAppStatus::Failed(error);
                    }
                }
            }
        }

        for app in &mut self.apps {
            let Some(child) = app.child.as_mut() else {
                continue;
            };
            let status = match child.try_wait() {
                Ok(Some(status)) => status,
                Ok(None) => continue,
                Err(error) => {
                    stop_child(child);
                    let stderr = app
                        .stderr
                        .as_mut()
                        .and_then(StderrCapture::finish_if_ready)
                        .unwrap_or_default();
                    app.child = None;
                    app.stderr = None;
                    app.info.status = RemoteAppStatus::Failed(exit_details(
                        None,
                        &format!("could not observe waypipe process: {error}; {stderr}"),
                    ));
                    continue;
                }
            };
            app.child = None;
            let success = status.success();
            app.pending_exit = Some(status);
            app.info.status = if success {
                RemoteAppStatus::Exited("collecting diagnostics…".into())
            } else {
                RemoteAppStatus::Failed("collecting diagnostics…".into())
            };
        }

        for app in &mut self.apps {
            let Some(status) = app.pending_exit else {
                continue;
            };
            let Some(stderr) = app.stderr.as_mut().and_then(StderrCapture::finish_if_ready) else {
                continue;
            };
            app.stderr = None;
            app.pending_exit = None;
            app.info.status = completed_status(status, &stderr);
        }
    }

    pub fn apps(&self) -> Vec<RemoteAppInfo> {
        self.apps.iter().map(|app| app.info.clone()).collect()
    }

    pub fn has_finished(&self) -> bool {
        self.apps.iter().any(|app| app.info.status.is_finished())
    }

    pub fn clear_finished(&mut self) {
        self.apps.retain(|app| !app.info.status.is_finished());
    }

    pub fn stop(&mut self, id: u64) {
        let Some(app) = self.apps.iter_mut().find(|app| app.info.id == id) else {
            return;
        };
        if let Some(child) = app.child.as_mut() {
            stop_child(child);
            // The child has been reaped, so an unfinished reader has an EOF
            // pending and will self-terminate after this non-blocking drop.
            let _ = app.stderr.as_mut().and_then(StderrCapture::finish_if_ready);
            app.child = None;
            app.stderr = None;
            app.pending_exit = None;
            app.info.status = RemoteAppStatus::Exited("stopped".into());
        } else if matches!(app.info.status, RemoteAppStatus::Starting) {
            self.cancelled.insert(id);
            app.info.status = RemoteAppStatus::Exited("stopped".into());
        }
    }

    pub fn stop_all(&mut self) {
        let ids = self.apps.iter().map(|app| app.info.id).collect::<Vec<_>>();
        for id in ids {
            self.stop(id);
        }
    }
}

impl Drop for RemoteAppManager {
    fn drop(&mut self) {
        self.closed.store(true, Ordering::Relaxed);
        self.stop_all();
    }
}

/// Split a user-supplied command into an argv without ever invoking a shell.
pub fn parse_remote_command(command: &str) -> Result<Vec<String>> {
    let argv = shell_words::split(command).context("invalid remote command quoting")?;
    let Some(program) = argv.first() else {
        bail!("enter a remote command, for example: firefox --new-instance")
    };
    if program.starts_with('-') {
        bail!("the remote command must start with a program name, not an option")
    }
    Ok(argv)
}

fn preflight(control: &SshControl) -> Result<PathBuf> {
    if env::var_os("WAYLAND_DISPLAY").is_none_or(|display| display.is_empty()) {
        bail!("a local Wayland session is required (WAYLAND_DISPLAY is not set)");
    }
    let waypipe = find_local_executable(OsStr::new("waypipe"))
        .context("waypipe is not installed locally or is not on PATH")?;

    let output = Command::new("ssh")
        .args(["-S"])
        .arg(control.socket())
        .arg(control.destination())
        .arg("command -v waypipe >/dev/null 2>&1")
        .output()
        .context("failed to check for waypipe on the remote host via the existing SSH session")?;
    if !output.status.success() {
        let detail = stderr_text(&output.stderr);
        if detail.is_empty() {
            bail!("waypipe is not installed on the remote host or is not on PATH");
        }
        bail!("remote waypipe check failed: {detail}");
    }
    Ok(waypipe)
}

fn find_local_executable(name: &OsStr) -> Option<PathBuf> {
    let candidate = Path::new(name);
    if candidate.components().count() > 1 {
        return candidate.is_file().then(|| candidate.to_path_buf());
    }
    let path = env::var_os("PATH")?;
    env::split_paths(&path)
        .map(|directory| directory.join(name))
        .find(|candidate| candidate.is_file())
}

fn waypipe_command(waypipe: &Path, control: &SshControl, argv: &[String]) -> Command {
    let mut command = Command::new(waypipe);
    // OpenSSH joins arguments after the destination into a remote shell
    // command. Quote each parsed argv entry first and pass that one string,
    // preserving spaces and making shell metacharacters literal.
    let remote_command = argv
        .iter()
        .map(|arg| shell_words::quote(arg).into_owned())
        .collect::<Vec<_>>()
        .join(" ");
    command
        // Remote VMs commonly have no usable DRM render node. Advertising
        // dmabuf there makes GTK/WebKit probe /dev/dri and fail noisily;
        // shared-memory buffers are slower but reliable over SSH.
        .arg("--no-gpu")
        .arg("ssh")
        .args(["-S"])
        .arg(control.socket())
        .arg(control.destination())
        .arg(remote_command);
    command
}

fn drain_stderr(mut stderr: ChildStderr, tail: Arc<std::sync::Mutex<Vec<u8>>>) {
    let mut buffer = [0_u8; 8_192];
    loop {
        match stderr.read(&mut buffer) {
            Ok(0) => break,
            Ok(size) => append_tail(
                &mut tail.lock().unwrap_or_else(|error| error.into_inner()),
                &buffer[..size],
            ),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(_) => break,
        }
    }
}

fn append_tail(tail: &mut Vec<u8>, bytes: &[u8]) {
    if bytes.len() >= StderrCapture::TAIL_LIMIT {
        tail.clear();
        tail.extend_from_slice(&bytes[bytes.len() - StderrCapture::TAIL_LIMIT..]);
        return;
    }
    let overflow = tail
        .len()
        .saturating_add(bytes.len())
        .saturating_sub(StderrCapture::TAIL_LIMIT);
    if overflow > 0 {
        tail.drain(..overflow);
    }
    tail.extend_from_slice(bytes);
}

fn stderr_text(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let trimmed = text.trim();
    if trimmed.len() <= StderrCapture::TAIL_LIMIT {
        trimmed.to_owned()
    } else {
        let start = trimmed.len() - StderrCapture::TAIL_LIMIT;
        format!("…{}", String::from_utf8_lossy(&trimmed.as_bytes()[start..]))
    }
}

fn exit_details(code: Option<i32>, stderr: &str) -> String {
    let status = match code {
        Some(code) => format!("exit code {code}"),
        None => "terminated by signal".into(),
    };
    if stderr.is_empty() {
        status
    } else {
        format!("{status}: {stderr}")
    }
}

fn completed_status(status: ExitStatus, stderr: &str) -> RemoteAppStatus {
    let details = exit_details(status.code(), stderr);
    // Waypipe 0.8.x can exit successfully even when execvp fails or the child
    // panics. Treat those unambiguous child failures as failures in the UI.
    let child_failed = stderr.contains("Failed to execvp") || stderr.contains("panicked at");
    if status.success() && !child_failed {
        RemoteAppStatus::Exited(details)
    } else {
        RemoteAppStatus::Failed(details)
    }
}

fn stop_child(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[cfg(test)]
mod tests {
    use super::*;

    fn control() -> SshControl {
        SshControl::new("dev-vm".into(), PathBuf::from("/tmp/autotun.sock"))
    }

    #[test]
    fn parses_quoted_command_without_a_shell() {
        assert_eq!(
            parse_remote_command("firefox --profile '/tmp/a profile'").unwrap(),
            ["firefox", "--profile", "/tmp/a profile"]
        );
        assert!(parse_remote_command(" ").is_err());
        assert!(parse_remote_command("--help").is_err());
    }

    #[test]
    fn waypipe_command_reuses_the_control_master_and_quotes_remote_argv() {
        let command = waypipe_command(
            Path::new("/opt/bin/waypipe"),
            &control(),
            &[
                "firefox".into(),
                "--profile".into(),
                "/tmp/a profile".into(),
                "$(touch should-not-run)".into(),
                "semi;colon".into(),
            ],
        );
        let args = command
            .get_args()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect::<Vec<_>>();
        assert_eq!(
            args,
            [
                "--no-gpu",
                "ssh",
                "-S",
                "/tmp/autotun.sock",
                "dev-vm",
                "firefox --profile '/tmp/a profile' '$(touch should-not-run)' 'semi;colon'"
            ]
        );
        assert_eq!(
            shell_words::split(&args[5]).unwrap(),
            [
                "firefox",
                "--profile",
                "/tmp/a profile",
                "$(touch should-not-run)",
                "semi;colon"
            ]
        );
    }

    #[test]
    fn status_details_are_bounded_and_useful() {
        assert_eq!(exit_details(Some(23), "boom"), "exit code 23: boom");
        assert_eq!(RemoteAppStatus::Running.label(), "running");
        assert!(RemoteAppStatus::Starting.can_stop());
        assert!(!RemoteAppStatus::Exited("done".into()).can_stop());
        assert!(RemoteAppStatus::Failed("boom".into()).is_finished());
    }

    #[cfg(unix)]
    #[test]
    fn successful_waypipe_exit_with_child_failure_is_failed() {
        use std::os::unix::process::ExitStatusExt;

        let success = ExitStatus::from_raw(0);
        assert!(matches!(
            completed_status(success, "Failed to execvp 'firefox': No such file"),
            RemoteAppStatus::Failed(_)
        ));
    }

    #[test]
    fn stderr_tail_stays_bounded() {
        let mut tail = b"old".to_vec();
        append_tail(&mut tail, &vec![b'x'; StderrCapture::TAIL_LIMIT + 10]);
        assert_eq!(tail.len(), StderrCapture::TAIL_LIMIT);
        assert!(tail.iter().all(|byte| *byte == b'x'));
    }
}
