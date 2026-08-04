mod secret_masking;

use secret_masking::redact_log_file;
use libc::{c_int, pid_t};
use std::ffi::OsString;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::mem;
use std::os::unix::fs::OpenOptionsExt;
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const SIGNAL_GRACE_PERIOD: Duration = Duration::from_secs(1);
const FORWARDED_SIGNALS: [c_int; 3] = [libc::SIGHUP, libc::SIGINT, libc::SIGTERM];

static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

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
        if forwarded == 0 {
            let received = RECEIVED_SIGNAL.swap(0, Ordering::SeqCst);
            if received != 0 {
                match send_signal_to_group(process_group, received) {
                    Ok(()) => {
                        forwarded = received;
                        forwarded_at = Some(Instant::now());
                    }
                    Err(_) if child.try_wait()?.is_none() => {
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

        if let Some(status) = child.try_wait()? {
            break status;
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
    Ok(status)
}

pub(crate) fn run_direct(arguments: &[OsString], original_umask: libc::mode_t) -> io::Result<i32> {
    unsafe {
        libc::umask(original_umask);
    }

    let error = Command::new(&arguments[0])
        .args(&arguments[1..])
        .env("NO_COLOR", "1")
        .env("FORCE_COLOR", "0")
        .exec();
    eprintln!("logcut: failed to execute command: {error}");
    Ok(if error.kind() == io::ErrorKind::NotFound {
        127
    } else {
        126
    })
}
