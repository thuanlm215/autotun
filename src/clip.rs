//! Send a local clipboard image to the remote host as a PNG file.
//!
//! Konsole (and most terminals) cannot paste image bytes into an SSH PTY.
//! The useful action is: upload the PNG, put the remote path on the local
//! clipboard, then paste that path into the AI CLI.

use std::{
    env, fs,
    io::{self, Write},
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

const LATEST_NAME: &str = "autotun-clip.png";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LastSession {
    pub destination: String,
    pub socket: PathBuf,
}

pub fn runtime_dir() -> PathBuf {
    env::var_os("XDG_RUNTIME_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(env::temp_dir)
        .join("autotun")
}

pub fn sanitize_destination(destination: &str) -> String {
    let sanitized: String = destination
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '@' | '+') {
                c
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() {
        "session".into()
    } else {
        sanitized
    }
}

pub fn write_last_session(destination: &str, socket: &Path) -> Result<()> {
    let dir = runtime_dir();
    fs::create_dir_all(&dir).context("failed to create autotun runtime dir")?;
    let body = format!("destination={destination}\nsocket={}\n", socket.display());
    fs::write(dir.join("last"), body).context("failed to record last autotun session")
}

pub fn read_last_session() -> Option<LastSession> {
    let text = fs::read_to_string(runtime_dir().join("last")).ok()?;
    let mut destination = None;
    let mut socket = None;
    for line in text.lines() {
        if let Some(value) = line.strip_prefix("destination=") {
            destination = Some(value.to_owned());
        } else if let Some(value) = line.strip_prefix("socket=") {
            socket = Some(PathBuf::from(value));
        }
    }
    Some(LastSession {
        destination: destination?,
        socket: socket?,
    })
}

pub fn remote_unique_path() -> String {
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    format!("/tmp/autotun-clip-{stamp}.png")
}

pub fn remote_latest_path() -> &'static str {
    "/tmp/autotun-clip.png"
}

/// Upload the local clipboard PNG and copy the remote path to the clipboard.
///
/// Uses the live ControlMaster when `autotun` is already connected to the
/// same host; otherwise opens a one-shot `ssh` (same idea as clipssh).
pub fn send_clipboard_image(destination: Option<String>) -> Result<String> {
    let last = read_last_session();
    let destination = destination
        .or_else(|| last.as_ref().map(|session| session.destination.clone()))
        .context("no host: pass one (`autotun clip user@host`) or start autotun first")?;
    let png = read_clipboard_png()?;
    let remote_path = remote_unique_path();
    let master = last.filter(|session| {
        session.destination == destination && control_is_alive(&session.socket, &destination)
    });
    upload_via_ssh(
        master.as_ref().map(|session| session.socket.as_path()),
        &destination,
        &png,
        &remote_path,
    )?;
    copy_text_to_clipboard(&remote_path);
    Ok(remote_path)
}

pub fn upload_png_on_session(destination: &str, socket: &Path, png: &[u8]) -> Result<String> {
    let remote_path = remote_unique_path();
    upload_via_ssh(Some(socket), destination, png, &remote_path)?;
    copy_text_to_clipboard(&remote_path);
    Ok(remote_path)
}

pub fn read_clipboard_png() -> Result<Vec<u8>> {
    let data = if env::var_os("WAYLAND_DISPLAY").is_some() && command_exists("wl-paste") {
        run_clipboard_read(&["wl-paste", "--type", "image/png"])
            .context("wl-paste could not read an image from the clipboard")?
    } else if command_exists("xclip") {
        run_clipboard_read(&[
            "xclip",
            "-selection",
            "clipboard",
            "-target",
            "image/png",
            "-o",
        ])
        .context("xclip could not read an image from the clipboard")?
    } else if command_exists("wl-paste") {
        run_clipboard_read(&["wl-paste", "--type", "image/png"])
            .context("wl-paste could not read an image from the clipboard")?
    } else {
        bail!("no clipboard tool found; install wl-clipboard (Wayland) or xclip (X11)");
    };
    if data.len() < 8 || &data[..8] != b"\x89PNG\r\n\x1a\n" {
        bail!("clipboard does not contain a PNG image (take a screenshot first)");
    }
    Ok(data)
}

/// Copy text to the system clipboard (wl-copy / xclip) and via OSC 52.
pub fn copy_text_to_clipboard(text: &str) {
    if env::var_os("WAYLAND_DISPLAY").is_some() && command_exists("wl-copy") {
        let _ = write_to_command(&["wl-copy"], text.as_bytes());
    } else if command_exists("xclip") {
        let _ = write_to_command(&["xclip", "-selection", "clipboard"], text.as_bytes());
    } else if command_exists("wl-copy") {
        let _ = write_to_command(&["wl-copy"], text.as_bytes());
    }
    let encoded = base64_encode(text.as_bytes());
    let _ = write!(io::stdout(), "\x1b]52;c;{encoded}\x07");
    let _ = io::stdout().flush();
}

fn control_is_alive(socket: &Path, destination: &str) -> bool {
    Command::new("ssh")
        .args(["-S"])
        .arg(socket)
        .args(["-O", "check", destination])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .is_ok_and(|status| status.success())
}

fn upload_via_ssh(
    socket: Option<&Path>,
    destination: &str,
    png: &[u8],
    remote_path: &str,
) -> Result<()> {
    let latest = format!("/tmp/{LATEST_NAME}");
    let script = format!(
        "umask 077 && cat > {remote} && ln -sfn {remote} {latest}",
        remote = shell_words::quote(remote_path),
        latest = shell_words::quote(&latest),
    );
    let remote_command = format!("sh -lc {}", shell_words::quote(&script));
    let mut command = Command::new("ssh");
    if let Some(socket) = socket {
        command.args(["-S"]).arg(socket);
    }
    command.arg(destination).arg(&remote_command);
    command.stdin(Stdio::piped());
    command.stdout(Stdio::null());
    command.stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .context("failed to start ssh; is OpenSSH installed?")?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .context("failed to open ssh stdin for clipboard upload")?;
        stdin
            .write_all(png)
            .context("failed to write image to ssh")?;
    }
    let output = child
        .wait_with_output()
        .context("clipboard upload ssh failed")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "failed to write {remote_path} on {destination}: {stderr}",
            stderr = stderr.trim()
        );
    }
    Ok(())
}

