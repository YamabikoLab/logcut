#[allow(dead_code)]
mod implementation {
    include!("logging_impl.rs");
}

pub(crate) use implementation::{
    finish_limited_file, normalize_output, prepare_log_file, prune_logs, read_log, LimitedWriter,
};

use std::io;
use std::path::Path;

const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;
const LOG_TRUNCATION_NOTICE: &[u8] = b"\n[logcut: command output truncated at 10 MiB]\n";

pub(crate) fn redact_log_file(path: &Path) -> io::Result<()> {
    implementation::redact_log_file_with_limit(path, MAX_LOG_BYTES, LOG_TRUNCATION_NOTICE)
        .map(|_| ())
}
