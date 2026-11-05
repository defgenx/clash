//! One hardened HTTPS POST for the tracker clients (`jira`, `linear`).
//!
//! `curl` with stdin closed, a hard timeout and `--fail-with-body`, the
//! `infrastructure::webhook` approach. The body and the credentials travel
//! through files, never argv: argv is visible to every local process and a
//! body can exceed its limit. The credentials file is written 0600.

use std::io::Read;
use std::process::{Command, Stdio};
use std::time::Duration;

/// One POST, small payload — anything past this is a dead connection.
const SEND_TIMEOUT: Duration = Duration::from_secs(15);

/// Write `content` readable by the owner only (the file holds credentials).
pub fn write_private(path: &std::path::Path, content: &str) -> Result<(), String> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| e.to_string())?;
    f.write_all(content.as_bytes()).map_err(|e| e.to_string())
}

/// POST the JSON in `body_file` to `url`, with curl options (`user = …`,
/// `header = …`) read from `config_file`. Returns the response body. `label`
/// names the service in errors.
pub fn post_json(
    url: &str,
    body_file: &std::path::Path,
    config_file: &std::path::Path,
    label: &str,
) -> Result<String, String> {
    let mut child = Command::new("curl")
        .arg("-sS")
        .arg("--max-time")
        .arg(SEND_TIMEOUT.as_secs().to_string())
        .arg("--fail-with-body")
        .arg("-K")
        .arg(config_file)
        .arg("-X")
        .arg("POST")
        .arg("-H")
        .arg("Content-Type: application/json")
        .arg("--data")
        .arg(format!("@{}", body_file.display()))
        .arg(url)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("could not run curl: {}", e))?;

    let mut stdout = String::new();
    let mut stderr = String::new();
    if let Some(mut pipe) = child.stdout.take() {
        let _ = pipe.read_to_string(&mut stdout);
    }
    if let Some(mut pipe) = child.stderr.take() {
        let _ = pipe.read_to_string(&mut stderr);
    }
    // curl's own --max-time bounds the call; wait() cannot hang past it.
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(stdout)
    } else {
        // The service's error body says *why* (bad ticket, no permission)
        // where curl's stderr only says 4xx — surface both, briefly.
        let detail: String = format!("{} {}", stderr.trim(), stdout.trim())
            .trim()
            .chars()
            .take(300)
            .collect();
        Err(format!("{} POST failed: {}", label, detail))
    }
}
