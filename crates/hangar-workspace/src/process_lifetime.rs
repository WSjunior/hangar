//! O sistema operacional encerra o grupo mesmo quando o executor morre sem rodar destrutores.
use std::process::{Child, Command};

#[cfg(unix)]
pub struct Guard {
    monitor: Child,
    armed: bool,
}

#[cfg(unix)]
impl Guard {
    pub fn new() -> std::io::Result<Self> {
        use std::os::unix::process::CommandExt;
        use std::process::Stdio;
        // Só há texto fixo no monitor: argumentos e conteúdo do usuário continuam fora do shell.
        let monitor = Command::new("/bin/sh")
            .args(["-c", "IFS= read -r _hangar_done || kill -KILL 0"])
            .process_group(0)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()?;
        Ok(Self {
            monitor,
            armed: true,
        })
    }
    pub fn configure(&self, command: &mut Command) {
        use std::os::unix::process::CommandExt;
        command.process_group(self.monitor.id() as i32);
    }
    pub fn attach(&self, _: &Child) -> std::io::Result<()> {
        Ok(())
    }
    pub fn kill(&self) {
        unsafe {
            libc::kill(-(self.monitor.id() as i32), libc::SIGKILL);
        }
    }
    /// Comando terminou: o que ele deixou em segundo plano (hook) segue vivo, como no Python.
    pub fn disarm(mut self) {
        use std::io::Write;
        if let Some(mut stdin) = self.monitor.stdin.take() {
            self.armed = stdin.write_all(b"\n").is_err();
        }
    }
}
#[cfg(unix)]
impl Drop for Guard {
    fn drop(&mut self) {
        if self.armed {
            self.kill();
        }
        drop(self.monitor.stdin.take());
        let _ = self.monitor.wait();
    }
}

#[cfg(windows)]
pub struct Guard {
    job: windows_sys::Win32::Foundation::HANDLE,
    armed: bool,
}

#[cfg(windows)]
impl Guard {
    pub fn new() -> std::io::Result<Self> {
        use windows_sys::Win32::{Foundation::CloseHandle, System::JobObjects::*};
        unsafe {
            let job = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if job.is_null() {
                return Err(std::io::Error::last_os_error());
            }
            let mut info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            info.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&info) as u32,
            ) == 0
            {
                let error = std::io::Error::last_os_error();
                CloseHandle(job);
                return Err(error);
            }
            Ok(Self { job, armed: true })
        }
    }
    pub fn configure(&self, command: &mut Command) {
        use std::os::windows::process::CommandExt;
        // Suspenso até entrar no Job: nenhum descendente nasce fora da proteção.
        command.creation_flags(0x08000004);
    }
    pub fn attach(&self, child: &Child) -> std::io::Result<()> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::{
            Foundation::{CloseHandle, INVALID_HANDLE_VALUE},
            System::{
                Diagnostics::ToolHelp::*,
                JobObjects::AssignProcessToJobObject,
                Threading::{OpenThread, ResumeThread, THREAD_SUSPEND_RESUME},
            },
        };
        unsafe {
            if AssignProcessToJobObject(self.job, child.as_raw_handle() as _) == 0 {
                return Err(std::io::Error::last_os_error());
            }
            let snapshot = CreateToolhelp32Snapshot(TH32CS_SNAPTHREAD, 0);
            if snapshot == INVALID_HANDLE_VALUE {
                return Err(std::io::Error::last_os_error());
            }
            let mut entry: THREADENTRY32 = std::mem::zeroed();
            entry.dwSize = std::mem::size_of_val(&entry) as u32;
            let mut valid = Thread32First(snapshot, &mut entry);
            let mut resumed = false;
            while valid != 0 {
                if entry.th32OwnerProcessID == child.id() {
                    let thread = OpenThread(THREAD_SUSPEND_RESUME, 0, entry.th32ThreadID);
                    if !thread.is_null() {
                        resumed |= ResumeThread(thread) != u32::MAX;
                        CloseHandle(thread);
                    }
                }
                valid = Thread32Next(snapshot, &mut entry);
            }
            CloseHandle(snapshot);
            if !resumed {
                return Err(std::io::Error::other("command thread not resumed"));
            }
            Ok(())
        }
    }
    pub fn kill(&self) {
        unsafe {
            windows_sys::Win32::System::JobObjects::TerminateJobObject(self.job, 1);
        }
    }
    /// Comando terminou: o que ele deixou em segundo plano (hook) segue vivo, como no Python.
    pub fn disarm(mut self) {
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let info: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            self.armed = SetInformationJobObject(
                self.job,
                JobObjectExtendedLimitInformation,
                &info as *const _ as *const std::ffi::c_void,
                std::mem::size_of_val(&info) as u32,
            ) == 0;
        }
    }
}
#[cfg(windows)]
impl Drop for Guard {
    fn drop(&mut self) {
        if self.armed {
            self.kill();
        }
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.job);
        }
    }
}
