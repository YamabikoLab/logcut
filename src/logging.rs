use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs::{self, DirBuilder, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt};
#[cfg(test)]
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FAILURE_LOG_TAIL_BYTES: u64 = 1024 * 1024;
const MAX_REDACTION_LINE_BYTES: usize = 1024 * 1024;
const LONG_LINE_REDACTION: &[u8] = b"[REDACTED: line exceeded safe masking limit]";
const SECONDS_PER_DAY: u64 = 86_400;
const LOG_DIRECTORY_MARKER: &str = ".logcut-directory";
const LOG_DIRECTORY_MARKER_CONTENT: &[u8] = b"logcut log directory\n";
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
            return Err(unsafe_log_directory("log directory must not be a symlink"));
        }
        Ok(_) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            create_log_directory(&settings.log_directory)?;
        }
        Err(error) => return Err(error),
    }

    validate_log_directory(&settings.log_directory)?;
    ensure_log_directory_marker(&settings.log_directory)?;

    prune_logs(
        &settings.log_directory,
        settings.max_log_age_days,
        settings.max_log_files,
    );

    create_unique_log(&settings.log_directory)
}

fn create_log_directory(directory: &Path) -> io::Result<()> {
    if let Some(parent) = directory
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
    {
        fs::create_dir_all(parent)?;
    }

    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    create_log_directory_with(directory, |path| builder.create(path))
}

fn create_log_directory_with<F>(directory: &Path, create: F) -> io::Result<()>
where
    F: FnOnce(&Path) -> io::Result<()>,
{
    match create(directory) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error),
    }
}

fn validate_log_directory(directory: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(directory)?;
    if metadata.file_type().is_symlink() {
        return Err(unsafe_log_directory("log directory must not be a symlink"));
    }
    if !metadata.is_dir() {
        return Err(unsafe_log_directory("log directory must be a directory"));
    }
    if metadata.uid() != current_user_id() {
        return Err(unsafe_log_directory(
            "log directory must be owned by the current user",
        ));
    }
    if metadata.mode() & 0o777 != 0o700 {
        return Err(unsafe_log_directory(
            "log directory permissions must already be 0700",
        ));
    }
    Ok(())
}

fn ensure_log_directory_marker(directory: &Path) -> io::Result<()> {
    let marker = directory.join(LOG_DIRECTORY_MARKER);
    match fs::symlink_metadata(&marker) {
        Ok(_) => validate_log_directory_marker(&marker),
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            if fs::read_dir(directory)?.next().is_some() {
                return Err(unsafe_log_directory(
                    "existing log directory is not marked as logcut-owned",
                ));
            }

            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&marker)?;
            file.write_all(LOG_DIRECTORY_MARKER_CONTENT)?;
            file.sync_all()?;
            validate_log_directory_marker(&marker)
        }
        Err(error) => Err(error),
    }
}

fn validate_log_directory_marker(marker: &Path) -> io::Result<()> {
    let metadata = fs::symlink_metadata(marker)?;
    if metadata.file_type().is_symlink()
        || !metadata.is_file()
        || metadata.uid() != current_user_id()
        || metadata.mode() & 0o777 != 0o600
    {
        return Err(unsafe_log_directory(
            "log directory marker ownership or permissions are unsafe",
        ));
    }

    let content = fs::read(marker)?;
    if content != LOG_DIRECTORY_MARKER_CONTENT {
        return Err(unsafe_log_directory(
            "log directory marker content is invalid",
        ));
    }
    Ok(())
}

