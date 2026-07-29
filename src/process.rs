use std::ffi::OsString;
use std::fs::OpenOptions;
use std::io::{self, Write};
use std::os::unix::process::{CommandExt, ExitStatusExt};
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::atomic::{AtomicI32, Ordering};
use std::thread;
use std::time::Duration;

const SIGHUP: i32 = 1;
const SIGINT: i32 = 2;
const SIGKILL: i32 = 9;
const SIGTERM: i32 = 15;

static RECEIVED_SIGNAL: AtomicI32 = AtomicI32::new(0);

extern "C" {
    fn kill(pid: i32, signal: i32) -> i32;
    fn setsid() -> i32;
    fn signal(signal: i32, handler: extern "C" fn(i32)) -> usize;
}

extern "C" fn record_signal(signal: i32) {
    RECEIVED_SIGNAL.store(signal, Ordering::SeqCst);
}

fn install_signal_handlers() {
    unsafe {
        signal(SIGHUP, record_signal);
        signal(SIGINT, record_signal);
        signal(SIGTERM, record_signal);
    }
}

pub(crate) fn run_suppressed(arguments: &[OsString], log_path: &Path) -> io::Result<i32> {
    RECEIVED_SIGNAL.store(0, Ordering::SeqCst);
    install_signal_handlers();
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

    let exit_status = loop {
        if forwarded == 0 {
            let received = RECEIVED_SIGNAL.swap(0, Ordering::SeqCst);
            if received != 0 {
                forwarded = received;
                unsafe {
                    kill(-process_group, received);
                }
            }
        }
        if let Some(status) = child.try_wait()? {
            break status;
        }
        thread::sleep(Duration::from_millis(20));
    };

    if forwarded != 0 {
        for _ in 0..50 {
            let alive = unsafe { kill(-process_group, 0) == 0 };
            if !alive {
                break;
            }
            thread::sleep(Duration::from_millis(20));
        }
        if unsafe { kill(-process_group, 0) == 0 } {
            unsafe {
                kill(-process_group, SIGKILL);
            }
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
