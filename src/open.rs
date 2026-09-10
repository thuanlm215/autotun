//! Download a remote file over SSH and open it on the local machine.
//!
//! Inverse of [`crate::clip`]: the AI CLI prints a path (or `file://` URL),
//! the user pastes it, and autotun fetches the bytes over the live
//! ControlMaster then hands them to the local default app (`xdg-open`).

use std::{
    fs::{self, OpenOptions},
    io::{self, Read},
    os::unix::fs::OpenOptionsExt,
    path::{Path, PathBuf},
    process::{Command, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{Context, Result, bail};

use crate::clip;

const MAX_BYTES: u64 = 64 * 1024 * 1024;

/// Fetch a remote path (argument or clipboard) and open it locally.
///
/// Uses the live ControlMaster when `autotun` is already connected to the
/// same host; otherwise opens a one-shot `ssh`.
pub fn open_remote_file(destination: Option<String>, spec: Option<String>) -> Result<PathBuf> {
    let last = clip::read_last_session();
    let destination = destination
        .or_else(|| last.as_ref().map(|session| session.destination.clone()))
        .context("no host: pass --destination or start autotun first")?;
    let master = last.filter(|session| {
        session.destination == destination && clip::control_is_alive(&session.socket, &destination)
    });
    open_via_ssh(
        master.as_ref().map(|session| session.socket.as_path()),
        &destination,
        spec.as_deref().unwrap_or(""),
    )
}

pub fn open_remote_on_session(destination: &str, socket: &Path, spec: &str) -> Result<PathBuf> {
    open_via_ssh(Some(socket), destination, spec)
}

fn open_via_ssh(socket: Option<&Path>, destination: &str, spec: &str) -> Result<PathBuf> {
    let remote_path = spec_to_remote_path(spec)?;
    let bytes = download_via_ssh(socket, destination, &remote_path)?;
    let local_path = write_local(&remote_path, &bytes)?;
    open_local(&local_path)?;
    Ok(local_path)
}

fn spec_to_remote_path(spec: &str) -> Result<String> {
    let raw = if spec.trim().is_empty() {
        clip::read_clipboard_text().context("no path given and clipboard is empty")?
    } else {
        spec.to_string()
    };
    parse_remote_ref(&raw)
}

/// Turn pasted text into a remote filesystem path.
pub fn parse_remote_ref(input: &str) -> Result<String> {
    let trimmed = strip_wrappers(input.trim());
    if trimmed.is_empty() {
        bail!("no file path to open (paste a remote path or file:// URL)");
    }
    if looks_like_http(trimmed) {
        bail!("http(s) URLs open in a browser; autotun open is for remote file paths");
    }
    if let Some(path) = coerce_path(trimmed) {
        return validate_path(path);
    }
    if let Some(path) = find_path_in_text(trimmed) {
        return validate_path(path);
    }
    bail!("not a remote file path or file:// URL")
}

fn strip_wrappers(input: &str) -> &str {
    let stripped = input
        .strip_prefix('<')
        .and_then(|value| value.strip_suffix('>'))
        .unwrap_or(input);
    strip_quotes(stripped.trim())
}

fn strip_quotes(input: &str) -> &str {
    if input.len() >= 2 {
        let bytes = input.as_bytes();
        let first = bytes[0];
        let last = bytes[bytes.len() - 1];
        if (first == last) && (first == b'\'' || first == b'"') {
            return &input[1..input.len() - 1];
        }
    }
    input
}

fn looks_like_http(input: &str) -> bool {
    let head = input.get(..8).unwrap_or(input);
    head.eq_ignore_ascii_case("https://")
        || input
            .get(..7)
            .is_some_and(|h| h.eq_ignore_ascii_case("http://"))
}

fn coerce_path(input: &str) -> Option<String> {
    if let Some(path) = decode_file_url(input) {
        return Some(path);
    }
    if input.starts_with('/') || input == "~" || input.starts_with("~/") {
        return Some(input.to_string());
    }
    None
}

fn decode_file_url(input: &str) -> Option<String> {
    let rest = input.get(7..).filter(|_| {
        input
            .get(..7)
            .is_some_and(|scheme| scheme.eq_ignore_ascii_case("file://"))
    })?;
    let rest = rest.split(['?', '#']).next().unwrap_or("");
    let path = if rest.starts_with('/') {
        rest.to_string()
    } else {
        let slash = rest.find('/')?;
        rest[slash..].to_string()
    };
    let decoded = percent_decode(&path);
    decoded.starts_with('/').then_some(decoded)
}

fn percent_decode(input: &str) -> String {
    let bytes = input.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%'
            && index + 2 < bytes.len()
            && let Ok(value) = u8::from_str_radix(
                std::str::from_utf8(&bytes[index + 1..index + 3]).unwrap_or(""),
                16,
            )
        {
            out.push(value);
            index += 3;
            continue;
        }
        out.push(bytes[index]);
        index += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

fn find_path_in_text(text: &str) -> Option<String> {
    let mut last = None;
    for (offset, _) in text.match_indices("file://") {
        let token = next_token(&text[offset..]);
        if let Some(path) = coerce_path(token) {
            last = Some(path);
        }
    }
    for token in text.split_whitespace() {
        let token = trim_trailing_punct(token);
        if token.starts_with('/')
            && let Some(path) = coerce_path(token)
        {
            last = Some(path);
        }
    }
    last
}

fn next_token(text: &str) -> &str {
    let token = text.split_whitespace().next().unwrap_or(text);
    trim_trailing_punct(token)
}

fn trim_trailing_punct(token: &str) -> &str {
    token.trim_end_matches([',', ';', ')', ']', '>', '"', '\''])
}

fn validate_path(path: String) -> Result<String> {
    if path.is_empty() || path.contains('\0') || path.contains('\n') || path.contains('\r') {
        bail!("invalid remote path");
    }
    if !(path.starts_with('/') || path == "~" || path.starts_with("~/")) {
        bail!("remote path must be absolute (or start with ~/)");
    }
    Ok(path)
}

fn download_via_ssh(
    socket: Option<&Path>,
    destination: &str,
    remote_path: &str,
) -> Result<Vec<u8>> {
    let script = format!(
        "umask 077\npath={path}\ncase \"$path\" in\n~/*) path=\"$HOME/${{path#~/}}\" ;;\n~) path=\"$HOME\" ;;\nesac\nif [ ! -e \"$path\" ]; then printf '%s\\n' \"not found: $path\" >&2; exit 1; fi\nif [ ! -f \"$path\" ]; then printf '%s\\n' \"not a regular file: $path\" >&2; exit 1; fi\nif [ ! -r \"$path\" ]; then printf '%s\\n' \"permission denied: $path\" >&2; exit 1; fi\nexec cat -- \"$path\"",
        path = shell_words::quote(remote_path),
    );
    let remote_command = format!("sh -lc {}", shell_words::quote(&script));
    let mut command = Command::new("ssh");
    if let Some(socket) = socket {
        command.args(["-S"]).arg(socket);
    }
    command
        .arg("-T")
        .arg(destination)
        .arg(&remote_command)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let mut child = command
        .spawn()
        .context("failed to start ssh; is OpenSSH installed?")?;
    let mut stdout = child
        .stdout
        .take()
        .context("failed to open ssh stdout for download")?;
    let mut bytes = Vec::new();
    let mut chunk = [0_u8; 8_192];
    loop {
        let read = stdout
            .read(&mut chunk)
            .context("failed to read remote file")?;
        if read == 0 {
            break;
        }
        let total = bytes.len() as u64 + read as u64;
        if total > MAX_BYTES {
            let _ = child.kill();
            let _ = child.wait();
            bail!("remote file is larger than 64 MiB");
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
    let output = child
        .wait_with_output()
        .context("remote file download ssh failed")?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "failed to read {remote_path} on {destination}: {stderr}",
            stderr = stderr.trim()
        );
    }
    if bytes.is_empty() {
        bail!("{remote_path} is empty");
    }
    Ok(bytes)
}

fn write_local(remote_path: &str, bytes: &[u8]) -> Result<PathBuf> {
    let dir = clip::runtime_dir();
    fs::create_dir_all(&dir).context("failed to create autotun runtime dir")?;
    let stamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_nanos())
        .unwrap_or(0);
    let local_path = dir.join(format!("open-{stamp}-{}", local_basename(remote_path)));
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(&local_path)
        .with_context(|| format!("failed to create {}", local_path.display()))?;
    io::Write::write_all(&mut file, bytes)
        .with_context(|| format!("failed to write {}", local_path.display()))?;
    let latest = dir.join("open-latest");
    let _ = fs::remove_file(&latest);
    let _ = std::os::unix::fs::symlink(&local_path, &latest);
    Ok(local_path)
}

fn local_basename(remote_path: &str) -> String {
    let base = Path::new(remote_path)
        .file_name()
        .and_then(|name| name.to_str())
        .unwrap_or("file");
    let sanitized: String = base
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || matches!(character, '.' | '-' | '_') {
                character
            } else {
                '_'
            }
        })
        .collect();
    if sanitized.is_empty() || sanitized == "." || sanitized == ".." {
        "file".into()
    } else {
        sanitized
    }
}

