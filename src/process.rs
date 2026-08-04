mod secret_masking;

use libc::{c_int, pid_t};
use secret_masking::redact_log_file;
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::mem;
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
) -> RunOutcome {
    let mut cleanup_notes = Vec::new();
    let mut safe_to_wait = false;

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
    let log = OpenOptions::new().append(true).open(log_path)?;
    let stdout = Stdio::from(log.try_clone()?);
    let stderr = Stdio::from(log);

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
        Err(error) => return Err(error),
    };
    let process_group = child.id() as pid_t;
    let mut forwarded = 0;
    let mut forwarded_at = None;
    let mut killed = false;

    let exit_status = loop {
        let child_status = match child.try_wait() {
            Ok(status) => status,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
            Err(error) => {
                return Ok(terminate_after_runtime_failure(
                    &mut child,
                    process_group,
                    log_path,
                    error,
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
        let notice = runtime_failure_notice(&io::Error::other("wait failed"));

        assert!(notice.contains("command may have executed"));
        assert!(!notice.contains("command was not executed"));
    }

    #[test]
    fn runtime_failure_is_not_reported_as_child_exit_code_one() {
        assert_eq!(RunOutcome::RuntimeFailure.exit_code(), RUNTIME_FAILURE_EXIT_CODE);
        assert_ne!(RunOutcome::RuntimeFailure.exit_code(), 1);
    }
}
