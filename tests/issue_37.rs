#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::thread;
use std::time::{Duration, Instant};

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn combined(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

fn write_fake_command(directory: &Path, name: &str, body: &[u8]) {
    let path = directory.join(name);
    let mut script = b"#!/bin/sh\n".to_vec();
    script.extend_from_slice(body);
    script.push(b'\n');
    fs::write(&path, script).unwrap();
    let mut permissions = fs::metadata(&path).unwrap().permissions();
    permissions.set_mode(0o755);
    fs::set_permissions(path, permissions).unwrap();
}

fn run_fake(name: &str, body: &[u8]) -> (TestDir, Output) {
    let root = TestDir::new("logcut-issue-37", name);
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();
    write_fake_command(&bin, name, body);

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    let output = Command::new(binary())
        .env("PATH", path)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg(name)
        .output()
        .unwrap();
    (root, output)
}

fn retained_log(root: &Path) -> PathBuf {
    let logs = root.join("logs");
    let entries = fs::read_dir(logs)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries.into_iter().next().unwrap()
}

#[test]
fn masks_stdout_and_stderr_in_the_retained_log_and_summary() {
    let (root, output) = run_fake(
        "secrets",
        br#"printf '%s\n' 'access_token=stdout-secret' '{"password":"json secret","api-key":"quoted key"}'
printf '%s\n' 'Authorization: Bearer stderr-secret' 'https://user:pass@example.invalid/repo' >&2
exit 42"#,
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(42), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    for secret in [
        "stdout-secret",
        "json secret",
        "quoted key",
        "stderr-secret",
        "user:pass",
    ] {
        assert!(!log.contains(secret), "{log}");
        assert!(!text.contains(secret), "{text}");
    }
    assert!(log.matches("[REDACTED]").count() >= 5, "{log}");
    assert!(text.contains("[REDACTED]"), "{text}");
}

#[test]
fn masks_multiple_case_insensitive_assignments_without_changing_normal_output() {
    let (root, output) = run_fake(
        "assignments",
        br#"printf '%s\n' 'ordinary output stays here' 'TOKEN=first SECRET: second passwd="third value" refresh_token='"'"'fourth value'"'"'' >&2
exit 7"#,
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(7), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    assert!(log.contains("ordinary output stays here"), "{log}");
    for secret in ["first", "second", "third value", "fourth value"] {
        assert!(!log.contains(secret), "{log}");
    }
    assert_eq!(log.matches("[REDACTED]").count(), 4, "{log}");
}

#[test]
fn invalid_utf8_and_control_bytes_do_not_crash_log_masking() {
    let (root, output) = run_fake(
        "binary-output",
        b"printf 'normal\\000bytes\\n'\nprintf 'token=hidden\\377value\\n' >&2\nexit 9",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(9), "{text}");

    let log = fs::read(retained_log(&root)).unwrap();
    assert!(log.starts_with(b"normal\0bytes\n"), "{log:?}");
    assert!(!log.windows(b"hidden".len()).any(|value| value == b"hidden"));
    assert!(!log.windows(b"value".len()).any(|value| value == b"value"));
    assert!(log
        .windows(b"[REDACTED]".len())
        .any(|value| value == b"[REDACTED]"));
}

#[test]
fn large_output_is_processed_without_changing_the_exit_code() {
    let (root, output) = run_fake(
        "large-output",
        b"i=0\nwhile [ \"$i\" -lt 20000 ]; do\n  printf 'line-%s password=secret-%s\\n' \"$i\" \"$i\"\n  i=$((i + 1))\ndone\nexit 23",
    );
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(23), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    assert!(log.contains("line-19999"), "missing final line");
    assert!(!log.contains("secret-"), "{log}");
    assert!(log.contains("[REDACTED]"), "{log}");
}

#[test]
fn background_writer_does_not_extend_log_finalization_or_get_sigpipe() {
    let root = TestDir::new("logcut-issue-37", "background-writer");
    let bin = root.join("bin");
    let marker = root.join("background-finished");
    fs::create_dir_all(&bin).unwrap();
    write_fake_command(
        &bin,
        "background-writer",
        br#"(
  i=0
  while [ "$i" -lt 200 ]; do
    printf 'password=background-secret-%s\n' "$i"
    i=$((i + 1))
    sleep 0.01
  done
  printf '%s\n' finished > "$BACKGROUND_MARKER"
) &
printf '%s\n' 'password=foreground-secret'
exit 7"#,
    );

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    let started = Instant::now();
    let output = Command::new(binary())
        .env("PATH", path)
        .env("BACKGROUND_MARKER", &marker)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("background-writer")
        .output()
        .unwrap();
    let elapsed = started.elapsed();
    let text = combined(&output);

    assert_eq!(output.status.code(), Some(7), "{text}");
    assert!(
        elapsed < Duration::from_secs(1),
        "log finalization followed background writes for {elapsed:?}"
    );

    let deadline = Instant::now() + Duration::from_secs(4);
    while !marker.exists() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
    assert!(
        marker.exists(),
        "background writer did not survive logcut exit; output: {text}"
    );

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    assert!(!log.contains("foreground-secret"), "{log}");
    assert!(!log.contains("background-secret"), "{log}");
    assert!(log.contains("[REDACTED]"), "{log}");
}
