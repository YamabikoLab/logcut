#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::process::Command;

fn binary() -> &'static str {
    env!("CARGO_BIN_EXE_logcut")
}

fn log_files(directory: &std::path::Path) -> Vec<std::path::PathBuf> {
    fs::read_dir(directory)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with("command.") && name.ends_with(".log"))
        })
        .collect()
}

#[test]
fn oversized_output_is_truncated_without_changing_exit_code() {
    let root = TestDir::new("logcut-issue-52", "truncate");
    let logs = root.join("logs");
    let output = Command::new(binary())
        .env("LOGCUT_LOG_DIRECTORY", &logs)
        .args([
            "sh",
            "-c",
            "yes 0123456789abcdef | head -c 11534336; exit 52",
        ])
        .output()
        .unwrap();

    assert_eq!(output.status.code(), Some(52));
    let stderr = String::from_utf8_lossy(&output.stderr);
    assert!(stderr.contains("command output exceeded 10 MiB and was truncated"));

    let files = log_files(&logs);
    assert_eq!(files.len(), 1);
    let metadata = fs::metadata(&files[0]).unwrap();
    assert!(metadata.len() <= 10 * 1024 * 1024);

    let log = fs::read_to_string(&files[0]).unwrap();
    assert!(log.contains("[logcut: command output truncated at 10 MiB]"));
}
