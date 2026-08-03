use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FAILURE_LOG_TAIL_BYTES: u64 = 1024 * 1024;
const SECONDS_PER_DAY: u64 = 86_400;
const SENSITIVE_KEYS: [&str; 12] = [
    "proxy-authorization",
    "authorization",
    "access_token",
    "refresh_token",
    "id_token",
    "x-api-key",
    "api-key",
    "api_token",
    "password",
    "passwd",
    "secret",
    "token",
];

pub(crate) fn prepare_log_file(settings: &crate::Settings) -> io::Result<PathBuf> {
    match fs::symlink_metadata(&settings.log_directory) {
        Ok(metadata) if metadata.file_type().is_symlink() => {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                "log directory must not be a symlink",
            ));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    fs::create_dir_all(&settings.log_directory)?;
    fs::set_permissions(&settings.log_directory, fs::Permissions::from_mode(0o700))?;
    let metadata = fs::metadata(&settings.log_directory)?;
    if !metadata.is_dir() || metadata.uid() != current_user_id() || metadata.mode() & 0o777 != 0o700
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "log directory ownership or permissions are unsafe",
        ));
    }

    prune_logs(
        &settings.log_directory,
        settings.max_log_age_days,
        settings.max_log_files,
    );

    create_unique_log(&settings.log_directory)
}

fn current_user_id() -> libc::uid_t {
    unsafe { libc::getuid() }
}

fn create_unique_log(directory: &Path) -> io::Result<PathBuf> {
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();
    for attempt in 0..1000u32 {
        let path = directory.join(format!(
            "command.{}.{}.{}.log",
            std::process::id(),
            nanos,
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&path)
        {
            Ok(_) => return Ok(path),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }
    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a unique log file",
    ))
}

pub(crate) fn prune_logs(directory: &Path, max_age_days: u64, max_files: usize) {
    let Ok(entries) = fs::read_dir(directory) else {
        return;
    };
    let now = SystemTime::now();
    let max_age = Duration::from_secs(max_age_days.saturating_mul(SECONDS_PER_DAY));
    let mut logs = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !name.starts_with("command.") || !name.ends_with(".log") {
            continue;
        }
        let Ok(file_type) = entry.file_type() else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(metadata) = entry.metadata() else {
            continue;
        };
        let modified = metadata.modified().unwrap_or(UNIX_EPOCH);
        if now.duration_since(modified).is_ok_and(|age| age > max_age) {
            let _ = fs::remove_file(&path);
        } else {
            logs.push((modified, path));
        }
    }

    logs.sort_by_key(|entry| Reverse(entry.0));
    for (_, path) in logs.into_iter().skip(max_files) {
        let _ = fs::remove_file(path);
    }
}

pub(crate) fn read_log(path: &Path) -> io::Result<Vec<u8>> {
    let mut file = File::open(path)?;
    let length = file.metadata()?.len();
    let start = length.saturating_sub(FAILURE_LOG_TAIL_BYTES);
    file.seek(SeekFrom::Start(start))?;

    let read_limit = length - start;
    let mut bytes = Vec::with_capacity(read_limit as usize);
    file.take(read_limit).read_to_end(&mut bytes)?;

    if start > 0 {
        if let Some(newline) = bytes.iter().position(|byte| *byte == b'\n') {
            bytes.drain(..=newline);
        } else {
            bytes.clear();
        }
    }

    Ok(bytes)
}

pub(crate) fn normalize_output(mut input: Vec<u8>) -> String {
    let mut read_index = 0;
    let mut write_index = 0;

    while read_index < input.len() {
        match input[read_index] {
            0x1b => read_index = skip_escape_sequence(&input, read_index),
            b'\n' | b'\t' => {
                input[write_index] = input[read_index];
                write_index += 1;
                read_index += 1;
            }
            byte if byte < 0x20 || byte == 0x7f => read_index += 1,
            byte => {
                input[write_index] = byte;
                write_index += 1;
                read_index += 1;
            }
        }
    }

    input.truncate(write_index);
    let mut output = match String::from_utf8(input) {
        Ok(output) => output,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    };
    output.retain(|character| character == '\n' || character == '\t' || !character.is_control());
    redact_sensitive_output(&output)
}

fn redact_sensitive_output(output: &str) -> String {
    let trailing_newline = output.ends_with('\n');
    let mut redacted = output
        .lines()
        .map(redact_sensitive_line)
        .collect::<Vec<_>>()
        .join("\n");
    if trailing_newline {
        redacted.push('\n');
    }
    redacted
}

