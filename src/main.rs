#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

#[cfg(not(target_os = "linux"))]
compile_error!("logcut currently supports Linux only");

mod logging;
mod process;
mod summary;

use logging::{normalize_output, prepare_log_file, prune_logs, read_log};
use process::{run_direct, run_suppressed};
use std::env;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::time::Instant;
use summary::{detect_profile, summarize, Profile};

extern "C" {
    fn getuid() -> u32;
    fn umask(mask: u32) -> u32;
}

pub(crate) struct Settings {
    pub(crate) profile: Profile,
    pub(crate) summary_lines: usize,
    pub(crate) max_errors: usize,
    pub(crate) max_log_files: usize,
    pub(crate) max_log_age_days: u64,
    pub(crate) log_directory: PathBuf,
}

pub(crate) struct SummarySettings {
    pub(crate) summary_lines: usize,
    pub(crate) max_errors: usize,
}

fn main() {
    let code = match run() {
        Ok(code) => code,
        Err(error) => {
            eprintln!("logcut: {error}");
            1
        }
    };
    std::process::exit(code);
}

fn run() -> io::Result<i32> {
    unsafe {
        umask(0o077);
    }

    let mut arguments: Vec<OsString> = env::args_os().skip(1).collect();
    let mut profile_value = env::var("LOGCUT_PROFILE").unwrap_or_else(|_| "auto".to_string());

    if let Some(first) = arguments.first().and_then(|value| value.to_str()) {
        if let Some(value) = first.strip_prefix("--profile=") {
            profile_value = value.to_string();
            arguments.remove(0);
        }
    }

    let Some(profile) = Profile::parse(&profile_value) else {
        eprintln!("Unknown profile: {profile_value}");
        return Ok(2);
    };

    if arguments.is_empty() {
        eprintln!("Usage: logcut [--profile=PROFILE] <command> [arguments...]");
        return Ok(2);
    }

    let settings = Settings {
        profile,
        summary_lines: positive_setting(
            "LOGCUT_SUMMARY_LINES",
            env::var("LOGCUT_SUMMARY_LINES")
                .ok()
                .or_else(|| env::var("LOGCUT_TAIL_LINES").ok()),
            40,
            200,
        ),
        max_errors: positive_setting(
            "LOGCUT_MAX_ERRORS",
            env::var("LOGCUT_MAX_ERRORS").ok(),
            20,
            100,
        ),
        max_log_files: positive_setting(
            "LOGCUT_LOG_MAX_FILES",
            env::var("LOGCUT_LOG_MAX_FILES").ok(),
            10,
            100,
        ),
        max_log_age_days: positive_setting(
            "LOGCUT_LOG_MAX_AGE_DAYS",
            env::var("LOGCUT_LOG_MAX_AGE_DAYS").ok(),
            7,
            30,
        ) as u64,
        log_directory: env::var_os("LOGCUT_LOG_DIRECTORY")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from(format!("/tmp/logcut-{}", unsafe { getuid() }))),
    };

    let label = command_label(&arguments);
    let start = Instant::now();
    println!("Running: {label}");
    let _ = io::stdout().flush();

    let log_path = match prepare_log_file(&settings) {
        Ok(path) => path,
        Err(_) => {
            eprintln!(
                "logcut: secure logging is unavailable; running command without output suppression"
            );
            return run_direct(&arguments);
        }
    };

    let status = match run_suppressed(&arguments, &log_path) {
        Ok(status) => status,
        Err(error) => {
            let _ = fs::remove_file(&log_path);
            eprintln!(
                "logcut: command setup failed ({error}); running command without output suppression"
            );
            return run_direct(&arguments);
        }
    };
    let elapsed = start.elapsed().as_secs();

    if status == 0 {
        println!("PASS ({elapsed}s): {label}");
        let _ = fs::remove_file(&log_path);
        return Ok(0);
    }

    let raw = read_log(&log_path)?;
    let clean = normalize_output(&raw);
    let selected = if settings.profile == Profile::Auto {
        detect_profile(&clean)
    } else {
        settings.profile
    };
    let summary_settings = SummarySettings {
        summary_lines: settings.summary_lines,
        max_errors: settings.max_errors,
    };
    let mut summary = summarize(selected, &clean, &summary_settings);
    if !summary.iter().any(|line| !line.trim().is_empty()) {
        summary = summarize(Profile::Generic, &clean, &summary_settings);
    }

    eprintln!("FAIL ({elapsed}s, exit {status}): {label}");
    eprintln!("\n----- Failure summary ({}) -----", selected.as_str());
    for line in summary {
        eprintln!("{line}");
    }
    eprintln!("\nFull log: {}", log_path.display());

    prune_logs(
        &settings.log_directory,
        settings.max_log_age_days,
        settings.max_log_files,
    );
    Ok(status)
}

fn positive_setting(name: &str, value: Option<String>, fallback: usize, maximum: usize) -> usize {
    let Some(value) = value else {
        return fallback;
    };
    match value.parse::<usize>() {
        Ok(number) if number > 0 && number <= maximum => number,
        _ => {
            eprintln!("logcut: invalid {name}={value:?}; using {fallback}");
            fallback
        }
    }
}

fn command_label(arguments: &[OsString]) -> String {
    let name = Path::new(&arguments[0])
        .file_name()
        .unwrap_or_else(|| OsStr::new(""))
        .to_string_lossy();
    let sanitized: String = name
        .chars()
        .filter(|character| character.is_ascii_alphanumeric() || "_.+-".contains(*character))
        .collect();
    let command = if sanitized.is_empty() {
        "<command>".to_string()
    } else {
        sanitized
    };
    let argument_count = arguments.len() - 1;
    if argument_count == 0 {
        command
    } else {
        format!("{command} [{argument_count} args]")
    }
}
