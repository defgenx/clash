//! Process-level test for login-shell PATH adoption (runs in its own
//! process, so mutating PATH here cannot race other tests).

use clash::infrastructure::env_path::adopt_login_shell_path;

#[test]
fn adopts_user_path_when_launched_with_launchd_path() {
    let home = dirs::home_dir().expect("home dir");
    let home = home.to_string_lossy();

    // Simulate a Finder/Dock launch: launchd's minimal PATH.
    std::env::set_var("PATH", "/usr/bin:/bin:/usr/sbin:/sbin");
    adopt_login_shell_path();

    let path = std::env::var("PATH").unwrap();
    assert!(
        path.split(':').any(|e| e.starts_with(home.as_ref())),
        "PATH should gain home-relative entries (login shell or fallback), got: {path}"
    );
    // Original entries must survive the merge.
    assert!(path.split(':').any(|e| e == "/usr/bin"));

    // Second call is a no-op (PATH now contains home entries).
    let before = path.clone();
    adopt_login_shell_path();
    assert_eq!(std::env::var("PATH").unwrap(), before);
}
