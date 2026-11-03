use std::{ffi::OsStr, io::Write, os::windows::ffi::OsStrExt, path::PathBuf};
use windows_sys::Win32::{
    Foundation::*, Security::Authorization::*, Security::*, Storage::FileSystem::*,
    System::SystemInformation::*, System::Threading::*,
};

pub fn wide(value: impl AsRef<OsStr>) -> Vec<u16> {
    value.as_ref().encode_wide().chain(Some(0)).collect()
}
pub fn error(operation: &str) -> String {
    format!("{operation}（Windows 错误 {}）", unsafe {
        GetLastError()
    })
}
pub struct Handle(pub HANDLE);
// Windows kernel handles are thread-safe references; lifecycle ownership stays
// with this wrapper, not with the numeric process ID.
unsafe impl Send for Handle {}
unsafe impl Sync for Handle {}
impl Handle {
    pub fn new(handle: HANDLE) -> Result<Self, String> {
        if handle.is_null() || handle == INVALID_HANDLE_VALUE {
            Err(error("无法创建应用代理系统资源"))
        } else {
            Ok(Self(handle))
        }
    }
    pub fn alive(&self) -> bool {
        unsafe { WaitForSingleObject(self.0, 0) == WAIT_TIMEOUT }
    }
    pub fn wait(&self, millis: u32) -> bool {
        unsafe { WaitForSingleObject(self.0, millis) == WAIT_OBJECT_0 }
    }
    pub fn pid(&self) -> u32 {
        unsafe { GetProcessId(self.0) }
    }
    pub fn open(pid: u32) -> Result<Self, String> {
        Self::new(unsafe { OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION | SYNCHRONIZE, 0, pid) })
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        unsafe {
            CloseHandle(self.0);
        }
    }
}
pub struct Descriptor(PSECURITY_DESCRIPTOR);
impl Descriptor {
    pub fn new(text: &str) -> Result<Self, String> {
        let mut pointer = std::ptr::null_mut();
        if unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide(text).as_ptr(),
                1,
                &mut pointer,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(error("无法限制应用代理资源访问权限"));
        }
        Ok(Self(pointer))
    }
    pub fn attributes(&self) -> SECURITY_ATTRIBUTES {
        SECURITY_ATTRIBUTES {
            nLength: std::mem::size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: self.0,
            bInheritHandle: 0,
        }
    }
}
impl Drop for Descriptor {
    fn drop(&mut self) {
        unsafe {
            LocalFree(self.0);
        }
    }
}
pub fn elevated() -> bool {
    let mut token = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) } == 0 {
        return false;
    }
    let Ok(token) = Handle::new(token) else {
        return false;
    };
    let mut info: TOKEN_ELEVATION = unsafe { std::mem::zeroed() };
    let mut size = 0;
    unsafe {
        GetTokenInformation(
            token.0,
            TokenElevation,
            &mut info as *mut _ as _,
            std::mem::size_of_val(&info) as u32,
            &mut size,
        ) != 0
            && info.TokenIsElevated != 0
    }
}
pub fn current_sid() -> Result<String, String> {
    let mut raw = std::ptr::null_mut();
    if unsafe { OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut raw) } == 0 {
        return Err(error("无法读取应用代理用户身份"));
    }
    let token = Handle::new(raw)?;
    let mut size = 0;
    unsafe {
        GetTokenInformation(token.0, TokenUser, std::ptr::null_mut(), 0, &mut size);
    }
    if size == 0 || size > 65536 {
        return Err("应用代理用户身份无效".into());
    }
    let mut buffer = vec![0u64; (size as usize + 7) / 8];
    if unsafe {
        GetTokenInformation(
            token.0,
            TokenUser,
            buffer.as_mut_ptr() as _,
            size,
            &mut size,
        )
    } == 0
    {
        return Err(error("无法读取应用代理用户身份"));
    }
    let info = unsafe { &*(buffer.as_ptr() as *const TOKEN_USER) };
    let mut value = std::ptr::null_mut();
    if unsafe { ConvertSidToStringSidW(info.User.Sid, &mut value) } == 0 {
        return Err(error("无法读取应用代理用户身份"));
    }
    let result = read_wide(value, 256);
    unsafe {
        LocalFree(value as _);
    }
    if !result.starts_with("S-1-")
        || !result
            .bytes()
            .all(|c| c.is_ascii_digit() || c == b'-' || c == b'S')
    {
        return Err("应用代理用户身份无效".into());
    }
    Ok(result)
}
pub fn read_wide(pointer: *const u16, limit: usize) -> String {
    if pointer.is_null() {
        return String::new();
    }
    let mut count = 0;
    unsafe {
        while count < limit && *pointer.add(count) != 0 {
            count += 1;
        }
        String::from_utf16_lossy(std::slice::from_raw_parts(pointer, count))
    }
}
pub fn windows_directory() -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    let size =
        unsafe { GetSystemWindowsDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if size == 0 || size >= buffer.len() {
        return Err(error("无法定位 Windows 系统目录"));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..size])))
}
pub fn system_directory() -> Result<PathBuf, String> {
    let mut buffer = vec![0u16; 32768];
    let size = unsafe { GetSystemDirectoryW(buffer.as_mut_ptr(), buffer.len() as u32) } as usize;
    if size == 0 || size >= buffer.len() {
        return Err(error("无法定位 Windows 系统目录"));
    }
    Ok(PathBuf::from(String::from_utf16_lossy(&buffer[..size])))
}
pub struct PrivateCore {
    pub directory: PathBuf,
    pub binary: PathBuf,
}
impl PrivateCore {
    pub fn prepare(cancel: &std::sync::atomic::AtomicBool) -> Result<Self, String> {
        if !elevated() {
            return Err("应用代理需要 Windows 管理员授权".into());
        }
        let parent = windows_directory()?.join("Temp");
        let attributes = unsafe { GetFileAttributesW(wide(&parent).as_ptr()) };
        if attributes == INVALID_FILE_ATTRIBUTES || attributes & FILE_ATTRIBUTE_REPARSE_POINT != 0 {
            return Err("系统临时目录不符合应用代理安全要求".into());
        }
        let descriptor = Descriptor::new("O:BAG:BAD:P(A;OICI;FA;;;SY)(A;OICI;FA;;;BA)")?;
        let directory = parent.join(format!("NodeCloakApps-{}", uuid::Uuid::new_v4().simple()));
        let attrs = descriptor.attributes();
        if unsafe { CreateDirectoryW(wide(&directory).as_ptr(), &attrs) } == 0 {
            return Err(error("无法创建受保护的应用代理目录"));
        }
        let core = Self {
            binary: directory.join("sing-box.exe"),
            directory,
        };
        let handle = Handle::new(unsafe {
            CreateFileW(
                wide(&core.binary).as_ptr(),
                GENERIC_READ | GENERIC_WRITE,
                0,
                &attrs,
                CREATE_NEW,
                FILE_ATTRIBUTE_NORMAL | FILE_FLAG_OPEN_REPARSE_POINT,
                std::ptr::null_mut(),
            )
        })?;
        use std::os::windows::io::FromRawHandle;
        let handle = std::mem::ManuallyDrop::new(handle);
        let mut file = unsafe { std::fs::File::from_raw_handle(handle.0) };
        let (compressed, expected) = crate::proxy::core::embedded();
        let mut decoder = flate2::read::GzDecoder::new(compressed);
        let mut buffer = [0u8; 65536];
        use sha2::{Digest, Sha256};
        use std::io::Read;
        let mut digest = Sha256::new();
        loop {
            if cancel.load(std::sync::atomic::Ordering::Acquire) {
                return Err("应用代理启动已取消".into());
            }
            let n = decoder
                .read(&mut buffer)
                .map_err(|_| "内置应用代理程序损坏")?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
            file.write_all(&buffer[..n])
                .map_err(|_| "无法准备内置应用代理程序")?;
        }
        file.flush().map_err(|_| "无法准备内置应用代理程序")?;
        drop(file);
        if format!("{:x}", digest.finalize()) != expected.trim() {
            return Err("内置应用代理程序校验失败".into());
        }
        Ok(core)
    }
}
impl Drop for PrivateCore {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.binary);
        let _ = std::fs::remove_dir(&self.directory);
    }
}
