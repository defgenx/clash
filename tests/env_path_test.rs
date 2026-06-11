//! Process-level test for login-shell PATH adoption (runs in its own
//! process, so mutating PATH/SHELL here cannot race other tests).
//!
//! `SHELL` is pointed at controlled stand-ins so the test never depends on
//! the host's real shell configuration (CI runners' login shells often have
//! no home-relative PATH entries).
#![cfg(unix)]

use clash::infrastructure::env_path::adopt_login_shell_path;
use std::io::Write;
use std::os::unix::fs::PermissionsExt;

const LAUNCHD_PATH: &str = "/usr/bin:/bin:/usr/sbin:/sbin";

/// Write an executable stub shell that ignores its arguments and prints a
/// fixed PATH containing a home-relative entry.
fn write_stub_shell(dir: &std::path::Path) -> std::path::PathBuf {
    let path = dir.join("stub-shell.sh");
    let mut f = std::fs::File::create(&path).expect("create stub shell");
    f.write_all(b"#!/bin/sh\nprintf '%s' \"$HOME/.local/bin:/opt/stub/bin\"\n")
        .expect("write stub shell");
    drop(f);
    std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o755))
        .expect("chmod stub shell");
    path
}

#[test]
fn adopts_user_path_when_launched_with_launchd_path() {
    let home = dirs::home_dir().expect("home dir");
    let home = home.to_string_lossy();
    let tmp = std::env::temp_dir().join(format!("clash-env-path-test-{}", std::process::id()));
    std::fs::create_dir_all(&tmp).expect("temp dir");

    // --- Login-shell path: SHELL answers with a home-relative PATH. ---
    let stub = write_stub_shell(&tmp);
    std::env::set_var("SHELL", &stub);
    std::env::set_var("PATH", LAUNCHD_PATH);
    adopt_login_shell_path();

    let path = std::env::var("PATH").unwrap();
    assert!(
        path.split(':').any(|e| e.starts_with(home.as_ref())),
        "PATH should gain home-relative entries from the login shell, got: {path}"
    );
    assert!(
        path.split(':').any(|e| e == "/opt/stub/bin"),
        "PATH should gain the stub shell's entries, got: {path}"
    );
    // Original entries must survive the merge.
    assert!(path.split(':').any(|e| e == "/usr/bin"));

    // Second call is a no-op (PATH now contains home entries).
    let before = path.clone();
    adopt_login_shell_path();
    assert_eq!(std::env::var("PATH").unwrap(), before);

    // --- Fallback path: SHELL can't be run, well-known dirs are merged. ---
    std::env::set_var("SHELL", tmp.join("no-such-shell"));
    std::env::set_var("PATH", LAUNCHD_PATH);
    adopt_login_shell_path();

    let path = std::env::var("PATH").unwrap();
    assert!(
        path.split(':').any(|e| e.starts_with(home.as_ref())),
        "PATH should gain home-relative entries from the fallback dirs, got: {path}"
    );
    assert!(path.split(':').any(|e| e == "/usr/bin"));

    let _ = std::fs::remove_dir_all(&tmp);
}
