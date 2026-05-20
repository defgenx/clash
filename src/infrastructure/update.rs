//! Self-update functionality — checks for and installs new versions from GitHub releases.

use std::path::{Path, PathBuf};

const REPO: &str = "defgenx/clash";
const CURRENT_VERSION: &str = env!("CARGO_PKG_VERSION");

/// Result of checking for updates.
#[derive(Debug, Clone)]
pub enum UpdateCheck {
    /// A newer version is available.
    Available {
        version: String,
        download_url: String,
    },
    /// Already on the latest version.
    UpToDate,
}

/// Check GitHub for the latest release version.
/// Returns `None` on network errors (silent fail for background checks).
///
/// Reads the redirect target of `github.com/REPO/releases/latest`
/// rather than the REST API — the API caps unauthenticated callers at
/// 60 requests/hour/IP and was returning 403s for users behind a busy
/// NAT. The HTML site has no such limit.
pub async fn check_for_update() -> Option<UpdateCheck> {
    let tag = fetch_latest_tag(&format!("https://github.com/{}/releases/latest", REPO)).await?;
    let latest = tag.strip_prefix('v').unwrap_or(&tag);

    if is_newer(latest, CURRENT_VERSION) {
        let download_url = build_download_url(&tag);
        Some(UpdateCheck::Available {
            version: latest.to_string(),
            download_url,
        })
    } else {
        Some(UpdateCheck::UpToDate)
    }
}

/// Send a HEAD to `releases/latest` and return the tag from the
/// `Location:` header (e.g. `…/releases/tag/v1.2.3` → `"v1.2.3"`).
async fn fetch_latest_tag(url: &str) -> Option<String> {
    // -I = HEAD, -s/-S = silent except errors, no -L so the 302 is the
    // *response we receive*, not a thing curl chases. No -f either:
    // 302 is the success case here.
    let output = tokio::process::Command::new("curl")
        .args(["-sSI", url])
        .output()
        .await
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let body = String::from_utf8(output.stdout).ok()?;
    parse_latest_tag_from_headers(&body)
}

/// Pull the tag basename out of a `Location:` header in raw HTTP
/// response headers. Pure so it can be unit-tested without curl.
fn parse_latest_tag_from_headers(headers: &str) -> Option<String> {
    let location = headers.lines().find_map(|line| {
        let (name, value) = line.split_once(':')?;
        if name.trim().eq_ignore_ascii_case("location") {
            Some(value.trim().to_string())
        } else {
            None
        }
    })?;
    let tag = location.rsplit('/').next()?.trim();
    if tag.is_empty() {
        None
    } else {
        Some(tag.to_string())
    }
}

/// Perform the update from the CLI (blocking, returns result).
pub async fn perform_update_cli() -> Result<String, String> {
    let check = check_for_update()
        .await
        .ok_or_else(|| "Failed to check for updates (network error)".to_string())?;

    match check {
        UpdateCheck::UpToDate => Err(format!(
            "Already on the latest version ({})",
            CURRENT_VERSION
        )),
        UpdateCheck::Available {
            version,
            download_url,
        } => {
            install_update(&download_url).await?;
            Ok(version)
        }
    }
}

/// Perform the update with TUI progress reporting.
/// Sends progress phases through `tx` so the TUI can display them.
pub async fn perform_update(
    tx: tokio::sync::mpsc::UnboundedSender<crate::application::state::UpdatePhase>,
) {
    use crate::application::state::UpdatePhase;

    let _ = tx.send(UpdatePhase::Checking);

    let check = match check_for_update().await {
        Some(c) => c,
        None => {
            let _ = tx.send(UpdatePhase::Failed {
                message: "Network error while checking for updates".to_string(),
            });
            return;
        }
    };

    match check {
        UpdateCheck::UpToDate => {
            let _ = tx.send(UpdatePhase::Failed {
                message: format!("Already on the latest version ({})", CURRENT_VERSION),
            });
        }
        UpdateCheck::Available {
            version,
            download_url,
        } => {
            let _ = tx.send(UpdatePhase::Downloading {
                version: version.clone(),
            });

            if let Err(msg) = install_update_phased(&download_url, &tx).await {
                let _ = tx.send(UpdatePhase::Failed { message: msg });
                return;
            }

            let _ = tx.send(UpdatePhase::Done { version });
        }
    }
}