fn redact_sensitive_line(line: &str) -> String {
    let mut redacted = redact_url_userinfo(line);
    for key in SENSITIVE_KEYS {
        redacted = redact_key_value(&redacted, key);
    }
    redacted
}

fn redact_url_userinfo(line: &str) -> String {
    let mut result = line.to_string();
    let mut search_from = 0usize;

    while let Some(relative) = result[search_from..].find("://") {
        let authority_start = search_from + relative + 3;
        let authority_end = result[authority_start..]
            .find(|character: char| character.is_whitespace() || matches!(character, '/' | '?' | '#'))
            .map_or(result.len(), |offset| authority_start + offset);
        let Some(at_offset) = result[authority_start..authority_end].find('@') else {
            search_from = authority_end.min(result.len());
            continue;
        };
        let at = authority_start + at_offset;
        result.replace_range(authority_start..at, "[REDACTED]");
        search_from = authority_start + "[REDACTED]@".len();
    }

    result
}

fn redact_key_value(line: &str, key: &str) -> String {
    let lower = line.to_ascii_lowercase();
    let mut search_from = 0usize;

    while let Some(relative) = lower[search_from..].find(key) {
        let key_start = search_from + relative;
        let key_end = key_start + key.len();
        let before_is_boundary = key_start == 0
            || !lower.as_bytes()[key_start - 1].is_ascii_alphanumeric()
                && !matches!(lower.as_bytes()[key_start - 1], b'_' | b'-');
        if !before_is_boundary {
            search_from = key_end;
            continue;
        }

        let mut separator = key_end;
        while separator < line.len() && line.as_bytes()[separator].is_ascii_whitespace() {
            separator += 1;
        }
        if !line
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| matches!(*byte, b':' | b'='))
        {
            search_from = key_end;
            continue;
        }

        let value_start = separator + 1;
        let mut value_end = value_start;
        while value_end < line.len() && line.as_bytes()[value_end].is_ascii_whitespace() {
            value_end += 1;
        }
        if key.contains("authorization") {
            value_end = line.len();
        } else {
            while value_end < line.len()
                && !line.as_bytes()[value_end].is_ascii_whitespace()
                && !matches!(line.as_bytes()[value_end], b',' | b';')
            {
                value_end += 1;
            }
        }

        let mut result = line.to_string();
        result.replace_range(value_start..value_end, " [REDACTED]");
        return result;
    }

    line.to_string()
}

fn skip_escape_sequence(input: &[u8], start: usize) -> usize {
    let Some(next) = input.get(start + 1) else {
        return start + 1;
    };

    match next {
        b'[' => skip_control_sequence(input, start + 2),
        b']' => skip_string_sequence(input, start + 2, true),
        b'P' | b'X' | b'^' | b'_' => skip_string_sequence(input, start + 2, false),
        _ => {
            let mut index = start + 1;
            while input
                .get(index)
                .is_some_and(|byte| (0x20..=0x2f).contains(byte))
            {
                index += 1;
            }
            if input
                .get(index)
                .is_some_and(|byte| (0x30..=0x7e).contains(byte))
            {
                index + 1
            } else {
                index
            }
        }
    }
}

fn skip_control_sequence(input: &[u8], mut index: usize) -> usize {
    while let Some(byte) = input.get(index) {
        index += 1;
        if (0x40..=0x7e).contains(byte) {
            break;
        }
    }
    index
}

fn skip_string_sequence(input: &[u8], mut index: usize, allow_bel_terminator: bool) -> usize {
    while index < input.len() {
        if allow_bel_terminator && input[index] == 0x07 {
            return index + 1;
        }
        if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
            return index + 2;
        }
        index += 1;
    }
    index
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redacts_headers_assignments_and_url_credentials() {
        let output = normalize_output(
            b"Authorization: Bearer top-secret\nTOKEN=abc123 command\nhttps://user:password@example.invalid/repo\n"
                .to_vec(),
        );
        assert!(!output.contains("top-secret"));
        assert!(!output.contains("abc123"));
        assert!(!output.contains("user:password"));
        assert!(output.contains("Authorization: [REDACTED]"));
        assert!(output.contains("TOKEN= [REDACTED]"));
        assert!(output.contains("https://[REDACTED]@example.invalid/repo"));
    }
}
