//! Inspect browser singleton ownership; never remove browser-owned lock data.
use std::path::Path;

fn failure(error: std::io::Error) -> String {
    format!(
        "无法确认浏览器副本锁状态（kind={:?}, os_code={:?}）；本机数据已保留",
        error.kind(),
        error.raw_os_error()
    )
}
fn finish_probe(file: std::fs::File, result: Result<(), String>) -> Result<(), String> {
    // A concurrent spawn can inherit another reference to this open file description
    // until exec. Drop alone need not release flock before browser startup.
    fs2::FileExt::unlock(&file).map_err(failure)?;
    result
}
fn idle_regular_file(path: &Path) -> Result<(), String> {
    let mut options = std::fs::OpenOptions::new();
    options.read(true).write(true);
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK);
    }
    let file = options.open(path).map_err(failure)?;
    let opened = file.metadata().map_err(failure)?;
    if !opened.is_file() {
        return Err("浏览器副本锁类型异常；原文件已保留".into());
    }
    fs2::FileExt::try_lock_exclusive(&file).map_err(|error| {
        if error.kind() == std::io::ErrorKind::WouldBlock
            || error.raw_os_error() == fs2::lock_contended_error().raw_os_error()
        {
            "此副本仍被浏览器占用，请先从该浏览器菜单退出后重试".into()
        } else {
            failure(error)
        }
    })?;
    let result = (|| {
        let current = std::fs::symlink_metadata(path).map_err(failure)?;
        if !current.is_file() || current.file_type().is_symlink() {
            return Err("浏览器副本锁在检查期间发生变化，请重试".into());
        }
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            use std::os::unix::fs::MetadataExt;
            if opened.dev() != current.dev() || opened.ino() != current.ino() {
                return Err("浏览器副本锁在检查期间发生变化，请重试".into());
            }
        }
        Ok(())
    })();
    // Release even when validation fails; never unlink or rewrite browser data.
    finish_probe(file, result)
}
fn stale_link(
    target: &Path,
    hostname: &str,
    alive: impl FnOnce(i32) -> Result<bool, String>,
) -> Result<(), String> {
    let value = target
        .to_str()
        .ok_or("浏览器副本锁格式无效；原文件已保留")?;
    if value.contains(['/', '\\']) {
        return Err("浏览器副本锁格式无效；原文件已保留".into());
    }
    let (host, pid) = value
        .rsplit_once('-')
        .ok_or("浏览器副本锁格式无效；原文件已保留")?;
    let pid = pid
        .parse::<i32>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or("浏览器副本锁进程无效；原文件已保留")?;
    if host.is_empty() || !host.eq_ignore_ascii_case(hostname) {
        return Err("此副本的锁来自其他主机；请确认原设备已退出浏览器，原文件已保留".into());
    }
    if alive(pid)? {
        return Err("此副本的锁进程仍在运行，请先正常退出该浏览器；原文件已保留".into());
    }
    Ok(())
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn hostname() -> Result<String, String> {
    let mut bytes = [0u8; 256];
    if unsafe { libc::gethostname(bytes.as_mut_ptr().cast(), bytes.len()) } != 0 {
        return Err(failure(std::io::Error::last_os_error()));
    }
    let end = bytes
        .iter()
        .position(|b| *b == 0)
        .ok_or("无法确认副本锁的主机信息")?;
    std::str::from_utf8(&bytes[..end])
        .ok()
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .ok_or("无法确认副本锁的主机信息".into())
}
#[cfg(any(target_os = "macos", target_os = "linux"))]
fn process_alive(pid: i32) -> Result<bool, String> {
    if unsafe { libc::kill(pid, 0) } == 0 {
        return Ok(true);
    }
    let error = std::io::Error::last_os_error();
    if error.raw_os_error() == Some(libc::ESRCH) {
        Ok(false)
    } else {
        Err(failure(error))
    }
}
pub(super) fn check(profile: &Path) -> Result<(), String> {
    let path = profile.join("SingletonLock");
    let metadata = match std::fs::symlink_metadata(&path) {
        Ok(m) => m,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(e) => return Err(failure(e)),
    };
    let result = if metadata.file_type().is_symlink() {
        #[cfg(any(target_os = "macos", target_os = "linux"))]
        {
            let verify = || -> Result<(), String> {
                use std::os::unix::fs::MetadataExt;
                let target = std::fs::read_link(&path).map_err(failure)?;
                stale_link(&target, &hostname()?, process_alive)?;
                let current = std::fs::symlink_metadata(&path).map_err(failure)?;
                if !current.file_type().is_symlink()
                    || current.dev() != metadata.dev()
                    || current.ino() != metadata.ino()
                    || std::fs::read_link(&path).map_err(failure)? != target
                {
                    return Err("浏览器副本锁在检查期间发生变化，请重试".into());
                }
                Ok(())
            };
            verify()
        }
        #[cfg(not(any(target_os = "macos", target_os = "linux")))]
        {
            Err("此平台不支持检查浏览器链接锁".into())
        }
    } else if metadata.is_file() {
        idle_regular_file(&path)
    } else {
        Err("浏览器副本锁类型异常；原文件已保留".into())
    };
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let browser = profile
            .file_name()
            .and_then(|s| s.to_str())
            .and_then(|s| s.strip_prefix("browser-"))
            .filter(|s| ["chrome", "edge"].contains(s))
            .unwrap_or("chromium");
        crate::browser_control::log_startup_endpoint(
            profile,
            browser,
            if result.is_ok() {
                "profile_lock_idle"
            } else {
                "profile_lock_blocked"
            },
            result
                .as_ref()
                .err()
                .map(String::as_str)
                .unwrap_or("browser-owned lock is idle; preserved for browser-managed cleanup"),
        );
    }
    result
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    struct Site {
        url: String,
        reports: std::sync::mpsc::Receiver<bool>,
        stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
        worker: Option<std::thread::JoinHandle<()>>,
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    impl Site {
        fn new() -> Self {
            use std::io::{Read, Write};
            use std::sync::atomic::Ordering;
            let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
            listener.set_nonblocking(true).unwrap();
            let url = format!("http://{}/", listener.local_addr().unwrap());
            let (sender, reports) = std::sync::mpsc::channel();
            let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
            let ended = stop.clone();
            let worker = std::thread::spawn(move || {
                while !ended.load(Ordering::SeqCst) {
                    let (mut stream, _) = match listener.accept() {
                        Ok(s) => s,
                        Err(_) => {
                            std::thread::sleep(std::time::Duration::from_millis(20));
                            continue;
                        }
                    };
                    stream.set_nonblocking(false).unwrap();
                    stream
                        .set_read_timeout(Some(std::time::Duration::from_millis(300)))
                        .unwrap();
                    stream
                        .set_write_timeout(Some(std::time::Duration::from_millis(300)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut buf = [0; 1024];
                    while request.len() < 8192 && !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        match stream.read(&mut buf) {
                            Ok(0) | Err(_) => break,
                            Ok(n) => request.extend_from_slice(&buf[..n]),
                        }
                    }
                    if !request.windows(4).any(|w| w == b"\r\n\r\n") {
                        continue;
                    }
                    let text = String::from_utf8_lossy(&request);
                    let mut fields = text.lines().next().unwrap_or("").split_whitespace();
                    if fields.next() != Some("GET") {
                        continue;
                    }
                    let path = fields.next().unwrap_or("");
                    let body = if path == "/" {
                        r#"<script>const kept=document.cookie.includes('nodecloak-lock-fixture=kept');document.cookie='nodecloak-lock-fixture=kept;path=/;Max-Age=3600;SameSite=Lax';fetch('/ready?kept='+kept)</script>"#
                    } else {
                        if let Some(value) = path.strip_prefix("/ready?kept=") {
                            if value == "true" || value == "false" {
                                let _ = sender.send(value == "true");
                            }
                        }
                        ""
                    };
                    let _=write!(stream,"HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-store\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len());
                }
            });
            Self {
                url,
                reports,
                stop,
                worker: Some(worker),
            }
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    impl Drop for Site {
        fn drop(&mut self) {
            self.stop.store(true, std::sync::atomic::Ordering::SeqCst);
            if let Some(worker) = self.worker.take() {
                worker.join().unwrap();
            }
        }
    }
    #[test]
    fn idle_singleton_file_survives_repeated_closed_checks_unchanged() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("SingletonLock");
        std::fs::write(&path, b"browser-owned lock metadata").unwrap();
        for _ in 0..3 {
            assert!(
                check(root.path()).is_ok(),
                "an idle persistent lock is not an occupied profile"
            );
        }
        assert_eq!(
            std::fs::read(&path).unwrap(),
            b"browser-owned lock metadata"
        );
    }
    #[test]
    fn active_native_lock_blocks_until_released_without_deleting_data() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let path = root.join("SingletonLock");
        std::fs::write(&path, b"keep").unwrap();
        let owner = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(&path)
            .unwrap();
        fs2::FileExt::lock_exclusive(&owner).unwrap();
        assert!(check(&root).is_err());
        fs2::FileExt::unlock(&owner).unwrap();
        drop(owner);
        let result = check(&root);
        assert!(result.is_ok(), "after releasing owner: {result:?}");
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn explicit_probe_release_frees_shared_description_on_success_and_error() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let path = root.join("SingletonLock");
        std::fs::write(&path, b"keep").unwrap();
        for validation in [Ok(()), Err("identity changed".to_owned())] {
            let probe = std::fs::OpenOptions::new()
                .read(true)
                .write(true)
                .open(&path)
                .unwrap();
            fs2::FileExt::lock_exclusive(&probe).unwrap();
            let inherited = probe.try_clone().unwrap();
            assert!(check(&root).is_err());
            assert_eq!(finish_probe(probe, validation.clone()), validation);
            // Keep the duplicate alive: implicit close alone cannot free this lock.
            let result = check(&root);
            assert!(
                result.is_ok(),
                "shared-description probe release: {result:?}"
            );
            drop(inherited);
        }
        assert_eq!(std::fs::read(&path).unwrap(), b"keep");
    }
    #[test]
    fn links_require_same_host_and_proven_dead_pid() {
        assert!(
            stale_link(Path::new("my-mac.local-123"), "my-mac.local", |pid| {
                assert_eq!(pid, 123);
                Ok(false)
            })
            .is_ok()
        );
        assert!(stale_link(Path::new("my-mac.local-123"), "my-mac.local", |_| Ok(true)).is_err());
        assert!(
            stale_link(Path::new("foreign-123"), "my-mac.local", |_| panic!(
                "must not probe foreign PID"
            ))
            .is_err()
        );
        assert!(
            stale_link(Path::new("my-mac.local-123"), "my-mac.local", |_| Err(
                "permission unknown".into()
            ))
            .is_err()
        );
        for value in [
            "invalid",
            "my-mac.local-0",
            "my-mac.local--1",
            "/tmp/my-mac.local-123",
            "../my-mac.local-123",
        ] {
            assert!(stale_link(Path::new(value), "my-mac.local", |_| Ok(false)).is_err());
        }
    }
    #[test]
    fn unusual_lock_types_are_not_treated_as_unlocked() {
        let root = tempfile::tempdir().unwrap();
        std::fs::create_dir(root.path().join("SingletonLock")).unwrap();
        assert!(check(root.path()).is_err());
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    #[test]
    fn unix_stale_symlink_preserved_and_live_pid_blocks() {
        let root = tempfile::tempdir().unwrap();
        let root = root.path().canonicalize().unwrap();
        let path = root.join("SingletonLock");
        let host = hostname().unwrap();
        let mut child = std::process::Command::new("/usr/bin/true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        std::os::unix::fs::symlink(format!("{host}-{pid}"), &path).unwrap();
        assert!(check(&root).is_ok());
        assert!(path.symlink_metadata().unwrap().file_type().is_symlink());
        std::fs::remove_file(&path).unwrap();
        std::os::unix::fs::symlink(format!("{host}-{}", std::process::id()), &path).unwrap();
        assert!(check(&root).is_err());
    }
    #[cfg(target_os = "macos")]
    #[test]
    #[ignore = "Starts installed macOS Edge and Chrome in disposable profiles for three close/reopen cycles"]
    fn native_macos_chromium_profiles_reopen_with_persistent_singleton_files() {
        for browser in ["edge", "chrome"] {
            let site = Site::new();
            let root = tempfile::tempdir().unwrap();
            let root = root.path().canonicalize().unwrap();
            let profile = root.join(format!("browser-{browser}"));
            std::fs::create_dir_all(&profile).unwrap();
            struct Cleanup(std::path::PathBuf);
            impl Drop for Cleanup {
                fn drop(&mut self) {
                    let _ = crate::platform::force_close_profile(&self.0);
                }
            }
            let _cleanup = Cleanup(profile.clone());
            let marker = profile.join("keep-existing-profile-data");
            std::fs::write(&marker, b"keep-login-data-fixture").unwrap();
            for cycle in 0..3 {
                let lock = profile.join("SingletonLock");
                if lock.symlink_metadata().is_err() {
                    std::fs::write(&lock, b"").unwrap();
                }
                crate::platform::ensure_profile_closed(&profile).unwrap();
                crate::platform::spawn_browser_with_proxy(
                    browser, &profile, &site.url, "direct", None,
                )
                .unwrap();
                let deadline = std::time::Instant::now() + std::time::Duration::from_secs(15);
                while crate::platform::profile_process_ids(&profile)
                    .unwrap()
                    .is_empty()
                {
                    assert!(
                        std::time::Instant::now() < deadline,
                        "{browser} never started (cycle={cycle})"
                    );
                    std::thread::sleep(std::time::Duration::from_millis(100));
                }
                assert_eq!(
                    site.reports
                        .recv_timeout(std::time::Duration::from_secs(15))
                        .unwrap(),
                    cycle > 0,
                    "{browser} must load a real document and preserve its cookie (cycle={cycle})"
                );
                assert!(
                    crate::platform::ensure_profile_closed(&profile).is_err(),
                    "live profile must remain protected"
                );
                crate::platform::close_profile(&profile).unwrap();
                crate::platform::ensure_profile_closed(&profile).unwrap();
                assert_eq!(std::fs::read(&marker).unwrap(), b"keep-login-data-fixture");
                println!("macOS {browser} close/reopen cycle {cycle}: live protection and idle lock verified");
            }
        }
    }
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "Starts installed Chrome, Edge and Firefox with X11/XWayland in disposable profiles"]
    fn native_linux_browsers_close_reopen_and_preserve_other_profiles() {
        use crate::platform;
        struct Cleanup(std::path::PathBuf);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = platform::force_close_profile(&self.0); }
        }
        for browser in ["chrome", "edge", "firefox"] {
            let root = tempfile::tempdir().unwrap();
            let root = root.path().canonicalize().unwrap();
            let profile = root.join(format!("browser-{browser}"));
            let other = root.join(format!("browser-{browser}-other"));
            std::fs::create_dir_all(&profile).unwrap();
            std::fs::create_dir_all(&other).unwrap();
            let _cleanup = Cleanup(profile.clone());
            let _other_cleanup = Cleanup(other.clone());
            let site = Site::new();
            let other_site = Site::new();
            let marker = profile.join("existing-profile-data");
            std::fs::write(&marker, b"preserve").unwrap();
            platform::spawn_browser_with_proxy(browser, &other, &other_site.url, "direct", None).unwrap();
            assert!(!other_site.reports.recv_timeout(std::time::Duration::from_secs(25)).unwrap());
            let other_pids = platform::profile_process_ids(&other).unwrap();
            assert!(!other_pids.is_empty());
            for cycle in 0..3 {
                platform::ensure_profile_closed(&profile).unwrap();
                platform::spawn_browser_with_proxy(browser, &profile, &site.url, "direct", None).unwrap();
                assert_eq!(site.reports.recv_timeout(std::time::Duration::from_secs(25)).unwrap(), cycle > 0,
                    "{browser} cookie did not survive normal shutdown cycle {cycle}");
                assert!(platform::ensure_profile_closed(&profile).is_err());
                platform::focus_profile(&profile).unwrap();
                platform::close_profile(&profile).unwrap();
                platform::ensure_profile_closed(&profile).unwrap();
                assert_eq!(std::fs::read(&marker).unwrap(), b"preserve");
                assert_eq!(platform::profile_process_ids(&other).unwrap(), other_pids,
                    "closing {browser} profile affected a different profile");
                assert!(platform::ensure_profile_closed(&other).is_err());
                println!("Linux {browser} close/reopen cycle {cycle}: cookie retained, other profile untouched");
            }
            platform::close_profile(&other).unwrap();
        }
    }
}
