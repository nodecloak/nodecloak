use super::security::{self, wide, Handle, PrivateCore};
use std::{
    io::{Read, Write},
    os::windows::io::FromRawHandle,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc,
    },
    time::{Duration, Instant},
};
use windows_sys::Win32::{
    Foundation::*,
    Security::SECURITY_ATTRIBUTES,
    System::{Console::*, JobObjects::*, Pipes::*, Threading::*},
    UI::{Shell::*, WindowsAndMessaging::*},
};

pub fn launch_helper(suffix: &str, parent: u32) -> Result<Handle, String> {
    if !crate::app_proxy::config::valid_id(suffix) || parent == 0 {
        return Err("应用代理启动参数无效".into());
    }
    #[cfg(test)]
    let executable = std::env::var_os("NODECLOAK_TEST_APP_PROXY_HOST")
        .map(std::path::PathBuf::from)
        .or_else(|| std::env::current_exe().ok())
        .ok_or("无法定位应用代理控制程序")?;
    #[cfg(not(test))]
    let executable = std::env::current_exe().map_err(|_| "无法定位应用代理控制程序")?;
    let file = wide(&executable);
    let verb = wide("runas");
    let params = wide(format!("--nodecloak-app-proxy {suffix} {parent}"));
    let mut info: SHELLEXECUTEINFOW = unsafe { std::mem::zeroed() };
    info.cbSize = std::mem::size_of_val(&info) as u32;
    info.fMask = SEE_MASK_NOCLOSEPROCESS | SEE_MASK_NOASYNC;
    info.lpVerb = verb.as_ptr();
    info.lpFile = file.as_ptr();
    info.lpParameters = params.as_ptr();
    info.nShow = SW_HIDE;
    if unsafe { ShellExecuteExW(&mut info) } == 0 {
        return Err(if unsafe { GetLastError() } == ERROR_CANCELLED {
            "已取消 Windows 管理员授权，应用代理保持关闭".into()
        } else {
            security::error("无法请求应用代理管理员授权")
        });
    }
    Handle::new(info.hProcess)
}
fn anonymous_pipe() -> Result<(Handle, Handle), String> {
    let mut read = std::ptr::null_mut();
    let mut write = std::ptr::null_mut();
    let attrs = SECURITY_ATTRIBUTES {
        nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
        lpSecurityDescriptor: std::ptr::null_mut(),
        bInheritHandle: 1,
    };
    if unsafe { CreatePipe(&mut read, &mut write, &attrs, 0) } == 0 {
        return Err(security::error("无法创建应用代理内部通道"));
    }
    Ok((Handle::new(read)?, Handle::new(write)?))
}
pub struct Ownership {
    handle: Handle,
    _same_thread: std::marker::PhantomData<std::rc::Rc<()>>,
}
impl Drop for Ownership {
    fn drop(&mut self) {
        unsafe {
            ReleaseMutex(self.handle.0);
        }
    }
}
pub fn ownership_mutex() -> Result<Ownership, String> {
    let descriptor = security::Descriptor::new("O:BAG:BAD:P(A;;GA;;;SY)(A;;GA;;;BA)")?;
    let attrs = descriptor.attributes();
    let handle = Handle::new(unsafe {
        CreateMutexW(
            &attrs,
            0,
            wide(r"Global\NodeCloakApplicationProxy").as_ptr(),
        )
    })?;
    let result = unsafe { WaitForSingleObject(handle.0, 0) };
    if result != WAIT_OBJECT_0 && result != WAIT_ABANDONED {
        return Err("另一应用代理会话正在运行，请先停止原会话".into());
    }
    Ok(Ownership {
        handle,
        _same_thread: std::marker::PhantomData,
    })
}

