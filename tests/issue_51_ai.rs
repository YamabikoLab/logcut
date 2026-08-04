#![cfg(target_os = "linux")]

mod common;

use common::TestDir;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const AI_SECRET_KEYS: [&str; 27] = [
    "CODEX_API_KEY",
    "OPENAI_API_KEY",
    "ANTHROPIC_API_KEY",
    "ANTHROPIC_AUTH_TOKEN",
    "COPILOT_GITHUB_TOKEN",
    "COPILOT_PROVIDER_API_KEY",
    "GH_TOKEN",
    "GITHUB_TOKEN",
    "CURSOR_API_KEY",
    "GEMINI_API_KEY",
    "GOOGLE_API_KEY",
    "CONTINUE_API_KEY",
    "SRC_ACCESS_TOKEN",
    "AIDER_OPENAI_API_KEY",
    "AIDER_ANTHROPIC_API_KEY",
    "AIDER_API_KEY",
    "AZURE_OPENAI_API_KEY",
    "OPENROUTER_API_KEY",
    "DEEPSEEK_API_KEY",
    "GROQ_API_KEY",
    "MISTRAL_API_KEY",
    "XAI_API_KEY",
    "COHERE_API_KEY",
    "TOGETHER_API_KEY",
    "PERPLEXITY_API_KEY",
    "HF_TOKEN",
    "HUGGING_FACE_HUB_TOKEN",
];

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

fn retained_log(root: &Path) -> PathBuf {
    let entries = fs::read_dir(root.join("logs"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "log"))
        .collect::<Vec<_>>();
    assert_eq!(entries.len(), 1, "{entries:?}");
    entries.into_iter().next().unwrap()
}

#[test]
fn masks_ai_coding_tool_and_provider_credentials() {
    let root = TestDir::new("logcut-issue-51", "ai-credentials");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    let mut body = String::new();
    for (index, key) in AI_SECRET_KEYS.iter().enumerate() {
        body.push_str(&format!("printf '%s\\n' '{key}=ai-secret-{index:02}'\n"));
    }
    body.push_str("exit 17");
    write_fake_command(&bin, "ai-credentials", body.as_bytes());

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    let output = Command::new(binary())
        .env("PATH", path)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("ai-credentials")
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(17), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    for index in 0..AI_SECRET_KEYS.len() {
        let secret = format!("ai-secret-{index:02}");
        assert!(!log.contains(&secret), "{log}");
        assert!(!text.contains(&secret), "{text}");
    }
    assert_eq!(log.matches("[REDACTED]").count(), AI_SECRET_KEYS.len());
}

#[test]
fn masks_complete_unquoted_space_and_json_values() {
    let root = TestDir::new("logcut-issue-51", "complex-values");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    let spaced_secret = "correct horse battery staple";
    let json_secret = "{\"first\":\"safe\",\"credential\":\"leaky secret\"}";
    let body = format!(
        "printf '%s\\n' 'MY_SECRET={spaced_secret}'\nprintf '%s\\n' 'GOOGLE_CLOUD_KEYFILE_JSON={json_secret}'\nexit 17"
    );
    write_fake_command(&bin, "complex-values", body.as_bytes());

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    let output = Command::new(binary())
        .env("PATH", path)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("complex-values")
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(17), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    for secret_fragment in [
        spaced_secret,
        "horse battery staple",
        json_secret,
        "credential",
        "leaky secret",
    ] {
        assert!(!log.contains(secret_fragment), "{log}");
        assert!(!text.contains(secret_fragment), "{text}");
    }
    assert_eq!(log.matches("[REDACTED]").count(), 2, "{log}");
    assert_eq!(text.matches("[REDACTED]").count(), 2, "{text}");
}

#[test]
fn masks_secret_values_that_begin_with_redacted_marker() {
    let root = TestDir::new("logcut-issue-51", "redacted-prefix");
    let bin = root.join("bin");
    fs::create_dir_all(&bin).unwrap();

    write_fake_command(
        &bin,
        "redacted-prefix",
        b"printf '%s\\n' 'MY_SECRET=[REDACTED] actual-secret'\nprintf '%s\\n' 'OTHER_TOKEN=[REDACTED],delimiter-secret'\nexit 17",
    );

    let inherited_path = std::env::var_os("PATH").unwrap_or_default();
    let path = format!("{}:{}", bin.display(), inherited_path.to_string_lossy());
    let output = Command::new(binary())
        .env("PATH", path)
        .env("LOGCUT_LOG_DIRECTORY", root.join("logs"))
        .arg("redacted-prefix")
        .output()
        .unwrap();
    let text = combined(&output);
    assert_eq!(output.status.code(), Some(17), "{text}");

    let log = fs::read_to_string(retained_log(&root)).unwrap();
    for secret in ["actual-secret", "delimiter-secret"] {
        assert!(!log.contains(secret), "{log}");
        assert!(!text.contains(secret), "{text}");
    }
    assert_eq!(log.matches("[REDACTED]").count(), 2, "{log}");
    assert_eq!(text.matches("[REDACTED]").count(), 2, "{text}");
}
