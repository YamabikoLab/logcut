use std::cmp::Reverse;
use std::ffi::OsStr;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read};
use std::os::unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

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
    if !metadata.is_dir()
        || metadata.uid() != unsafe { getuid() }
        || metadata.mode() & 0o777 != 0o700
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
    let mut bytes = Vec::new();
    file.read_to_end(&mut bytes)?;
    Ok(bytes)
}

pub(crate) fn normalize_output(input: &[u8]) -> String {
    let mut output = Vec::with_capacity(input.len());
    let mut index = 0;
    while index < input.len() {
        match input[index] {
            b'\r' => index += 1,
            0x1b if input.get(index + 1) == Some(&b'[') => {
                index += 2;
                while index < input.len() {
                    let byte = input[index];
                    index += 1;
                    if (0x40..=0x7e).contains(&byte) {
                        break;
                    }
                }
            }
            0x1b if input.get(index + 1) == Some(&b']') => {
                index += 2;
                while index < input.len() {
                    if input[index] == 0x07 {
                        index += 1;
                        break;
                    }
                    if input[index] == 0x1b && input.get(index + 1) == Some(&b'\\') {
                        index += 2;
                        break;
                    }
                    index += 1;
                }
            }
            byte => {
                output.push(byte);
                index += 1;
            }
        }
    }
    String::from_utf8_lossy(&output).into_owned()
}
