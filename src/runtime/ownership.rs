use crate::localization::{self, message};

use std::{fs::File, path::Path, process::Child};

use anyhow::{Context, Result};
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Process {
    pub pid: u32,
    created: u64,
}

impl Process {
    pub fn identify(child: &Child) -> Result<Self> {
        let pid = child.id();
        let created =
            creation_time(pid)?.context(message(localization::PROCESS_GONE_BEFORE_IDENTITY))?;
        Ok(Self { pid, created })
    }

    pub fn active(&self) -> Result<bool> {
        Ok(creation_time(self.pid)? == Some(self.created))
    }

    pub fn created_before(&self, file_time: u64) -> bool {
        self.created != 0 && self.created < file_time
    }
}

pub(super) fn open_native_run_lock(path: &Path) -> std::io::Result<File> {
    #[cfg(windows)]
    {
        use std::os::windows::fs::OpenOptionsExt;
        File::options()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(path)
    }
    #[cfg(not(windows))]
    {
        let _ = path;
        Err(std::io::ErrorKind::Unsupported.into())
    }
}

pub(super) fn windows_boot_time() -> Option<u64> {
    #[cfg(windows)]
    {
        use std::{process::Command, time::Duration};

        const QUERY: &str = r#"
$ErrorActionPreference = 'Stop'
$boot = (Get-CimInstance Win32_OperatingSystem).LastBootUpTime.ToFileTimeUtc()
$event = Get-WinEvent -FilterHashtable @{
    LogName = 'System'; ProviderName = 'Microsoft-Windows-Kernel-General'; Id = 12
} -MaxEvents 1
$stamp = ([xml]$event.ToXml()).Event.EventData.Data |
    Where-Object Name -eq StartTime | Select-Object -ExpandProperty '#text'
$started = [DateTimeOffset]::Parse($stamp, [Globalization.CultureInfo]::InvariantCulture).ToFileTime()
if ([Math]::Abs($boot - $started) -lt [TimeSpan]::TicksPerSecond) {
    [Math]::Min($boot, $started).ToString([Globalization.CultureInfo]::InvariantCulture)
}
"#;
        let (_, output) = crate::process::metadata_output(
            Command::new("pwsh.exe").args([
                "-NoLogo",
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                QUERY,
            ]),
            Duration::from_secs(10),
            128,
        )
        .filter(|(status, _)| status.success())?;
        std::str::from_utf8(&output).ok()?.trim().parse().ok()
    }
    #[cfg(not(windows))]
    None
}

#[cfg(windows)]
fn creation_time(pid: u32) -> Result<Option<u64>> {
    use std::{io, mem::zeroed};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, ERROR_INVALID_PARAMETER, FILETIME, STILL_ACTIVE},
        System::Threading::{
            GetExitCodeProcess, GetProcessTimes, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION,
        },
    };

    // The handle pins the process identity while both liveness and creation time are queried.
    let handle = unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid) };
    if handle.is_null() {
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(ERROR_INVALID_PARAMETER as i32) {
            return Ok(None);
        }
        return Err(error)
            .with_context(|| format!("{} {pid}", message(localization::PROCESS_INSPECT_FAILED)));
    }
    let result = (|| {
        let mut exit = 0;
        if unsafe { GetExitCodeProcess(handle, &mut exit) } == 0 {
            return Err(io::Error::last_os_error());
        }
        if exit != STILL_ACTIVE as u32 {
            return Ok(None);
        }
        let (mut creation, mut ended, mut kernel, mut user): (
            FILETIME,
            FILETIME,
            FILETIME,
            FILETIME,
        ) = unsafe { zeroed() };
        if unsafe { GetProcessTimes(handle, &mut creation, &mut ended, &mut kernel, &mut user) }
            == 0
        {
            return Err(io::Error::last_os_error());
        }
        Ok(Some(
            u64::from(creation.dwHighDateTime) << 32 | u64::from(creation.dwLowDateTime),
        ))
    })();
    unsafe { CloseHandle(handle) };
    Ok(result?)
}

#[cfg(not(windows))]
fn creation_time(_: u32) -> Result<Option<u64>> {
    anyhow::bail!("{}", message(localization::PROCESS_CONTROL_PLATFORM))
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;

    #[test]
    fn stale_creation_time_cannot_match_an_existing_process() {
        let pid = std::process::id();
        let created = creation_time(pid).unwrap().unwrap();
        assert!(Process { pid, created }.active().unwrap());
        assert!(
            !Process {
                pid,
                created: created + 1
            }
            .active()
            .unwrap()
        );
    }
}