fn unsafe_log_directory(message: &'static str) -> io::Error {
    io::Error::new(io::ErrorKind::PermissionDenied, message)
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

pub(crate) fn redact_log_file(path: &Path) -> io::Result<()> {
    let (temporary_path, temporary_file) = create_redacted_log(path)?;
    let result = redact_log_file_to(path, &temporary_path, temporary_file);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_redacted_log(path: &Path) -> io::Result<(PathBuf, File)> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let name = path
        .file_name()
        .and_then(OsStr::to_str)
        .unwrap_or("command.log");
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_nanos();

    for attempt in 0..1000u32 {
        let temporary_path = directory.join(format!(
            ".{name}.redacted.{}.{}.{}.tmp",
            std::process::id(),
            nanos,
            attempt
        ));
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(error) => return Err(error),
        }
    }

    Err(io::Error::new(
        io::ErrorKind::AlreadyExists,
        "could not allocate a temporary redacted log file",
    ))
}

fn redact_log_file_to(path: &Path, temporary_path: &Path, temporary_file: File) -> io::Result<()> {
    let source = File::open(path)?;
    let length = source.metadata()?.len();
    let mut reader = BufReader::new(source.take(length));
    let mut writer = BufWriter::new(temporary_file);
    redact_stream(&mut reader, &mut writer)?;
    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary_path, path)
}

fn redact_stream<R: BufRead, W: Write>(reader: &mut R, writer: &mut W) -> io::Result<()> {
    let mut line = Vec::new();
    let mut discarding_long_line = false;

    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            break;
        }

        let newline = available.iter().position(|byte| *byte == b'\n');
        let consumed = newline.map_or(available.len(), |index| index + 1);
        let segment = &available[..consumed];
        let ends_line = newline.is_some();

        if discarding_long_line {
            if ends_line {
                writer.write_all(b"\n")?;
                discarding_long_line = false;
            }
        } else if line.len().saturating_add(segment.len()) > MAX_REDACTION_LINE_BYTES {
            writer.write_all(LONG_LINE_REDACTION)?;
            line.clear();
            if ends_line {
                writer.write_all(b"\n")?;
            } else {
                discarding_long_line = true;
            }
        } else {
            line.extend_from_slice(segment);
            if ends_line {
                write_redacted_line(&line, writer)?;
                line.clear();
            }
        }

        reader.consume(consumed);
    }

    if !discarding_long_line && !line.is_empty() {
        write_redacted_line(&line, writer)?;
    }

    Ok(())
}

fn write_redacted_line<W: Write>(line: &[u8], writer: &mut W) -> io::Result<()> {
    let text = String::from_utf8_lossy(line);
    let redacted = redact_sensitive_output(&text);
    if redacted == text.as_ref() {
        writer.write_all(line)
    } else {
        writer.write_all(redacted.as_bytes())
    }
}