fn run_clipboard_read(argv: &[&str]) -> Result<Vec<u8>> {
    let output = Command::new(argv[0])
        .args(&argv[1..])
        .output()
        .with_context(|| format!("failed to run {}", argv[0]))?;
    if !output.status.success() {
        bail!("{} exited with {}", argv[0], output.status);
    }
    Ok(output.stdout)
}

fn write_to_command(argv: &[&str], data: &[u8]) -> Result<()> {
    let mut child = Command::new(argv[0])
        .args(&argv[1..])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .with_context(|| format!("failed to run {}", argv[0]))?;
    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(data)?;
    }
    let _ = child.wait();
    Ok(())
}

fn command_exists(name: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!(
            "command -v {} >/dev/null",
            shell_words::quote(name)
        ))
        .status()
        .is_ok_and(|status| status.success())
}

fn base64_encode(data: &[u8]) -> String {
    const CHARS: &[u8] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut result = String::with_capacity(data.len().div_ceil(3) * 4);
    for chunk in data.chunks(3) {
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let triple = (b0 << 16) | (b1 << 8) | b2;
        result.push(CHARS[((triple >> 18) & 0x3F) as usize] as char);
        result.push(CHARS[((triple >> 12) & 0x3F) as usize] as char);
        if chunk.len() > 1 {
            result.push(CHARS[((triple >> 6) & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
        if chunk.len() > 2 {
            result.push(CHARS[(triple & 0x3F) as usize] as char);
        } else {
            result.push('=');
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sanitizes_destination_for_filenames() {
        assert_eq!(sanitize_destination("user@host"), "user@host");
        assert_eq!(sanitize_destination("my/server"), "my_server");
        assert_eq!(sanitize_destination(""), "session");
    }

    #[test]
    fn last_session_roundtrip() {
        static ENV_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());
        let _guard = ENV_LOCK.lock().unwrap();
        let dir = tempfile::tempdir().unwrap();
        unsafe {
            env::set_var("XDG_RUNTIME_DIR", dir.path());
        }
        let socket = dir.path().join("autotun").join("dev.sock");
        write_last_session("dev", &socket).unwrap();
        let got = read_last_session().unwrap();
        assert_eq!(got.destination, "dev");
        assert_eq!(got.socket, socket);
        unsafe {
            env::remove_var("XDG_RUNTIME_DIR");
        }
    }

    #[test]
    fn unique_path_is_tmp_png() {
        let path = remote_unique_path();
        assert!(path.starts_with("/tmp/autotun-clip-"));
        assert!(path.ends_with(".png"));
        assert_eq!(remote_latest_path(), "/tmp/autotun-clip.png");
    }
}
