use std::{
    io,
    sync::atomic::{AtomicBool, Ordering},
};

static REQUESTED: AtomicBool = AtomicBool::new(false);

pub(super) struct Cancellation {
    #[cfg(unix)]
    previous: libc::sigaction,
}

impl Cancellation {
    pub(super) fn listen() -> io::Result<Self> {
        REQUESTED.store(false, Ordering::Relaxed);
        #[cfg(windows)]
        {
            use windows_sys::Win32::System::Console::SetConsoleCtrlHandler;
            if unsafe { SetConsoleCtrlHandler(Some(control), 1) } == 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self {})
        }
        #[cfg(unix)]
        {
            let mut action: libc::sigaction = unsafe { std::mem::zeroed() };
            let mut previous: libc::sigaction = unsafe { std::mem::zeroed() };
            action.sa_sigaction = interrupt as *const () as usize;
            // The installed handler only updates a lock-free flag.
            unsafe { libc::sigemptyset(&mut action.sa_mask) };
            if unsafe { libc::sigaction(libc::SIGINT, &action, &mut previous) } != 0 {
                return Err(io::Error::last_os_error());
            }
            Ok(Self { previous })
        }
        #[cfg(not(any(windows, unix)))]
        Ok(Self {})
    }

    pub(super) fn flag(&self) -> &AtomicBool {
        &REQUESTED
    }
}

#[cfg(windows)]
unsafe extern "system" fn control(event: u32) -> i32 {
    use windows_sys::Win32::System::Console::{CTRL_BREAK_EVENT, CTRL_C_EVENT};
    if matches!(event, CTRL_C_EVENT | CTRL_BREAK_EVENT) {
        REQUESTED.store(true, Ordering::Relaxed);
        1
    } else {
        0
    }
}

#[cfg(unix)]
extern "C" fn interrupt(_: libc::c_int) {
    REQUESTED.store(true, Ordering::Relaxed);
}

impl Drop for Cancellation {
    fn drop(&mut self) {
        #[cfg(windows)]
        unsafe {
            windows_sys::Win32::System::Console::SetConsoleCtrlHandler(Some(control), 0);
        }
        #[cfg(unix)]
        unsafe {
            libc::sigaction(libc::SIGINT, &self.previous, std::ptr::null_mut());
        }
    }
}