pub(crate) fn prune_logs(directory: &Path, max_age_days: u64, max_files: usize) {
    if validate_log_directory(directory).is_err()
        || validate_log_directory_marker(&directory.join(LOG_DIRECTORY_MARKER)).is_err()
    {
        return;
    }

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
            .find(|character: char| {
                character.is_whitespace() || matches!(character, '/' | '?' | '#')
            })
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
    let mut result = line.to_string();
    let mut search_from = 0usize;

    loop {
        let lower = result.to_ascii_lowercase();
        let Some(relative) = lower[search_from..].find(key) else {
            break;
        };
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
        if result.as_bytes().get(separator) == Some(&b'"') {
            separator += 1;
        }
        while separator < result.len() && result.as_bytes()[separator].is_ascii_whitespace() {
            separator += 1;
        }
        if !result
            .as_bytes()
            .get(separator)
            .is_some_and(|byte| matches!(*byte, b':' | b'='))
        {
            search_from = key_end;
            continue;
        }

        let value_start = separator + 1;
        let mut content_start = value_start;
        while content_start < result.len() && result.as_bytes()[content_start].is_ascii_whitespace()
        {
            content_start += 1;
        }

        let value_end = if key.contains("authorization") {
            result.len()
        } else if let Some(quote @ (b'\'' | b'"')) = result.as_bytes().get(content_start).copied() {
            let mut index = content_start + 1;
            let mut escaped = false;
            while index < result.len() {
                let byte = result.as_bytes()[index];
                if escaped {
                    escaped = false;
                } else if byte == b'\\' {
                    escaped = true;
                } else if byte == quote {
                    index += 1;
                    break;
                }
                index += 1;
            }
            index
        } else {
            let mut index = content_start;
            while index < result.len()
                && !result.as_bytes()[index].is_ascii_whitespace()
                && !matches!(result.as_bytes()[index], b',' | b';' | b'}' | b']')
            {
                index += 1;
            }
            index
        };

        result.replace_range(value_start..value_end, " [REDACTED]");
        search_from = value_start + " [REDACTED]".len();
    }

    result
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

    fn temporary_log(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("logcut-{name}-{}-{unique}.log", std::process::id()))
    }

    #[test]
    fn directory_creation_race_does_not_change_existing_permissions() {
        let directory = temporary_log("directory-creation-race");

        create_log_directory_with(&directory, |path| {
            fs::create_dir(path)?;
            fs::set_permissions(path, fs::Permissions::from_mode(0o755))?;
            Err(io::Error::from(io::ErrorKind::AlreadyExists))
        })
        .unwrap();

        assert_eq!(
            fs::metadata(&directory).unwrap().permissions().mode() & 0o777,
            0o755
        );
        fs::remove_dir(directory).unwrap();
    }

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

    #[test]
    fn redacts_repeated_sensitive_assignments_on_one_line() {
        let output = normalize_output(
            b"token=first-secret token=second-secret password=third-secret password=fourth-secret\n"
                .to_vec(),
        );
        assert!(!output.contains("first-secret"));
        assert!(!output.contains("second-secret"));
        assert!(!output.contains("third-secret"));
        assert!(!output.contains("fourth-secret"));
        assert_eq!(output.matches("[REDACTED]").count(), 4);
    }

    #[test]
    fn redacts_quoted_and_json_sensitive_values() {
        let output = normalize_output(
            br#"password="alpha beta" token='gamma delta' {"token":"json secret","password": "other secret"}
"#
            .to_vec(),
        );
        for secret in ["alpha beta", "gamma delta", "json secret", "other secret"] {
            assert!(!output.contains(secret));
        }
        assert_eq!(output.matches("[REDACTED]").count(), 4);
    }

    #[test]
    fn redacts_persisted_log_and_preserves_unmatched_bytes() {
        let path = temporary_log("persisted-redaction");
        let original = b"ordinary\x1b[31m output\npassword=top-secret\ninvalid:\xff\xfe\n";
        fs::write(&path, original).unwrap();

        redact_log_file(&path).unwrap();
        let redacted = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert!(redacted.starts_with(b"ordinary\x1b[31m output\n"));
        assert!(redacted.ends_with(b"invalid:\xff\xfe\n"));
        assert!(!redacted
            .windows(b"top-secret".len())
            .any(|value| value == b"top-secret"));
        assert!(redacted
            .windows(b"[REDACTED]".len())
            .any(|value| value == b"[REDACTED]"));
    }

    #[test]
    fn redacts_sensitive_values_on_invalid_utf8_lines() {
        let path = temporary_log("invalid-utf8-redaction");
        fs::write(&path, b"token=hidden\xffvalue\n").unwrap();

        redact_log_file(&path).unwrap();
        let redacted = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert!(!redacted
            .windows(b"hidden".len())
            .any(|value| value == b"hidden"));
        assert!(!redacted
            .windows(b"value".len())
            .any(|value| value == b"value"));
        assert!(redacted
            .windows(b"[REDACTED]".len())
            .any(|value| value == b"[REDACTED]"));
    }

    #[test]
    fn replaces_oversized_single_lines_without_buffering_the_entire_line() {
        let path = temporary_log("oversized-line");
        let mut line = b"token=".to_vec();
        line.resize(MAX_REDACTION_LINE_BYTES + 1, b'x');
        fs::write(&path, line).unwrap();

        redact_log_file(&path).unwrap();
        let redacted = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(redacted, LONG_LINE_REDACTION);
    }
}
