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
pub async fn check_for_update() -> Option<UpdateCheck> {
    let url = format!("https://api.github.com/repos/{}/releases/latest", REPO);

    let output = tokio::process::Command::new("curl")
        .args([
            "-fsSL",
            "-H",
            "Accept: application/vnd.github.v3+json",
            &url,
        ])
        .output()
        .await
        .ok()?;

    if !output.status.success() {
        return None;
    }

    let body = String::from_utf8(output.stdout).ok()?;
    let json: serde_json::Value = serde_json::from_str(&body).ok()?;

    let tag = json.get("tag_name")?.as_str()?;
    let latest = tag.strip_prefix('v').unwrap_or(tag);

    if is_newer(latest, CURRENT_VERSION) {
        let download_url = build_download_url(tag);
        Some(UpdateCheck::Available {
            version: latest.to_string(),
            download_url,
        })
    } else {
        Some(UpdateCheck::UpToDate)
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
}
