//! Native Linux browser integration. All process/window operations are profile scoped.
use std::{
    fs, io::Read,
    os::{fd::AsRawFd, unix::fs::{MetadataExt, OpenOptionsExt, PermissionsExt}},
    path::{Path, PathBuf},
};
use x11rb::{
    connection::Connection,
    protocol::xproto::{AtomEnum, ClientMessageEvent, ConnectionExt, EventMask, Window},
    rust_connection::RustConnection,
    CURRENT_TIME,
};

pub(super) fn browser_path(browser: &str) -> Result<PathBuf, String> {
    let (names, installed): (&[&str], &[&str]) = match browser {
        "chrome" => (&["google-chrome-stable", "google-chrome"], &["/opt/google/chrome/google-chrome"]),
        "edge" => (&["microsoft-edge-stable", "microsoft-edge"], &["/opt/microsoft/msedge/microsoft-edge"]),
        "firefox" => (&["firefox", "firefox-esr"], &["/opt/firefox/firefox", "/usr/lib/firefox/firefox", "/usr/lib/firefox-esr/firefox-esr"]),
        _ => return Err("不支持的浏览器".into()),
    };
    let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).filter(|p| p.is_absolute()).collect())
        .unwrap_or_default();
    roots.extend([super::home_dir().join(".local/bin"), PathBuf::from("/usr/local/bin"), PathBuf::from("/usr/bin")]);
    let paths = roots.iter().flat_map(|root| names.iter().map(move |name| root.join(name)))
        .chain(installed.iter().map(PathBuf::from));
    for path in paths {
        if native_executable(&path) { return Ok(path); }
    }
    Err("未找到可用的原生浏览器。请安装 Google Chrome、Microsoft Edge 或 Mozilla 官方 Firefox 的 deb / tar 版本；Snap 和 Flatpak 版本无法访问专用配置与扩展宿主。".into())
}

fn native_executable(path: &Path) -> bool {
    let Ok(real) = path.canonicalize() else { return false; };
    if ["/snap", "/var/lib/snapd", "/var/lib/flatpak"].iter().any(|root| real.starts_with(root)) {
        return false;
    }
    if !fs::metadata(&real).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0) {
        return false;
    }
    // Ubuntu's transitional /usr/bin/firefox is a shell launcher for Snap.
    let mut head = Vec::new();
    if fs::File::open(real).and_then(|f| f.take(16384).read_to_end(&mut head)).is_err() { return false; }
    !head.starts_with(b"#!") || !String::from_utf8_lossy(&head).lines().any(|line| {
        let line = line.trim();
        !line.starts_with('#') && (line.contains("/snap/bin/") || line.contains("snap run ") || line.contains("flatpak run "))
    })
}

pub(super) fn check_firefox_lock(profile: &Path) -> Result<(), String> {
    let path = profile.join(".parentlock");
    let fcntl_checked = match fs::symlink_metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => false,
        Err(_) => return Err("无法读取 Firefox 专用配置锁".into()),
        Ok(m) if !m.is_file() || m.file_type().is_symlink() => return Err("Firefox 配置锁类型异常，原文件已保留".into()),
        Ok(_) => {
            let file = fs::OpenOptions::new().read(true).write(true)
                .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
                .open(&path).map_err(|_| "无法检查 Firefox 专用配置锁")?;
            // Firefox uses POSIX record locks. flock/fs2 does not detect these.
            let mut lock: libc::flock = unsafe { std::mem::zeroed() };
            lock.l_type = libc::F_WRLCK as _;
            lock.l_whence = libc::SEEK_SET as _;
            if unsafe { libc::fcntl(file.as_raw_fd(), libc::F_GETLK, &mut lock) } == -1 {
                return Err("文件系统不支持检查 Firefox 配置锁，请使用本地配置目录".into());
            }
            if lock.l_type != libc::F_UNLCK as _ {
                return Err("Firefox 专用配置仍被占用，请正常退出该浏览器后重试".into());
            }
            let opened = file.metadata().map_err(|_| "无法检查 Firefox 配置锁")?;
            let current = fs::symlink_metadata(&path).map_err(|_| "Firefox 配置锁在检查期间发生变化")?;
            if !current.is_file() || current.dev() != opened.dev() || current.ino() != opened.ino() {
                return Err("Firefox 配置锁在检查期间发生变化，请重试".into());
            }
            true
        }
    };
    let legacy = profile.join("lock");
    match fs::symlink_metadata(&legacy) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Ok(m) if m.file_type().is_symlink() => {
            let target = fs::read_link(&legacy).map_err(|_| "无法读取 Firefox 链接锁")?;
            let value = target.to_str().ok_or("Firefox 链接锁格式无效")?;
            let (_, pid) = value.rsplit_once(':').ok_or("Firefox 链接锁格式无效")?;
            // '+' marks Mozilla's obsolete compatibility lock. Its actual
            // ownership is the .parentlock record lock checked above.
            if fcntl_checked && pid.strip_prefix('+').is_some_and(|p| p.parse::<u32>().is_ok_and(|p| p > 0)) {
                Ok(())
            } else {
                Err("此 Firefox 副本仍有旧式配置锁，请从原浏览器正常退出后重试；原文件已保留".into())
            }
        }
        _ => Err("无法确认 Firefox 链接锁状态，原文件已保留".into()),
    }
}

