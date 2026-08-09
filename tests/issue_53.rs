#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

#[test]
fn successful_command_warns_but_keeps_zero_exit_when_log_cleanup_fails() {
    if unsafe { libc::geteuid() } == 0 {
        return;
    }

    let root = TestDir::new("logcut-issue-53", "success-cleanup-failure");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            r#"printf 'token=success-secret\033[31m\n'
chmod 0500 "$LOGCUT_LOG_DIRECTORY""#,
        ])
        .output()
        .unwrap();

    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    let stderr = String::from_utf8_lossy(&output.stderr);

    assert_eq!(output.status.code(), Some(0), "stderr: {stderr}");
    assert!(stdout.contains("PASS ("), "{stdout}");
    assert!(
        stderr.contains("failed to remove successful command log"),
        "{stderr}"
    );
    assert!(
        stderr.contains("failed to sanitize the remaining log"),
        "{stderr}"
    );
    assert!(
        stderr.contains("warning: unmasked or unsafe terminal-control data may remain"),
        "{stderr}"
    );
    assert!(!stdout.contains("success-secret"), "{stdout}");
    assert!(!stderr.contains("success-secret"), "{stderr}");

    let retained_logs = fs::read_dir(&logs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    assert_eq!(retained_logs.len(), 1, "{retained_logs:?}");
}
