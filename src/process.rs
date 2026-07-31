use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::{Duration, Instant};

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;
const SIG_ERR: usize = usize::MAX;
const SIGNAL_GRACE_PERIOD: Duration = Duration::from_secs(1);

static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn setsid() -> i32;
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn record_signal(signal: i32) {
    RECEIVED_SIGNAL.store(signal, Ordering::SeqCst);
}

fn install_signal_handlers() -> io::Result<()> {
    for signal_number in [SIGHUP, SIGINT, SIGTERM] {
        // SAFETY: `record_signal` has the required C ABI and remains valid for the process lifetime.
        if unsafe { signal(signal_number, record_signal) } == SIG_ERR {
            return Err(io::Error::last_os_error());
        }
    }
    Ok(())
}

fn send_signal_to_group(process_group: i32, signal_number: i32) -> io::Result<()> {
    // SAFETY: A negative PID targets the process group created with `setsid` below.
    if unsafe { kill(-process_group, signal_number) } == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

fn process_group_is_alive(process_group: i32) -> bool {
    // SAFETY: Signal 0 checks whether the process group exists without delivering a signal.
    unsafe { kill(-process_group, 0) == 0 }
}

fn wait_for_process_group(process_group: i32, duration: Duration) {
    let deadline = Instant::now() + duration;
    while process_group_is_alive(process_group) && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(20));
    }
}

fn finish_forwarded_signal(
    process_group: i32,
    forwarded_at: Instant,
    already_killed: bool,
) {
    if !already_killed {
        wait_for_process_group(
            process_group,
            SIGNAL_GRACE_PERIOD.saturating_sub(forwarded_at.elapsed()),
        );
    }

    if process_group_is_alive(process_group) {
        if let Err(error) = send_signal_to_group(process_group, SIGKILL) {
            eprintln!("logcut: failed to terminate process group: {error}");
        }
        wait_for_process_group(process_group, SIGNAL_GRACE_PERIOD);
    }
}

pub(crate) fn run_suppressed(arguments: &[OsString], log_path: &Path) -> io::Result<i32> {
    RECEIVED_SIGNAL.store(0, Ordering::SeqCst);
    install_signal_handlers()?;
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

    // SAFETY: `pre_exec` runs after fork and before exec; the closure only calls async-signal-safe
    // `setsid` and converts its failure into an `io::Error`.
    unsafe {
        command.pre_exec(|| {
            if setsid() == -1 {
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
            return Ok(if error.kind() == io::ErrorKind::NotFound {
                127
            } else {
                126
            });
        }
        Err(error) => return Err(error),
    };
    let process_group = child.id() as i32;
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
            if let Err(error) = send_signal_to_group(process_group, SIGKILL) {
                eprintln!("logcut: failed to terminate process group: {error}");
            }
            killed = true;
        }

        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };

    if forwarded != 0 {
        if let Some(time) = forwarded_at {
            finish_forwarded_signal(process_group, time, killed);
        }
        return Ok(128 + forwarded);
    }

    Ok(exit_status
        .code()
        .unwrap_or_else(|| 128 + exit_status.signal().unwrap_or(1)))
}

pub(crate) fn run_direct(arguments: &[OsString]) -> io::Result<i32> {
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