/// Download the release tarball and replace the current binary, reporting phases.
async fn install_update_phased(
    download_url: &str,
    tx: &tokio::sync::mpsc::UnboundedSender<crate::application::state::UpdatePhase>,
) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Cannot determine current binary path: {}", e))?;
    let install_target = resolve_install_target(&current_exe)?;

    let tmpdir = std::env::temp_dir().join("clash-update");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let tarball = tmpdir.join("clash.tar.gz");

    // Download
    let status = tokio::process::Command::new("curl")
        .args(["-fsSL", "-o", tarball.to_str().unwrap(), download_url])
        .status()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !status.success() {
        return Err("Download failed (curl returned non-zero)".to_string());
    }

    let _ = tx.send(crate::application::state::UpdatePhase::Extracting);

    // Extract
    let status = tokio::process::Command::new("tar")
        .args([
            "xzf",
            tarball.to_str().unwrap(),
            "-C",
            tmpdir.to_str().unwrap(),
        ])
        .status()
        .await
        .map_err(|e| format!("Extraction failed: {}", e))?;

    if !status.success() {
        return Err("Extraction failed (tar returned non-zero)".to_string());
    }

    let new_binary = tmpdir.join("clash");
    if !new_binary.exists() {
        return Err("Binary not found in archive".to_string());
    }

    let _ = tx.send(crate::application::state::UpdatePhase::Installing);

    replace_binary(&new_binary, &install_target)?;

    // Cleanup
    let _ = std::fs::remove_dir_all(&tmpdir);

    Ok(())
}

/// Download the release tarball and replace the current binary (CLI path, no progress).
async fn install_update(download_url: &str) -> Result<(), String> {
    let current_exe = std::env::current_exe()
        .map_err(|e| format!("Cannot determine current binary path: {}", e))?;
    let install_target = resolve_install_target(&current_exe)?;

    let tmpdir = std::env::temp_dir().join("clash-update");
    let _ = std::fs::remove_dir_all(&tmpdir);
    std::fs::create_dir_all(&tmpdir).map_err(|e| format!("Failed to create temp dir: {}", e))?;

    let tarball = tmpdir.join("clash.tar.gz");

    let status = tokio::process::Command::new("curl")
        .args(["-fsSL", "-o", tarball.to_str().unwrap(), download_url])
        .status()
        .await
        .map_err(|e| format!("Download failed: {}", e))?;

    if !status.success() {
        return Err("Download failed (curl returned non-zero)".to_string());
    }

    let status = tokio::process::Command::new("tar")
        .args([
            "xzf",
            tarball.to_str().unwrap(),
            "-C",
            tmpdir.to_str().unwrap(),
        ])
        .status()
        .await
        .map_err(|e| format!("Extraction failed: {}", e))?;

    if !status.success() {
        return Err("Extraction failed (tar returned non-zero)".to_string());
    }

    let new_binary = tmpdir.join("clash");
    if !new_binary.exists() {
        return Err("Binary not found in archive".to_string());
    }

    replace_binary(&new_binary, &install_target)?;

    let _ = std::fs::remove_dir_all(&tmpdir);

    Ok(())
}

/// Determine where to install the updated binary.
///
/// If the directory containing the current executable is writable, install there.
/// Otherwise fall back to `~/.local/bin` (created if needed).
fn resolve_install_target(current_exe: &Path) -> Result<PathBuf, String> {
    if let Some(dir) = current_exe.parent() {
        if is_dir_writable(dir) {
            return Ok(current_exe.to_path_buf());
        }
    }

    // Fall back to ~/.local/bin
    let home = std::env::var("HOME").map_err(|_| "HOME not set".to_string())?;
    let fallback_dir = PathBuf::from(home).join(".local").join("bin");
    std::fs::create_dir_all(&fallback_dir)
        .map_err(|e| format!("Failed to create {}: {}", fallback_dir.display(), e))?;
    let target = fallback_dir.join("clash");
    Ok(target)
}

/// Check whether a directory is writable by the current user.
fn is_dir_writable(dir: &std::path::Path) -> bool {
    let probe = dir.join(".clash-write-probe");
    if std::fs::write(&probe, b"").is_ok() {
        let _ = std::fs::remove_file(&probe);
        true
    } else {
        false
    }
}

