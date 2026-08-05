mod secret_masking;

use libc::{c_int, pid_t};
use secret_masking::redact_log_file;
use std::ffi::OsString;
use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Write};
use std::mem;
use std::os::fd::{AsRawFd, FromRawFd, RawFd};
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const SIGNAL_GRACE_PERIOD: Duration = Duration::from_secs(1);
const FORWARDED_SIGNALS: [c_int; 3] = [libc::SIGHUP, libc::SIGINT, libc::SIGTERM];
const RUNTIME_FAILURE_EXIT_CODE: i32 = 70;
const MAX_LOG_BYTES: usize = 10 * 1024 * 1024;
const POST_EXIT_DRAIN_BYTES: usize = 1024 * 1024;
const LOG_TRUNCATION_NOTICE: &str = "\n[logcut: command output truncated at 10 MiB]\n";
const CAPTURE_OK: u8 = b'0';
const CAPTURE_TRUNCATED: u8 = b'1';
const CAPTURE_FAILED: u8 = b'E';

static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum RunOutcome {
    Exited(i32),
    RuntimeFailure,
}

impl RunOutcome {
    fn exit_code(self) -> i32 {
        match self {
            Self::Exited(status) => status,
            Self::RuntimeFailure => RUNTIME_FAILURE_EXIT_CODE,
        }
    }
}

extern "C" fn record_signal(signal: c_int) {
    RECEIVED_SIGNAL.store(signal, Ordering::SeqCst);
}

struct SignalHandlers {
    previous: Vec<(c_int, libc::sigaction)>,
}

impl SignalHandlers {
    fn install() -> io::Result<Self> {
        let mut previous = Vec::with_capacity(FORWARDED_SIGNALS.len());

        for signal_number in FORWARDED_SIGNALS {
            let mut action: libc::sigaction = unsafe { mem::zeroed() };
            let mut old_action: libc::sigaction = unsafe { mem::zeroed() };
            action.sa_sigaction = record_signal as *const () as usize;
            action.sa_flags = libc::SA_RESTART;

            if unsafe { libc::sigemptyset(&mut action.sa_mask) } == -1
                || unsafe { libc::sigaction(signal_number, &action, &mut old_action) } == -1
            {
                let error = io::Error::last_os_error();
                for (installed_signal, installed_action) in previous.iter().rev() {
                    unsafe {
                        libc::sigaction(*installed_signal, installed_action, std::ptr::null_mut());
                    }
                }
                return Err(error);
            }
            previous.push((signal_number, old_action));
        }

        Ok(Self { previous })
    }
}

impl Drop for SignalHandlers {
    fn drop(&mut self) {
        for (signal_number, action) in self.previous.iter().rev() {
            unsafe {
                libc::sigaction(*signal_number, action, std::ptr::null_mut());
            }
        }
    }
}

