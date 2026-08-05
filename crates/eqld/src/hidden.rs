//! An overlay launched on a desktop of eqld's own: nothing renders that
//! desktop, so its tk window never reaches the screen while its parser ticks on.

use std::io;
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;
use windows_sys::Win32::Foundation::{
    CloseHandle, GENERIC_ALL, HANDLE, STILL_ACTIVE, WAIT_OBJECT_0,
};
use windows_sys::Win32::System::StationsAndDesktops::CreateDesktopW;
use windows_sys::Win32::System::Threading::{
    CreateProcessW, GetExitCodeProcess, TerminateProcess, WaitForSingleObject, CREATE_NO_WINDOW,
    PROCESS_INFORMATION, STARTUPINFOW,
};

const DESKTOP_NAME: &str = "eqld-hidden";

pub struct Process {
    handle: HANDLE,
    pid: u32,
    exited: Option<i32>,
}

/// A process handle is valid process-wide, not per-thread.
unsafe impl Send for Process {}
unsafe impl Sync for Process {}

impl Process {
    pub fn id(&self) -> u32 {
        self.pid
    }

    pub fn try_wait(&mut self) -> io::Result<Option<i32>> {
        if let Some(code) = self.exited {
            return Ok(Some(code));
        }
        // An exit code of STILL_ACTIVE is legal, so the wait decides, not the code.
        if unsafe { WaitForSingleObject(self.handle, 0) } != WAIT_OBJECT_0 {
            return Ok(None);
        }
        let mut code: u32 = STILL_ACTIVE as u32;
        if unsafe { GetExitCodeProcess(self.handle, &mut code) } == 0 {
            return Err(io::Error::last_os_error());
        }
        self.exited = Some(code as i32);
        Ok(self.exited)
    }

    pub async fn stop(&mut self) {
        if matches!(self.try_wait(), Ok(Some(_))) {
            return;
        }
        unsafe { TerminateProcess(self.handle, 1) };
        while matches!(self.try_wait(), Ok(None)) {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
    }
}

impl Drop for Process {
    fn drop(&mut self) {
        unsafe { CloseHandle(self.handle) };
    }
}

struct Desktop {
    name: Vec<u16>,
}

/// Held for the life of the daemon: closing the last handle destroys the
/// desktop and every overlay standing on it.
unsafe impl Send for Desktop {}
unsafe impl Sync for Desktop {}

fn desktop() -> io::Result<&'static Desktop> {
    static DESKTOP: OnceLock<Option<Desktop>> = OnceLock::new();
    DESKTOP
        .get_or_init(|| {
            let name = wide(DESKTOP_NAME);
            let handle = unsafe {
                CreateDesktopW(
                    name.as_ptr(),
                    std::ptr::null(),
                    std::ptr::null(),
                    0,
                    GENERIC_ALL,
                    std::ptr::null(),
                )
            };
            (!handle.is_null()).then_some(Desktop { name })
        })
        .as_ref()
        .ok_or_else(|| io::Error::other("cannot create the hidden desktop"))
}

fn wide(value: &str) -> Vec<u16> {
    std::ffi::OsStr::new(value)
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

fn wide_path(value: &Path) -> Vec<u16> {
    value
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

/// Windows paths cannot contain a quote, so quoting whole is enough.
fn command_line(program: &Path, args: &[PathBuf]) -> Vec<u16> {
    let mut line = format!("\"{}\"", program.display());
    for arg in args {
        line.push_str(&format!(" \"{}\"", arg.display()));
    }
    wide(&line)
}

pub fn spawn(program: &Path, args: &[PathBuf], dir: Option<&Path>) -> io::Result<Process> {
    let desktop = desktop()?;
    let mut line = command_line(program, args);
    let dir = dir.map(wide_path);

    let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
    startup.cb = std::mem::size_of::<STARTUPINFOW>() as u32;
    startup.lpDesktop = desktop.name.as_ptr() as *mut u16;
    let mut information: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };

    let started = unsafe {
        CreateProcessW(
            std::ptr::null(),
            line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            0,
            CREATE_NO_WINDOW,
            std::ptr::null(),
            dir.as_ref().map_or(std::ptr::null(), |dir| dir.as_ptr()),
            &startup,
            &mut information,
        )
    };
    if started == 0 {
        return Err(io::Error::last_os_error());
    }
    unsafe { CloseHandle(information.hThread) };
    Ok(Process {
        handle: information.hProcess,
        pid: information.dwProcessId,
        exited: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_command_line_quotes_the_program_and_every_argument() {
        let line = command_line(
            Path::new(r"C:\Program Files\EQL Log Reader\eql_dps_meter.exe"),
            &[PathBuf::from(r"C:\EQ\Logs\eqlog_Dorsk_erudin.txt")],
        );
        let text = String::from_utf16(&line[..line.len() - 1]).unwrap();
        assert_eq!(
            text,
            "\"C:\\Program Files\\EQL Log Reader\\eql_dps_meter.exe\" \
             \"C:\\EQ\\Logs\\eqlog_Dorsk_erudin.txt\""
        );
        assert_eq!(line.last(), Some(&0), "the command line is nul terminated");
    }

    #[tokio::test]
    async fn a_hidden_process_runs_reports_its_exit_and_can_be_stopped() {
        let cmd = PathBuf::from(std::env::var_os("COMSPEC").unwrap_or("cmd.exe".into()));
        let mut quick = spawn(&cmd, &[PathBuf::from("/c"), PathBuf::from("exit 7")], None)
            .expect("the hidden desktop accepts a process");
        assert!(quick.id() > 0);
        while quick.try_wait().unwrap().is_none() {
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        assert_eq!(quick.try_wait().unwrap(), Some(7));

        let mut long = spawn(
            &cmd,
            &[PathBuf::from("/c"), PathBuf::from("pause")],
            Some(Path::new("C:\\")),
        )
        .unwrap();
        assert_eq!(long.try_wait().unwrap(), None, "it is still up");
        long.stop().await;
        assert!(long.try_wait().unwrap().is_some());
    }
}