pub struct CoreProcess {
    pub process: Arc<Handle>,
    pub job: Arc<Handle>,
    pub pid: u32,
    _files: PrivateCore,
}
impl CoreProcess {
    pub fn start(
        files: PrivateCore,
        mut bytes: Vec<u8>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(Self, std::fs::File), String> {
        let (input, writer) = anonymous_pipe()?;
        let (reader, output) = anonymous_pipe()?;
        if unsafe { SetHandleInformation(writer.0, HANDLE_FLAG_INHERIT, 0) } == 0
            || unsafe { SetHandleInformation(reader.0, HANDLE_FLAG_INHERIT, 0) } == 0
        {
            return Err(security::error("无法隔离应用代理通道"));
        }
        let job = Arc::new(Handle::new(unsafe {
            CreateJobObjectW(std::ptr::null(), std::ptr::null())
        })?);
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.0,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err(security::error("无法隔离应用代理内核"));
        }
        let mut startup: STARTUPINFOW = unsafe { std::mem::zeroed() };
        startup.cb = std::mem::size_of_val(&startup) as u32;
        startup.dwFlags = STARTF_USESTDHANDLES | STARTF_USESHOWWINDOW;
        startup.wShowWindow = SW_HIDE as u16;
        startup.hStdInput = input.0;
        startup.hStdError = output.0;
        startup.hStdOutput = output.0;
        let executable = wide(&files.binary);
        let mut command = wide(format!(
            "\"{}\" run --disable-color -c stdin",
            files.binary.display()
        ));
        let cwd = wide(&files.directory);
        let root = security::windows_directory()?;
        let system = security::system_directory()?;
        // No user-controlled PATH, proxy variables, debugging or certificate-file
        // variables enter the elevated core. It needs only native Windows APIs.
        let mut environment = Vec::<u16>::new();
        for (key, value) in [
            ("PATH", system),
            ("SystemRoot", root.clone()),
            ("TEMP", files.directory.clone()),
            ("TMP", files.directory.clone()),
            ("WINDIR", root),
        ] {
            environment.extend(wide(format!("{key}={}", value.display())));
        }
        environment.push(0);
        let mut info: PROCESS_INFORMATION = unsafe { std::mem::zeroed() };
        if unsafe {
            CreateProcessW(
                executable.as_ptr(),
                command.as_mut_ptr(),
                std::ptr::null(),
                std::ptr::null(),
                1,
                CREATE_SUSPENDED
                    | CREATE_NEW_CONSOLE
                    | CREATE_NEW_PROCESS_GROUP
                    | CREATE_UNICODE_ENVIRONMENT,
                environment.as_ptr() as _,
                cwd.as_ptr(),
                &startup,
                &mut info,
            )
        } == 0
        {
            return Err(security::error("无法启动应用代理内核"));
        }
        let process = Arc::new(Handle::new(info.hProcess)?);
        let thread = Handle::new(info.hThread)?;
        if unsafe { AssignProcessToJobObject(job.0, process.0) } == 0 {
            let error = security::error("无法隔离应用代理内核");
            unsafe {
                TerminateProcess(process.0, 1);
            }
            process.wait(5000);
            return Err(error);
        }
        let core = Self {
            process: process.clone(),
            job: job.clone(),
            pid: info.dwProcessId,
            _files: files,
        };
        // A stalled loader/stdin reader must not pin an elevated child forever.
        let finished = Arc::new(AtomicBool::new(false));
        let watcher = finished.clone();
        std::thread::spawn(move || {
            let deadline = Instant::now() + Duration::from_secs(30);
            while !watcher.load(Ordering::Acquire) && process.alive() {
                if cancel.load(Ordering::Acquire) || Instant::now() >= deadline {
                    unsafe {
                        TerminateJobObject(job.0, 1);
                    }
                    break;
                }
                std::thread::sleep(Duration::from_millis(50));
            }
        });
        if unsafe { ResumeThread(thread.0) } == u32::MAX {
            finished.store(true, Ordering::Release);
            return Err(security::error("无法启动应用代理内核"));
        }
        drop(input);
        drop(output);
        let writer = std::mem::ManuallyDrop::new(writer);
        let mut writer = unsafe { std::fs::File::from_raw_handle(writer.0) };
        let result = writer
            .write_all(&bytes)
            .map_err(|_| "无法传递应用代理内核配置".to_owned());
        use zeroize::Zeroize;
        bytes.zeroize();
        drop(writer);
        finished.store(true, Ordering::Release);
        result?;
        let reader = std::mem::ManuallyDrop::new(reader);
        let reader = unsafe { std::fs::File::from_raw_handle(reader.0) };
        Ok((core, reader))
    }
    pub fn alive(&self) -> bool {
        self.process.alive()
    }
    pub fn exit_code(&self) -> u32 {
        let mut code = 0;
        unsafe {
            GetExitCodeProcess(self.process.0, &mut code);
        }
        code
    }
    pub fn stop(&mut self) -> Result<(), String> {
        if !self.alive() {
            return Ok(());
        }
        unsafe extern "system" fn ignore(_event: u32) -> i32 {
            1
        }
        unsafe {
            // The core has its own hidden console; never attach to a user's
            // terminal or signal another application's console group.
            FreeConsole();
            if AttachConsole(self.pid) != 0 {
                SetConsoleCtrlHandler(Some(ignore), 1);
                // Target the core's process group. Broadcasting to group 0
                // also signals this attached privileged helper, which can exit
                // before removing protection and acknowledging explicit Stop.
                GenerateConsoleCtrlEvent(CTRL_BREAK_EVENT, self.pid);
                FreeConsole();
            }
        }
        if !self.process.wait(5000) {
            unsafe {
                TerminateJobObject(self.job.0, 1);
            }
            if !self.process.wait(5000) {
                return Err("应用代理内核仍在退出，请重试停止".into());
            }
        }
        Ok(())
    }
}
impl Drop for CoreProcess {
    fn drop(&mut self) {
        if self.alive() {
            unsafe {
                TerminateJobObject(self.job.0, 1);
            }
            self.process.wait(5000);
        }
    }
}

pub struct CoreSignal {
    pub ready: AtomicBool,
    pub error: std::sync::Mutex<Option<String>>,
}
pub fn watch_output(file: std::fs::File, secrets: Vec<String>) -> Arc<CoreSignal> {
    let signal = Arc::new(CoreSignal {
        ready: AtomicBool::new(false),
        error: std::sync::Mutex::new(None),
    });
    let status = signal.clone();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(file);
        let mut discarding = false;
        loop {
            let mut bytes = Vec::new();
            let count = reader
                .by_ref()
                .take(16385)
                .read_until(b'\n', &mut bytes)
                .unwrap_or(0);
            if count == 0 {
                break;
            }
            if discarding || bytes.len() > 16384 {
                discarding = !bytes.ends_with(b"\n");
                continue;
            }
            let mut line = String::from_utf8_lossy(&bytes).trim().to_owned();
            if line.contains("sing-box started") {
                status.ready.store(true, Ordering::Release);
            }
            if line.contains("FATAL") || line.to_lowercase().contains("fatal[") {
                for secret in &secrets {
                    if !secret.is_empty() {
                        line = line.replace(secret, "[redacted]");
                    }
                }
                line.truncate(line.floor_char_boundary(2048));
                if let Ok(mut error) = status.error.lock() {
                    *error = Some(line);
                }
            }
        }
    });
    signal
}