fn x11_error(_: impl std::fmt::Display) -> String {
    "无法访问浏览器窗口；请确认 Ubuntu 的 X11 / XWayland 桌面可用，并在该副本的浏览器菜单中操作。".into()
}
fn atom(conn: &RustConnection, name: &[u8]) -> Result<u32, String> {
    Ok(conn.intern_atom(false, name).map_err(x11_error)?.reply().map_err(x11_error)?.atom)
}
fn words(conn: &RustConnection, window: Window, property: u32, kind: u32) -> Vec<u32> {
    conn.get_property(false, window, property, kind, 0, 1024).ok()
        .and_then(|c| c.reply().ok()).and_then(|r| r.value32().map(|v| v.collect())).unwrap_or_default()
}
fn windows(conn: &RustConnection, root: Window, ids: &[u32]) -> Result<Vec<Window>, String> {
    let pid = atom(conn, b"_NET_WM_PID")?;
    let protocols = atom(conn, b"WM_PROTOCOLS")?;
    let delete = atom(conn, b"WM_DELETE_WINDOW")?;
    let mut pending = vec![root];
    let mut matched = Vec::new();
    let mut visited = 0;
    while let Some(window) = pending.pop() {
        visited += 1;
        if visited > 16384 { return Err("桌面窗口数量过多，无法确认专用浏览器窗口".into()); }
        if words(conn, window, pid, AtomEnum::CARDINAL.into()).first().is_some_and(|p| ids.contains(p))
            && words(conn, window, protocols, AtomEnum::ATOM.into()).contains(&delete) {
            matched.push(window);
        } else if let Ok(reply) = conn.query_tree(window).map_err(x11_error)?.reply() {
            pending.extend(reply.children);
        }
    }
    Ok(matched)
}

pub(super) fn request_close(pid: u32) -> Result<bool, String> {
    let (conn, screen) = x11rb::connect(None).map_err(x11_error)?;
    let root = conn.setup().roots[screen].root;
    let protocols = atom(&conn, b"WM_PROTOCOLS")?;
    let delete = atom(&conn, b"WM_DELETE_WINDOW")?;
    let owned = windows(&conn, root, &[pid])?;
    for window in &owned {
        // WM_DELETE_WINDOW invokes the browser's normal shutdown and any save
        // prompts. Never use SIGTERM here: Firefox may lose storage on TERM.
        let event = ClientMessageEvent::new(32, *window, protocols, [delete, CURRENT_TIME, 0, 0, 0]);
        conn.send_event(false, *window, EventMask::NO_EVENT, event).map_err(x11_error)?.check().map_err(x11_error)?;
    }
    conn.flush().map_err(x11_error)?;
    Ok(!owned.is_empty())
}

pub(super) fn focus(ids: &[u32]) -> Result<(), String> {
    let (conn, screen) = x11rb::connect(None).map_err(x11_error)?;
    let root = conn.setup().roots[screen].root;
    let owned = windows(&conn, root, ids)?;
    let window = owned.first().ok_or("浏览器窗口尚未就绪，请稍后重试")?;
    let active = atom(&conn, b"_NET_ACTIVE_WINDOW")?;
    let event = ClientMessageEvent::new(32, *window, active, [2, CURRENT_TIME, 0, 0, 0]);
    conn.send_event(false, root, EventMask::SUBSTRUCTURE_REDIRECT | EventMask::SUBSTRUCTURE_NOTIFY, event)
        .map_err(x11_error)?.check().map_err(x11_error)?;
    conn.flush().map_err(x11_error)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_non_executables_and_snap_forwarders() {
        let root = tempfile::tempdir().unwrap();
        let launcher = root.path().join("firefox");
        fs::write(&launcher, "#!/bin/sh\nexec /snap/bin/firefox \"$@\"\n").unwrap();
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o755)).unwrap();
        assert!(!native_executable(&launcher));
        fs::write(&launcher, "#!/bin/sh\nexec /opt/firefox/firefox \"$@\"\n").unwrap();
        assert!(native_executable(&launcher));
        fs::set_permissions(&launcher, fs::Permissions::from_mode(0o644)).unwrap();
        assert!(!native_executable(&launcher));
    }
    #[test]
    fn firefox_stale_record_file_is_preserved_and_symlinks_are_not_followed() {
        let root = tempfile::tempdir().unwrap();
        let lock = root.path().join(".parentlock");
        fs::write(&lock, b"keep").unwrap();
        std::os::unix::fs::symlink("127.0.1.1:+2147483647", root.path().join("lock")).unwrap();
        check_firefox_lock(root.path()).unwrap();
        assert_eq!(fs::read(&lock).unwrap(), b"keep");
        fs::remove_file(&lock).unwrap();
        let protected = root.path().join("protected");
        fs::write(&protected, b"keep protected").unwrap();
        std::os::unix::fs::symlink(&protected, lock).unwrap();
        assert!(check_firefox_lock(root.path()).is_err());
        assert_eq!(fs::read(protected).unwrap(), b"keep protected");
    }
    #[test]
    fn firefox_record_lock_blocks_while_another_process_owns_it() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join(".parentlock");
        fs::write(&path, b"keep").unwrap();
        // A separate process is essential: POSIX locks are per process.
        let mut child = std::process::Command::new("python3").args(["-c", "import fcntl,sys; f=open(sys.argv[1], 'r+'); fcntl.lockf(f,fcntl.LOCK_EX); print('ready',flush=True); sys.stdin.read()"])
            .arg(&path).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).spawn().unwrap();
        use std::io::BufRead;
        let mut line = String::new();
        std::io::BufReader::new(child.stdout.take().unwrap()).read_line(&mut line).unwrap();
        assert_eq!(line.trim(), "ready");
        let result = check_firefox_lock(root.path());
        drop(child.stdin.take());
        child.wait().unwrap();
        assert!(result.is_err());
        check_firefox_lock(root.path()).unwrap();
        assert_eq!(fs::read(path).unwrap(), b"keep");
    }
}