fn send_signal_to_group(process_group: pid_t, signal_number: c_int) -> io::Result<()> {
    if unsafe { libc::kill(-process_group, signal_number) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn process_group_is_alive(process_group: pid_t) -> bool {
    if unsafe { libc::kill(-process_group, 0) } == 0 {
        return true;
    }
    io::Error::last_os_error().raw_os_error() == Some(libc::EPERM)
}

fn wait_for_process_group(process_group: pid_t, duration: Duration) {
    let deadline = Instant::now() + duration;
    while process_group_is_alive(process_group) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

fn finish_forwarded_signal(process_group: pid_t, forwarded_at: Instant, already_killed: bool) {
    if !already_killed {
        wait_for_process_group(
            process_group,
            SIGNAL_GRACE_PERIOD.saturating_sub(forwarded_at.elapsed()),
        );
    }

    if process_group_is_alive(process_group) {
        if let Err(error) = send_signal_to_group(process_group, libc::SIGKILL) {
            eprintln!("logcut: failed to terminate process group: {error}");
        }
        wait_for_process_group(process_group, SIGNAL_GRACE_PERIOD);
    }
}

fn set_nonblocking(file: &File) -> io::Result<()> {
    let descriptor = file.as_raw_fd();
    let flags = unsafe { libc::fcntl(descriptor, libc::F_GETFL) };
    if flags == -1 {
        return Err(io::Error::last_os_error());
    }
    if unsafe { libc::fcntl(descriptor, libc::F_SETFL, flags | libc::O_NONBLOCK) } == -1 {
        return Err(io::Error::last_os_error());
    }
    Ok(())
}

fn pipe_files() -> io::Result<(File, File)> {
    let mut descriptors = [0; 2];
    if unsafe { libc::pipe2(descriptors.as_mut_ptr(), libc::O_CLOEXEC) } == -1 {
        return Err(io::Error::last_os_error());
    }

    let reader = unsafe { File::from_raw_fd(descriptors[0]) };
    let writer = unsafe { File::from_raw_fd(descriptors[1]) };
    Ok((reader, writer))
}

fn output_pipe() -> io::Result<(File, File)> {
    let (reader, writer) = pipe_files()?;
    set_nonblocking(&reader)?;
    Ok((reader, writer))
}

struct CaptureController {
    process_id: pid_t,
    control: File,
    status: File,
}

impl CaptureController {
    fn finish(mut self) -> io::Result<bool> {
        let control_error = self.control.write_all(&[1]).err();
        drop(self.control);

        let mut response = Vec::new();
        let result = match self.status.read_to_end(&mut response) {
            Ok(_) if !response.is_empty() => parse_capture_response(&response),
            Ok(_) => Err(control_error.unwrap_or_else(|| {
                io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "output capture process exited without reporting a result",
                )
            })),
            Err(error) => Err(error),
        };
        drop(self.status);

        loop {
            if unsafe { libc::waitpid(self.process_id, std::ptr::null_mut(), 0) } >= 0 {
                break;
            }
            let error = io::Error::last_os_error();
            if error.kind() == io::ErrorKind::Interrupted {
                continue;
            }
            if error.raw_os_error() != Some(libc::ECHILD) {
                return Err(error);
            }
            break;
        }

        result
    }
}

fn parse_capture_response(response: &[u8]) -> io::Result<bool> {
    match response.first().copied() {
        Some(CAPTURE_OK) => Ok(false),
        Some(CAPTURE_TRUNCATED) => Ok(true),
        Some(CAPTURE_FAILED) => Err(io::Error::new(
            io::ErrorKind::Other,
            String::from_utf8_lossy(&response[1..]).into_owned(),
        )),
        _ => Err(io::Error::new(
            io::ErrorKind::InvalidData,
            "output capture process returned an invalid result",
        )),
    }
}

fn close_descriptor_unless_kept(descriptor: RawFd, kept: &[RawFd]) {
    if !kept.contains(&descriptor) {
        unsafe {
            libc::close(descriptor);
        }
    }
}

fn start_capture_process(
    reader: File,
    writer_descriptors: &[RawFd],
    log: File,
) -> io::Result<CaptureController> {
    let (control_reader, control_writer) = pipe_files()?;
    let (status_reader, status_writer) = pipe_files()?;
    set_nonblocking(&control_reader)?;

    let process_id = unsafe { libc::fork() };
    if process_id == -1 {
        return Err(io::Error::last_os_error());
    }

    if process_id == 0 {
        drop(control_writer);
        drop(status_reader);
        for descriptor in writer_descriptors {
            unsafe {
                libc::close(*descriptor);
            }
        }

        let kept = [
            reader.as_raw_fd(),
            control_reader.as_raw_fd(),
            status_writer.as_raw_fd(),
            log.as_raw_fd(),
        ];
        close_descriptor_unless_kept(libc::STDIN_FILENO, &kept);
        close_descriptor_unless_kept(libc::STDOUT_FILENO, &kept);
        close_descriptor_unless_kept(libc::STDERR_FILENO, &kept);

        capture_process(reader, control_reader, status_writer, log);
    }

    drop(reader);
    drop(control_reader);
    drop(status_writer);
    drop(log);

    Ok(CaptureController {
        process_id,
        control: control_writer,
        status: status_reader,
    })
}

fn foreground_exit_requested(control: &mut File) -> io::Result<bool> {
    let mut signal = [0u8; 1];
    match control.read(&mut signal) {
        Ok(0) => Ok(true),
        Ok(_) => Ok(true),
        Err(error) if error.kind() == io::ErrorKind::Interrupted => {
            foreground_exit_requested(control)
        }
        Err(error) if error.kind() == io::ErrorKind::WouldBlock => Ok(false),
        Err(error) => Err(error),
    }
}

fn wait_for_output_or_control(reader: &File, control: &File) -> io::Result<()> {
    let mut descriptors = [
        libc::pollfd {
            fd: reader.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        },
        libc::pollfd {
            fd: control.as_raw_fd(),
            events: libc::POLLIN | libc::POLLHUP,
            revents: 0,
        },
    ];

    loop {
        let result = unsafe { libc::poll(descriptors.as_mut_ptr(), descriptors.len() as _, -1) };
        if result >= 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.kind() != io::ErrorKind::Interrupted {
            return Err(error);
        }
    }
}

fn capture_output(reader: &mut File, control: &mut File, log: &mut File) -> io::Result<bool> {
    let retained_limit = MAX_LOG_BYTES.saturating_sub(LOG_TRUNCATION_NOTICE.len());
    let mut retained = 0usize;
    let mut drained_after_exit = 0usize;
    let mut foreground_exited = false;
    let mut truncated = false;
    let mut buffer = [0u8; 8192];

    loop {
        if !foreground_exited && foreground_exit_requested(control)? {
            foreground_exited = true;
        }
        if foreground_exited && drained_after_exit >= POST_EXIT_DRAIN_BYTES {
            truncated = true;
            break;
        }

        let count = match reader.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                if foreground_exited {
                    break;
                }
                wait_for_output_or_control(reader, control)?;
                continue;
            }
            Err(error) => return Err(error),
        };

        if foreground_exited {
            drained_after_exit = drained_after_exit.saturating_add(count);
        }

        let remaining = retained_limit.saturating_sub(retained);
        let keep = count.min(remaining);
        if keep > 0 {
            log.write_all(&buffer[..keep])?;
            retained += keep;
        }
        if keep < count {
            truncated = true;
        }
    }

    if truncated {
        log.write_all(LOG_TRUNCATION_NOTICE.as_bytes())?;
    }
    log.flush()?;
    Ok(truncated)
}

