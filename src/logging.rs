use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

const FAILURE_LOG_TAIL_BYTES: u64 = 1024 * 1024;

extern "C" {
    fn getuid() -> u32;
}

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

fn current_user_id() -> u32 {
    // SAFETY: `getuid` takes no arguments and has no failure mode.
    unsafe { getuid() }
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
    let max_age = Duration::from_secs(max_age_days.saturating_add(1).saturating_mul(86_400));
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

    let mut bytes = Vec::with_capacity((length - start) as usize);
    file.read_to_end(&mut bytes)?;

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
            b'\r' => read_index += 1,
            0x1b if input.get(read_index + 1) == Some(&b'[') => {
                read_index += 2;
                while read_index < input.len() {
                    let byte = input[read_index];
                    read_index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            0x1b if input.get(read_index + 1) == Some(&b']') => {
                read_index += 2;
                while read_index < input.len() {
                    if input[read_index] == 0x07 {
                        read_index += 1;
                        break;
                    }
                    if input[read_index] == 0x1b && input.get(read_index + 1) == Some(&b'\\') {
                        read_index += 2;
                        break;
                    }
                    read_index += 1;
                }
            }
            byte => {
                input[write_index] = byte;
                write_index += 1;
                read_index += 1;
            }
        }
    }

    input.truncate(write_index);
    match String::from_utf8(input) {
        Ok(output) => output,
        Err(error) => String::from_utf8_lossy(error.as_bytes()).into_owned(),
    }
}
