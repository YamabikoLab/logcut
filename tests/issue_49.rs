#![cfg(target_os = "linux")]

mod common;

use common::{TestDir, LOG_DIRECTORY_MARKER};
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn combined(output: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn assert_tampered_marker_prevents_post_command_pruning(name: &str, action: &str) {
    let root = TestDir::new("logcut-issue-49", name);
    let logs = root.join("custom-logs");
    let script = format!(
        "{action}; \
         printf 'do not remove' > \"$1/command.user-owned.log\"; \
         touch -t 200001010000 \"$1/command.user-owned.log\"; \
         printf failure; \
         exit 37"
    );

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_LOG_MAX_AGE_DAYS", "1")
        .env("LOGCUT_LOG_MAX_FILES", "100")
        .arg("sh")
        .arg("-c")
        .arg(script)
        .arg("sh")
        .arg(&logs)
        .output()
        .unwrap();

    let user_owned = logs.join("command.user-owned.log");
    assert_eq!(output.status.code(), Some(37));
    assert_eq!(fs::read_to_string(user_owned).unwrap(), "do not remove");
}

#[test]
fn empty_existing_directory_is_initialized_without_changing_permissions() {
    let root = TestDir::new("logcut-issue-49", "empty-directory");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args(["sh", "-c", "printf failure; exit 7"])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(7));
    assert!(logs.join(LOG_DIRECTORY_MARKER).is_file());
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn existing_directory_permissions_are_not_changed() {
    let root = TestDir::new("logcut-issue-49", "permissions");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o755)).unwrap();
    let existing = logs.join("keep.txt");
    let child_ran = root.join("child-ran");
    fs::write(&existing, "keep").unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .arg("sh")
        .arg("-c")
        .arg("printf direct-output; touch \"$1\"; exit 23")
        .arg("sh")
        .arg(&child_ran)
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("secure logging is unavailable"));
    assert!(text.contains("command was not executed"));
    assert!(text.contains("Run the command without logcut"));
    assert!(!text.contains("direct-output"));
    assert!(!child_ran.exists());
    assert_eq!(
        fs::metadata(&logs).unwrap().permissions().mode() & 0o777,
        0o755
    );
    assert_eq!(fs::read_to_string(existing).unwrap(), "keep");
    assert!(!logs.join(LOG_DIRECTORY_MARKER).exists());
}

#[test]
fn unmarked_nonempty_directory_is_not_pruned() {
    let root = TestDir::new("logcut-issue-49", "unmarked");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    let existing = logs.join("command.user-owned.log");
    let child_ran = root.join("child-ran");
    fs::write(&existing, "do not remove").unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .env("LOGCUT_LOG_MAX_FILES", "1")
        .arg("sh")
        .arg("-c")
        .arg("printf direct-output; touch \"$1\"; exit 29")
        .arg("sh")
        .arg(&child_ran)
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("secure logging is unavailable"));
    assert!(text.contains("command was not executed"));
    assert!(text.contains("Run the command without logcut"));
    assert!(!text.contains("direct-output"));
    assert!(!child_ran.exists());
    assert_eq!(fs::read_to_string(existing).unwrap(), "do not remove");
    assert_eq!(fs::read_dir(&logs).unwrap().count(), 1);
}

#[test]
fn invalid_marker_is_rejected_without_replacing_it() {
    let root = TestDir::new("logcut-issue-49", "invalid-marker");
    let logs = root.join("custom-logs");
    fs::create_dir_all(&logs).unwrap();
    fs::set_permissions(&logs, fs::Permissions::from_mode(0o700)).unwrap();
    let marker = logs.join(LOG_DIRECTORY_MARKER);
    let child_ran = root.join("child-ran");
    fs::write(&marker, "not logcut\n").unwrap();
    fs::set_permissions(&marker, fs::Permissions::from_mode(0o600)).unwrap();

    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .arg("sh")
        .arg("-c")
        .arg("printf direct-output; touch \"$1\"; exit 31")
        .arg("sh")
        .arg(&child_ran)
        .output()
        .unwrap();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(1));
    assert!(text.contains("secure logging is unavailable"));
    assert!(text.contains("command was not executed"));
    assert!(text.contains("Run the command without logcut"));
    assert!(!text.contains("direct-output"));
    assert!(!child_ran.exists());
    assert_eq!(fs::read_to_string(marker).unwrap(), "not logcut\n");
}

#[test]
fn marker_removed_by_child_prevents_post_command_pruning() {
    assert_tampered_marker_prevents_post_command_pruning(
        "post-command-marker-removal",
        "rm \"$1/.logcut-directory\"",
    );
}

#[test]
fn marker_modified_by_child_prevents_post_command_pruning() {
    assert_tampered_marker_prevents_post_command_pruning(
        "post-command-marker-modification",
        "printf 'tampered\\n' > \"$1/.logcut-directory\"",
    );
}