fn write_capture_response(mut status: File, result: &io::Result<bool>) {
    let response = match result {
        Ok(false) => vec![CAPTURE_OK],
        Ok(true) => vec![CAPTURE_TRUNCATED],
        Err(error) => {
            let mut response = vec![CAPTURE_FAILED];
            response.extend_from_slice(error.to_string().as_bytes());
            response
        }
    };
    let _ = status.write_all(&response);
}

fn capture_process(mut reader: File, mut control: File, status: File, mut log: File) -> ! {
    let result = capture_output(&mut reader, &mut control, &mut log);
    drop(reader);
    drop(log);
    drop(control);
    write_capture_response(status, &result);
    unsafe { libc::_exit(0) }
}

fn finalize_log(log_path: &Path) {
    match redact_log_file(log_path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(error) => {
            let _ = fs::remove_file(log_path);
            if let Ok(mut log) = OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(log_path)
            {
                let _ = writeln!(
                    log,
                    "logcut: command output was discarded because secret masking failed: {error}"
                );
            }
        }
    }
}

fn runtime_failure_notice(error: &io::Error) -> String {
    format!(
        "logcut: command monitoring failed after the command started: {error}; command may have executed, and its final status could not be determined"
    )
}

fn capture_failure_notice(error: &io::Error, status: i32) -> String {
    format!(
        "logcut: command output collection failed after the command started: {error}; command completed with exit status {status}"
    )
}

fn record_capture_failure(log_path: &Path, error: &io::Error, status: i32) {
    let notice = capture_failure_notice(error, status);
    eprintln!("{notice}");
    match OpenOptions::new().append(true).open(log_path) {
        Ok(mut log) => {
            let _ = writeln!(log, "{notice}");
        }
        Err(append_error) => eprintln!(
            "logcut: failed to append output collection failure details to {}: {append_error}",
            log_path.display()
        ),
    }
}

fn finish_capture_result(status: i32, result: io::Result<bool>, log_path: &Path) -> i32 {
    match result {
        Ok(true) => eprintln!("logcut: command output exceeded 10 MiB and was truncated"),
        Ok(false) => {}
        Err(error) => record_capture_failure(log_path, &error, status),
    }
    status
}

