use super::identity;
use crate::app_proxy::{
    ipc::{self, Hello},
    macos_support,
};
use std::{
    io::Read,
    os::{
        fd::{AsRawFd, FromRawFd},
        unix::fs::{DirBuilderExt, MetadataExt, PermissionsExt},
    },
    path::PathBuf,
    process::{Child, Command, Stdio},
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};
pub type Server = tokio::net::UnixStream;
pub struct Handle {
    child: Mutex<Child>,
    shutdown: Mutex<Option<std::os::unix::net::UnixStream>>,
    error: Arc<Mutex<String>>,
    pub pid: u32,
    pub label: String,
}
impl Handle {
    #[cfg(test)]
    pub fn launcher_pid(&self) -> u32 {
        self.child.lock().unwrap().id()
    }
    pub fn alive(&self) -> bool {
        let running = self
            .child
            .lock()
            .is_ok_and(|mut c| matches!(c.try_wait(), Ok(None)));
        if !running {
            self.close_control();
        }
        running
    }
    pub fn wait(&self, millis: u32) -> bool {
        let end = Instant::now() + Duration::from_millis(millis as u64);
        while self.alive() && Instant::now() < end {
            std::thread::sleep(Duration::from_millis(50));
        }
        !self.alive()
    }
    pub fn close_control(&self) {
        if let Ok(mut stream) = self.shutdown.lock() {
            if let Some(stream) = stream.take() {
                let _ = stream.shutdown(std::net::Shutdown::Both);
            }
        }
    }
    fn bind(&self, stream: &Server) -> Result<(), String> {
        let fd = unsafe { libc::dup(stream.as_raw_fd()) };
        if fd < 0 {
            return Err("无法监控应用代理控制通道".into());
        }
        unsafe {
            libc::fcntl(fd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
        *self.shutdown.lock().map_err(|_| "应用代理监控状态不可用")? =
            Some(unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) });
        Ok(())
    }
    fn failure(&self) -> String {
        let error = self.error.lock().map(|s| s.clone()).unwrap_or_default();
        if error.contains("-128") || error.contains("User canceled") {
            "已取消 macOS 管理员授权，应用代理保持关闭".into()
        } else {
            format!(
                "Mac 应用代理授权或辅助进程启动失败：{}",
                error.trim().chars().take(1200).collect::<String>()
            )
        }
    }
}
impl Drop for Handle {
    fn drop(&mut self) {
        self.close_control();
        if let Ok(child) = self.child.get_mut() {
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}
struct Directory(PathBuf);
impl Drop for Directory {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(self.0.join("control.sock"));
        let _ = std::fs::remove_dir(&self.0);
    }
}
pub fn directory(suffix: &str) -> Result<PathBuf, String> {
    let id = uuid::Uuid::parse_str(suffix).map_err(|_| "应用代理通道标识无效")?;
    Ok(PathBuf::from(format!(
        "/private/tmp/nodecloak-ap-{}",
        id.simple()
    )))
}
fn launch(
    suffix: &str,
    parent: u32,
    hash: &str,
    exe: &std::path::Path,
    manifest: &[(String, String)],
) -> Result<Handle, String> {
    let script = macos_support::bootstrap(
        exe.to_str().ok_or("应用代理路径无效")?,
        suffix,
        parent,
        hash,
        cfg!(test),
        manifest,
    )?;
    #[cfg(test)]
    let mut command = {
        let mut c = Command::new("/usr/bin/sudo");
        c.args(["-n", "/bin/sh", "-c", &script]);
        c
    };
    #[cfg(not(test))]
    let mut command = {
        let apple = macos_support::applescript(&script);
        let mut c = Command::new("/usr/bin/osascript");
        c.args(["-e", &apple]);
        c
    };
    let mut child = command
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "无法请求 macOS 管理员授权")?;
    let mut stderr = child.stderr.take().ok_or("无法读取 macOS 授权结果")?;
    let error = Arc::new(Mutex::new(String::new()));
    let copy = error.clone();
    std::thread::spawn(move || {
        let mut bytes = vec![];
        let _ = stderr.by_ref().take(4096).read_to_end(&mut bytes);
        if let Ok(mut e) = copy.lock() {
            *e = String::from_utf8_lossy(&bytes).into_owned();
        }
    });
    Ok(Handle {
        child: Mutex::new(child),
        shutdown: Mutex::new(None),
        error,
        pid: 0,
        label: format!(
            "com.nodecloak.ap.{}",
            uuid::Uuid::parse_str(suffix).unwrap().simple()
        ),
    })
}
#[cfg(test)]
pub async fn connect_helper() -> Result<(Server, Handle), String> {
    connect_helper_cancellable(Arc::new(std::sync::atomic::AtomicBool::new(false))).await
}
pub async fn connect_helper_cancellable(
    cancel: Arc<std::sync::atomic::AtomicBool>,
) -> Result<(Server, Handle), String> {
    let hash = identity::live_hash()?;
    let exe = std::env::current_exe().map_err(|_| "无法定位应用代理程序")?;
    let copy = exe.clone();
    let manifest = crate::app_proxy::runtime::prepare(cancel.clone(), move || {
        crate::app_proxy::macos_loader::inspect(&copy)
    })
    .await?;
    if cancel.load(std::sync::atomic::Ordering::Acquire) {
        return Err("应用代理启动已取消".into());
    }
    let suffix = uuid::Uuid::new_v4().to_string();
    let dir = directory(&suffix)?;
    std::fs::DirBuilder::new()
        .mode(0o700)
        .create(&dir)
        .map_err(|_| "无法创建受限的应用代理通道")?;
    let _directory = Directory(dir.clone());
    let listener = tokio::net::UnixListener::bind(dir.join("control.sock"))
        .map_err(|_| "无法创建应用代理通道")?;
    std::fs::set_permissions(
        dir.join("control.sock"),
        std::fs::Permissions::from_mode(0o600),
    )
    .map_err(|_| "无法保护应用代理通道")?;
    let mut handle = launch(&suffix, std::process::id(), &hash, &exe, &manifest)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(120);
    let mut rejected_peer = None;
    let mut stream = loop {
        if !handle.alive() {
            tokio::time::sleep(Duration::from_millis(50)).await;
            return Err(match rejected_peer {
                Some(error) => format!("{}；{error}", handle.failure()),
                None => handle.failure(),
            });
        }
        if tokio::time::Instant::now() >= deadline {
            return Err("等待 macOS 管理员授权或控制通道超时".into());
        }
        match tokio::time::timeout(Duration::from_millis(100), listener.accept()).await {
            Ok(Ok((stream, _))) => match identity::peer(&stream, &hash, None, Some(0)) {
                Ok(peer) => {
                    handle.pid = peer.pid;
                    break stream;
                }
                Err(error) => {
                    #[cfg(test)]
                    eprintln!("Rejected application proxy root peer: {error}");
                    rejected_peer = Some(error);
                }
            },
            Ok(Err(_)) => return Err("应用代理控制通道连接失败".into()),
            _ => {}
        }
    };
    handle.bind(&stream)?;
    let challenge = Hello {
        protocol: 1,
        token: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
    };
    tokio::time::timeout(Duration::from_secs(5), async {
        ipc::write_frame(&mut stream, &challenge).await?;
        let hello: Hello = ipc::read_frame(&mut stream).await?;
        if !ipc::authenticated(&hello, &challenge.token) {
            return Err("应用代理辅助进程身份不匹配".into());
        }
        Ok::<_, String>(())
    })
    .await
    .map_err(|_| "应用代理身份核验超时")??;
    Ok((stream, handle))
}
pub async fn open_parent(suffix: &str, parent: u32) -> Result<(Server, identity::Peer), String> {
    if unsafe { libc::geteuid() } != 0 {
        return Err("应用代理需要 macOS 管理员授权".into());
    }
    super::security::directory()?;
    let dir = directory(suffix)?;
    let meta = std::fs::symlink_metadata(&dir).map_err(|_| "应用代理主程序已退出")?;
    if !meta.is_dir() || meta.mode() & 0o077 != 0 {
        return Err("应用代理通道目录不符合安全要求".into());
    }
    let mut stream = tokio::net::UnixStream::connect(dir.join("control.sock"))
        .await
        .map_err(|_| "无法连接应用代理主程序")?;
    // Darwin records an accepted socket's effective peer audit identity once
    // the accepting process has used it. Read only the bounded public challenge
    // first; authenticate its sender before echoing or accepting configuration.
    let hello: Hello = tokio::time::timeout(Duration::from_secs(5), ipc::read_frame(&mut stream))
        .await
        .map_err(|_| "应用代理身份核验超时")??;
    let peer = identity::peer(
        &stream,
        &identity::live_hash()?,
        Some(parent),
        Some(meta.uid()),
    )?;
    if !ipc::authenticated(&hello, &hello.token) {
        return Err("应用代理主程序身份核验失败".into());
    }
    ipc::write_frame(&mut stream, &hello).await?;
    Ok((stream, peer))
}
