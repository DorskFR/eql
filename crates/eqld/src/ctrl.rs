//! Windows delivers no SIGTERM, and `TerminateProcess` skips a tool's final
//! save. A console break is the only graceful stop, and it can only be raised
//! from inside the target's console — eqld runs as a scheduled task with no
//! console of its own, so it borrows the child's for the length of the call.

use std::io;
use std::time::Duration;
use windows_sys::Win32::System::Console::{
    AttachConsole, FreeConsole, GenerateConsoleCtrlEvent, ATTACH_PARENT_PROCESS, CTRL_BREAK_EVENT,
};

/// A child that has only just started has not created its console yet.
const ATTACH_TRIES: u32 = 20;
const ATTACH_WAIT: Duration = Duration::from_millis(50);
/// Detaching from the console before it has distributed the event drops it.
const DELIVERY_WAIT: Duration = Duration::from_millis(250);

/// Raises `CTRL_BREAK_EVENT` on the process group led by `pid`, which eqld is
/// not in, so it cannot break itself. Blocking.
pub fn break_group(pid: u32) -> io::Result<()> {
    unsafe {
        FreeConsole();
        let raised = match attach(pid) {
            Ok(()) => {
                let sent = GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, pid);
                let outcome = (sent == 0)
                    .then(io::Error::last_os_error)
                    .map_or(Ok(()), Err);
                std::thread::sleep(DELIVERY_WAIT);
                outcome
            }
            Err(err) => Err(err),
        };
        FreeConsole();
        AttachConsole(ATTACH_PARENT_PROCESS);
        raised
    }
}

unsafe fn attach(pid: u32) -> io::Result<()> {
    let mut last = io::Error::other("no attempt was made");
    for _ in 0..ATTACH_TRIES {
        if AttachConsole(pid) != 0 {
            return Ok(());
        }
        last = io::Error::last_os_error();
        std::thread::sleep(ATTACH_WAIT);
    }
    Err(last)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::windows::process::CommandExt;
    use std::sync::OnceLock;
    use std::time::Instant;
    use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;

    const TARGET: &str = "EQLD_CONSOLE_BREAK_MARK";
    const CASE: &str = "ctrl::tests::a_console_break_runs_the_childs_own_handler";
    const HANDLED: i32 = 42;

    static MARK: OnceLock<String> = OnceLock::new();

    unsafe extern "system" fn record(kind: u32) -> i32 {
        if let Some(path) = MARK.get() {
            let _ = std::fs::write(path, kind.to_string());
        }
        std::process::exit(HANDLED);
    }

    /// The child is this same test binary re-run on this one case: nothing
    /// else on a Windows rig is guaranteed to have a console handler at all,
    /// and the whole point is that the tool's handler runs before it dies.
    #[test]
    fn a_console_break_runs_the_childs_own_handler() {
        if let Ok(mark) = std::env::var(TARGET) {
            MARK.set(mark).unwrap();
            unsafe { SetConsoleCtrlHandler(Some(record), 1) };
            std::thread::sleep(Duration::from_secs(60));
            return;
        }

        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        let mark = std::env::temp_dir().join(format!("eqld-break-{}", std::process::id()));
        let _ = std::fs::remove_file(&mark);
        let mut child = std::process::Command::new(std::env::current_exe().unwrap())
            .args([CASE, "--exact"])
            .env(TARGET, &mark)
            .creation_flags(CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
            .expect("the test binary re-runs itself");
        std::thread::sleep(Duration::from_millis(500));

        break_group(child.id()).expect("the event was raised");

        let deadline = Instant::now() + Duration::from_secs(10);
        let mut code = None;
        while Instant::now() < deadline && code.is_none() {
            code = child.try_wait().unwrap().and_then(|status| status.code());
            std::thread::sleep(Duration::from_millis(50));
        }
        if code.is_none() {
            let _ = child.kill();
            panic!("the child never saw the console break");
        }
        assert_eq!(code, Some(HANDLED), "its own handler decided how it ended");
        assert_eq!(
            std::fs::read_to_string(&mark).unwrap(),
            CTRL_BREAK_EVENT.to_string(),
            "and it ran to completion before exiting"
        );
        let _ = std::fs::remove_file(&mark);
    }
}