fn record_runtime_failure(log_path: &Path, notice: &str, cleanup_notes: &[String]) {
    eprintln!("{notice}");
    for note in cleanup_notes {
        eprintln!("logcut: {note}");
    }
    eprintln!(
        "logcut: command output was preserved for failure handling: {}",
        log_path.display()
    );

    match OpenOptions::new().append(true).open(log_path) {
        Ok(mut log) => {
            let _ = writeln!(log, "{notice}");
            for note in cleanup_notes {
                let _ = writeln!(log, "logcut: {note}");
            }
        }
        Err(error) => eprintln!(
            "logcut: failed to append runtime failure details to {}: {error}",
            log_path.display()
        ),
    }

    finalize_log(log_path);
}

fn terminate_after_runtime_failure(
    child: &mut Child,
    process_group: pid_t,
    log_path: &Path,
    error: io::Error,
    capture_error: Option<io::Error>,
) -> RunOutcome {
    let mut cleanup_notes = Vec::new();
    let mut safe_to_wait = false;

    if let Some(capture_error) = capture_error {
        cleanup_notes.push(format!(
            "command output collection also failed after the command started: {capture_error}"
        ));
    }

    match send_signal_to_group(process_group, libc::SIGTERM) {
        Ok(()) => wait_for_process_group(process_group, SIGNAL_GRACE_PERIOD),
        Err(signal_error) if signal_error.raw_os_error() == Some(libc::ESRCH) => {
            safe_to_wait = true;
        }
        Err(signal_error) => cleanup_notes.push(format!(
            "failed to request process group termination after monitoring error: {signal_error}"
        )),
    }

    if !process_group_is_alive(process_group) {
        safe_to_wait = true;
    } else {
        match send_signal_to_group(process_group, libc::SIGKILL) {
            Ok(()) => {
                safe_to_wait = true;
                wait_for_process_group(process_group, SIGNAL_GRACE_PERIOD);
            }
            Err(signal_error) if signal_error.raw_os_error() == Some(libc::ESRCH) => {
                safe_to_wait = true;
            }
            Err(signal_error) => {
                cleanup_notes.push(format!(
                    "failed to force process group termination after monitoring error: {signal_error}"
                ));
                match child.kill() {
                    Ok(()) => safe_to_wait = true,
                    Err(kill_error) if kill_error.raw_os_error() == Some(libc::ESRCH) => {
                        safe_to_wait = true;
                    }
                    Err(kill_error) => cleanup_notes.push(format!(
                        "failed to terminate the child process after monitoring error: {kill_error}"
                    )),
                }
            }
        }
    }

    if safe_to_wait {
        if let Err(wait_error) = child.wait() {
            if wait_error.raw_os_error() != Some(libc::ECHILD) {
                cleanup_notes.push(format!(
                    "failed to reap the child process after monitoring error: {wait_error}"
                ));
            }
        }
    } else {
        cleanup_notes.push(
            "the child process could not be confirmed as terminated and may still be running"
                .to_string(),
        );
    }

    let notice = runtime_failure_notice(&error);
    record_runtime_failure(log_path, &notice, &cleanup_notes);
    RunOutcome::RuntimeFailure
}