fn open_local(path: &Path) -> Result<()> {
    let (program, extra): (&str, &[&str]) = if clip::command_exists("xdg-open") {
        ("xdg-open", &[])
    } else if clip::command_exists("gio") {
        ("gio", &["open"])
    } else if clip::command_exists("kde-open") {
        ("kde-open", &[])
    } else {
        bail!("no local opener found; install xdg-utils (xdg-open)");
    };
    let output = Command::new(program)
        .args(extra)
        .arg(path)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .output()
        .with_context(|| format!("failed to run {program}"))?;
    if !output.status.success() {
        let stderr = String::from_utf8_lossy(&output.stderr);
        bail!(
            "{program} could not open {}: {stderr}",
            path.display(),
            stderr = stderr.trim()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_absolute_path() {
        assert_eq!(parse_remote_ref("/tmp/a.jpg").unwrap(), "/tmp/a.jpg");
    }

    #[test]
    fn parses_file_url() {
        assert_eq!(parse_remote_ref("file:///tmp/a.jpg").unwrap(), "/tmp/a.jpg");
        assert_eq!(
            parse_remote_ref("FILE://localhost/tmp/a%20b.jpg").unwrap(),
            "/tmp/a b.jpg"
        );
        assert_eq!(
            parse_remote_ref("<file:///home/you/pic.png>").unwrap(),
            "/home/you/pic.png"
        );
    }

    #[test]
    fn parses_tilde_and_quotes() {
        assert_eq!(parse_remote_ref("~/clip.png").unwrap(), "~/clip.png");
        assert_eq!(
            parse_remote_ref("\"/tmp/quoted.jpg\"").unwrap(),
            "/tmp/quoted.jpg"
        );
    }

    #[test]
    fn extracts_path_from_grok_output() {
        let pasted = "imagine Documentary-style candid portrait…\n\
/home/thuanlee/.grok/sessions//home/thuanlee/01a08bde-4a20-7043-8220-8593152455b0/images/1.jpg";
        assert_eq!(
            parse_remote_ref(pasted).unwrap(),
            "/home/thuanlee/.grok/sessions//home/thuanlee/01a08bde-4a20-7043-8220-8593152455b0/images/1.jpg"
        );
    }

    #[test]
    fn rejects_http_and_relative() {
        assert!(parse_remote_ref("https://example.com/a.jpg").is_err());
        assert!(parse_remote_ref("images/1.jpg").is_err());
        assert!(parse_remote_ref("").is_err());
    }

    #[test]
    fn sanitizes_local_basename() {
        assert_eq!(local_basename("/tmp/a b.jpg"), "a_b.jpg");
        assert_eq!(local_basename("/tmp/.."), "file");
        assert_eq!(local_basename("/tmp/1.jpg"), "1.jpg");
    }
}
