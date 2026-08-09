use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, BufRead, BufReader, BufWriter, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

mod expanded_secret_masking {
    include!("process/secret_masking_impl.rs");
}

pub(crate) fn secure_log_file(path: &Path) -> io::Result<()> {
    expanded_secret_masking::redact_log_file(path)?;
    sanitize_log_file(path)
}

pub(crate) fn sanitize_log_file(path: &Path) -> io::Result<()> {
    let (temporary_path, temporary_file) = create_sanitized_log(path)?;
    let result = sanitize_log_file_to(path, &temporary_path, temporary_file);
    if result.is_err() {
        let _ = fs::remove_file(&temporary_path);
    }
    result
}

fn create_sanitized_log(path: &Path) -> io::Result<(PathBuf, File)> {
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
            ".{name}.sanitized.{}.{}.{}.tmp",
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
        "could not allocate a temporary sanitized log file",
    ))
}

fn sanitize_log_file_to(path: &Path, temporary_path: &Path, temporary_file: File) -> io::Result<()> {
    let source = File::open(path)?;
    let length = source.metadata()?.len();
    let mut reader = BufReader::new(source.take(length));
    let mut writer = BufWriter::new(temporary_file);
    let mut line = Vec::new();

    loop {
        line.clear();
        if reader.read_until(b'\n', &mut line)? == 0 {
            break;
        }
        write_sanitized_bytes(&line, &mut writer)?;
    }

    writer.flush()?;
    let file = writer.into_inner().map_err(|error| error.into_error())?;
    file.sync_all()?;
    drop(file);
    fs::rename(temporary_path, path)
}

fn write_sanitized_bytes<W: Write>(input: &[u8], writer: &mut W) -> io::Result<()> {
    let mut read_index = 0usize;
    let mut copied_from = 0usize;

    while read_index < input.len() {
        let next = match input[read_index] {
            0x1b => skip_escape_sequence(input, read_index),
            b'\n' | b'\t' => {
                read_index += 1;
                continue;
            }
            byte if byte < 0x20 || byte == 0x7f => read_index + 1,
            0xc2 if input
                .get(read_index + 1)
                .is_some_and(|byte| (0x80..=0x9f).contains(byte)) =>
            {
                read_index + 2
            }
            _ => {
                read_index += 1;
                continue;
            }
        };

        writer.write_all(&input[copied_from..read_index])?;
        read_index = next;
        copied_from = read_index;
    }

    writer.write_all(&input[copied_from..])
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
    fn removes_terminal_controls_without_changing_normal_or_invalid_bytes() {
        let path = temporary_log("sanitize-controls");
        fs::write(
            &path,
            b"normal\ttext\x1b[31m red\x1b[0m\n\x1b]0;unsafe title\x07next\0line\ninvalid:\xff\xfe\n",
        )
        .unwrap();

        sanitize_log_file(&path).unwrap();
        let sanitized = fs::read(&path).unwrap();
        let _ = fs::remove_file(&path);

        assert_eq!(
            sanitized,
            b"normal\ttext red\nnextline\ninvalid:\xff\xfe\n"
        );
    }
}
