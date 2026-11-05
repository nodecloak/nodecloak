use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    os::{
        fd::AsRawFd,
        unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt},
    },
    path::PathBuf,
    sync::atomic::{AtomicBool, Ordering},
};
pub fn directory() -> Result<PathBuf, String> {
    let executable = std::env::current_exe()
        .map_err(|_| "无法定位应用代理辅助程序")?
        .canonicalize()
        .map_err(|_| "无法读取应用代理辅助程序")?;
    let dir = executable.parent().ok_or("应用代理辅助程序目录无效")?;
    let m = std::fs::symlink_metadata(dir).map_err(|_| "无法检查应用代理目录")?;
    if unsafe { libc::geteuid() } != 0
        || dir.parent() != Some(std::path::Path::new("/private/var/run"))
        || !dir
            .file_name()
            .is_some_and(|s| s.to_string_lossy().starts_with("nodecloak-ap."))
        || m.uid() != 0
        || m.mode() & 0o077 != 0
        || !m.is_dir()
    {
        return Err("应用代理辅助程序没有受保护的目录".into());
    }
    Ok(dir.to_owned())
}
pub struct Ownership(File);
pub fn ownership_mutex() -> Result<Ownership, String> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
        .open("/private/var/run/com.nodecloak.application-proxy.lock")
        .map_err(|_| "无法锁定应用代理会话")?;
    let m = file.metadata().map_err(|_| "无法检查应用代理会话锁")?;
    if m.uid() != 0 || m.mode() & 0o077 != 0 || !m.is_file() || m.nlink() != 1 {
        return Err("应用代理会话锁不符合安全要求".into());
    }
    if unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) } != 0 {
        return Err("另一应用代理会话正在运行，请先停止原会话".into());
    }
    Ok(Ownership(file))
}
impl Drop for Ownership {
    fn drop(&mut self) {
        unsafe {
            libc::flock(self.0.as_raw_fd(), libc::LOCK_UN);
        }
    }
}
pub struct PrivateCore {
    pub directory: PathBuf,
    pub binary: PathBuf,
}
impl PrivateCore {
    pub fn prepare(cancel: &AtomicBool) -> Result<Self, String> {
        let directory = directory()?;
        let binary = directory.join("sing-box");
        let core = Self { directory, binary };
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .custom_flags(libc::O_NOFOLLOW | libc::O_CLOEXEC)
            .open(&core.binary)
            .map_err(|_| "无法创建受保护的应用代理内核")?;
        let (compressed, expected) = crate::proxy::core::embedded();
        let mut input = flate2::read::GzDecoder::new(compressed);
        let mut buffer = [0; 65536];
        use sha2::{Digest, Sha256};
        let mut digest = Sha256::new();
        loop {
            if cancel.load(Ordering::Acquire) {
                return Err("应用代理启动已取消".into());
            }
            let n = input
                .read(&mut buffer)
                .map_err(|_| "内置应用代理内核损坏")?;
            if n == 0 {
                break;
            }
            digest.update(&buffer[..n]);
            file.write_all(&buffer[..n])
                .map_err(|_| "无法准备应用代理内核")?;
        }
        if format!("{:x}", digest.finalize()) != expected.trim() {
            return Err("内置应用代理内核校验失败".into());
        }
        file.sync_all().map_err(|_| "无法保存应用代理内核")?;
        file.set_permissions(std::fs::Permissions::from_mode(0o500))
            .map_err(|_| "无法设置应用代理内核权限")?;
        Ok(core)
    }
}
impl Drop for PrivateCore {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.binary);
    }
}
