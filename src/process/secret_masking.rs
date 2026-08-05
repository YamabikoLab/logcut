#[allow(dead_code)]
mod implementation {
    include!("secret_masking_impl.rs");

    const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;
    const LOG_TRUNCATION_NOTICE: &[u8] = b"\n[logcut: command output truncated at 10 MiB]\n";

    pub(super) fn redact_log_file_limited(path: &Path) -> io::Result<()> {
        crate::logging::redact_log_file(path)?;

        let (temporary_path, temporary_file) = create_temporary_file(path)?;
        let result = redact_file_to_limited(path, &temporary_path, temporary_file);
        if result.is_err() {
            let _ = fs::remove_file(&temporary_path);
        }
        result
    }

    fn redact_file_to_limited(
        path: &Path,
        temporary_path: &Path,
        temporary_file: File,
    ) -> io::Result<()> {
        let source = File::open(path)?;
        let length = source.metadata()?.len();
        let mut reader = BufReader::new(source.take(length));
        let mut writer =
            crate::logging::LimitedWriter::new(BufWriter::new(temporary_file), MAX_LOG_BYTES);
        let mut line = Vec::new();

        loop {
            line.clear();
            if reader.read_until(b'\n', &mut line)? == 0 {
                break;
            }
            write_redacted_line(&line, &mut writer)?;
        }

        let (file, _) =
            crate::logging::finish_limited_file(writer, MAX_LOG_BYTES, LOG_TRUNCATION_NOTICE)?;
        drop(file);
        fs::rename(temporary_path, path)
    }
}

pub(super) use implementation::redact_log_file_limited as redact_log_file;