pub(crate) fn run_suppressed(
    arguments: &[OsString],
    log_path: &Path,
    original_umask: libc::mode_t,
) -> io::Result<i32> {
    RECEIVED_SIGNAL.store(0, Ordering::SeqCst);
    let _signal_handlers = SignalHandlers::install()?;
    let (reader, writer) = output_pipe()?;
    let stdout_writer = writer.try_clone()?;
    let writer_descriptors = [writer.as_raw_fd(), stdout_writer.as_raw_fd()];
    let log = OpenOptions::new().append(true).open(log_path)?;
    let capture = start_capture_process(reader, &writer_descriptors, log)?;
    let stdout = Stdio::from(stdout_writer);
    let stderr = Stdio::from(writer);

    let mut command = Command::new(&arguments[0]);
    command
        .args(&arguments[1..])
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .stdin(Stdio::inherit())
        .stdout(stdout)
        .stderr(stderr);

    unsafe {
        command.pre_exec(move || {
            libc::umask(original_umask);
            if libc::setsid() == -1 {
                Err(io::Error::last_os_error())
            } else {
                Ok(())
            }
        });
    }

    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error)
            if matches!(
                error.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            ) =>
        {
            drop(command);
            let capture_result = capture.finish();
            if let Err(capture_error) = capture_result {
                let status = if error.kind() == io::ErrorKind::NotFound {
                    127
                } else {
                    126
                };
                record_capture_failure(log_path, &capture_error, status);
            }
            let mut log = OpenOptions::new().append(true).open(log_path)?;
            writeln!(log, "logcut: failed to execute command: {error}")?;
            drop(log);
            finalize_log(log_path);
            return Ok(if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            });
        }
        Err(error) => {
            drop(command);
            let _ = capture.finish();
            return Err(error);
        }
    };

    // `Command` retains the configured `Stdio` handles after `spawn`. Drop it so the
    // parent closes its copies of the pipe writers.
    drop(command);

    let process_group = child.id() as pid_t;
    let mut forwarded = 0;
    let mut forwarded_at = None;
    let mut killed = false;

    let exit_status = loop {
        let child_status = match child.try_wait() {
            Ok(status) => status,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                let capture_error = capture.finish().err();
                return Ok(terminate_after_runtime_failure(
                    &mut child,
                    process_group,
                    log_path,
                    error,
                    capture_error,
                )
                .exit_code());
            }
        };

        if let Some(status) = child_status {
            break status;
        }

        if forwarded == 0 {
            let received = RECEIVED_SIGNAL.swap(0, Ordering::SeqCst);
            if received != 0 {
                match send_signal_to_group(process_group, received) {
                    Ok(()) => {
                        forwarded = received;
                        forwarded_at = Some(Instant::now());
                    }
                    Err(_) if process_group_is_alive(process_group) => {
                        RECEIVED_SIGNAL.store(received, Ordering::SeqCst);
                    }
                    Err(_) => {}
                }
            }
        }

        if forwarded != 0
            && !killed
            && forwarded_at.is_some_and(|time| time.elapsed() >= SIGNAL_GRACE_PERIOD)
            && process_group_is_alive(process_group)
        {
            if let Err(error) = send_signal_to_group(process_group, libc::SIGKILL) {
                eprintln!("logcut: failed to terminate process group: {error}");
            }
            killed = true;
        }

        thread::sleep(Duration::from_millis(20));
    };

    let capture_result = capture.finish();
    let status = if forwarded != 0 {
        if let Some(time) = forwarded_at {
            finish_forwarded_signal(process_group, time, killed);
        }
        128 + forwarded
    } else {
        exit_status
            .code()
            .unwrap_or_else(|| 128 + exit_status.signal().unwrap_or(1))
    };
    let status = finish_capture_result(status, capture_result, log_path);

    if status != 0 {
        finalize_log(log_path);
    }
    Ok(RunOutcome::Exited(status).exit_code())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn runtime_failure_notice_does_not_claim_the_command_was_not_executed() {
        let notice = runtime_failure_notice(&io::Error::from_raw_os_error(libc::EIO));

        assert!(notice.contains("command may have executed"));
        assert!(!notice.contains("command was not executed"));
    }

    #[test]
    fn runtime_failure_is_not_reported_as_child_exit_code_one() {
        assert_eq!(
            RunOutcome::RuntimeFailure.exit_code(),
            RUNTIME_FAILURE_EXIT_CODE
        );
        assert_ne!(RunOutcome::RuntimeFailure.exit_code(), 1);
    }

    #[test]
    fn capture_failure_preserves_the_known_child_exit_code() {
        let status = finish_capture_result(
            52,
            Err(io::Error::from_raw_os_error(libc::EIO)),
            Path::new("/dev/null"),
        );

        assert_eq!(status, 52);
    }

    #[test]
    fn capture_failure_notice_does_not_claim_the_command_was_not_executed() {
        let notice = capture_failure_notice(&io::Error::from_raw_os_error(libc::EIO), 52);

        assert!(notice.contains("after the command started"));
        assert!(notice.contains("exit status 52"));
        assert!(!notice.contains("command was not executed"));
    }
}