/// Replace the running binary with the new one.
///
/// On macOS, overwriting a binary in-place invalidates its code signature
/// (the kernel kills the process with SIGKILL "Code Signature Invalid").
/// We must remove the old file first so the replacement gets a fresh inode,
/// then ad-hoc re-sign on macOS.
fn replace_binary(new: &PathBuf, target: &PathBuf) -> Result<(), String> {
    // Remove existing binary first to get a fresh inode (critical for macOS codesigning)
    let _ = std::fs::remove_file(target);

    // Try direct rename first
    if std::fs::rename(new, target).is_ok() {
        set_permissions_and_sign(target);
        return Ok(());
    }

    // If rename fails (cross-device), try copy
    if std::fs::copy(new, target).is_ok() {
        set_permissions_and_sign(target);
        return Ok(());
    }

    Err(format!("Cannot write to {}", target.display()))
}

/// Set executable permissions and ad-hoc codesign on macOS.
fn set_permissions_and_sign(path: &PathBuf) {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755));
    }
    codesign(path);
}

/// Ad-hoc codesign a binary on macOS (no-op on other platforms).
fn codesign(path: &Path) {
    #[cfg(target_os = "macos")]
    {
        let _ = std::process::Command::new("codesign")
            .args(["--force", "--sign", "-", path.to_str().unwrap()])
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status();
    }
    #[cfg(not(target_os = "macos"))]
    {
        let _ = path;
    }
}

/// Build the platform-specific download URL for a given tag.
fn build_download_url(tag: &str) -> String {
    let target = current_target();
    format!(
        "https://github.com/{}/releases/download/{}/clash-{}.tar.gz",
        REPO, tag, target
    )
}

/// Detect the current platform's Rust target triple.
fn current_target() -> &'static str {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        "x86_64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "linux", target_arch = "aarch64"))]
    {
        "aarch64-unknown-linux-gnu"
    }
    #[cfg(all(target_os = "macos", target_arch = "x86_64"))]
    {
        "x86_64-apple-darwin"
    }
    #[cfg(all(target_os = "macos", target_arch = "aarch64"))]
    {
        "aarch64-apple-darwin"
    }
    #[cfg(not(any(
        all(target_os = "linux", target_arch = "x86_64"),
        all(target_os = "linux", target_arch = "aarch64"),
        all(target_os = "macos", target_arch = "x86_64"),
        all(target_os = "macos", target_arch = "aarch64"),
    )))]
    {
        "unknown"
    }
}

/// Compare semver strings. Returns true if `latest` is newer than `current`.
fn is_newer(latest: &str, current: &str) -> bool {
    let parse = |s: &str| -> (u32, u32, u32) {
        let mut parts = s.split('.');
        let major = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let minor = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
        (major, minor, patch)
    };
    parse(latest) > parse(current)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_is_newer() {
        assert!(is_newer("1.2.0", "1.1.0"));
        assert!(is_newer("2.0.0", "1.9.9"));
        assert!(is_newer("1.1.1", "1.1.0"));
        assert!(!is_newer("1.1.0", "1.1.0"));
        assert!(!is_newer("1.0.0", "1.1.0"));
        assert!(!is_newer("0.9.0", "1.0.0"));
    }

    #[test]
    fn test_build_download_url() {
        let url = build_download_url("v1.2.0");
        assert!(url.contains("defgenx/clash"));
        assert!(url.contains("v1.2.0"));
        assert!(url.ends_with(".tar.gz"));
    }

    #[test]
    fn test_parse_latest_tag_from_headers() {
        let headers = "HTTP/2 302\n\
            date: Wed, 20 May 2026 11:55:49 GMT\n\
            location: https://github.com/defgenx/clash/releases/tag/v1.35.3\n\
            content-length: 0\n";
        assert_eq!(
            parse_latest_tag_from_headers(headers),
            Some("v1.35.3".to_string())
        );
    }

    #[test]
    fn test_parse_latest_tag_case_insensitive() {
        // Some proxies upper-case header names.
        let headers = "HTTP/2 302\nLOCATION: https://github.com/foo/bar/releases/tag/v2.0.0\n";
        assert_eq!(
            parse_latest_tag_from_headers(headers),
            Some("v2.0.0".to_string())
        );
    }

    #[test]
    fn test_parse_latest_tag_no_location() {
        let headers = "HTTP/2 200\ncontent-type: text/html\n";
        assert_eq!(parse_latest_tag_from_headers(headers), None);
    }

    #[test]
    fn test_parse_latest_tag_empty_trailing_segment() {
        // Trailing slash would yield empty basename — must return None
        // rather than a phantom version string.
        let headers = "HTTP/2 302\nlocation: https://github.com/foo/bar/releases/tag/\n";
        assert_eq!(parse_latest_tag_from_headers(headers), None);
    }
}
