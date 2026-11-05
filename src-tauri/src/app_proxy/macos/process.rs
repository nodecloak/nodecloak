use super::security::{self, PrivateCore};
use std::{
    io::{Read, Write},
    process::{Child, ChildStderr, Command, Stdio},
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::{Duration, Instant},
};
pub struct CoreProcess {
    child: Child,
    _files: PrivateCore,
}
impl CoreProcess {
    pub fn start(
        files: PrivateCore,
        mut bytes: Vec<u8>,
        cancel: Arc<AtomicBool>,
    ) -> Result<(Self, ChildStderr), String> {
        if cancel.load(Ordering::Acquire) {
            return Err("应用代理启动已取消".into());
        }
        // Inherit the launchd job's process group: launchd reaps this child even
        // when the root Rust helper is killed without running destructors.
        let child = Command::new(&files.binary)
            .args(["run", "--disable-color", "-c", "stdin"])
            .env_clear()
            .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
            .current_dir(&files.directory)
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| "无法启动 Mac 应用代理内核")?;
        let mut core = Self {
            child,
            _files: files,
        };
        let mut input = core.child.stdin.take().ok_or("应用代理配置通道不可用")?;
        let (tx, rx) = std::sync::mpsc::sync_channel(1);
        std::thread::spawn(move || {
            let result = input.write_all(&bytes);
            use zeroize::Zeroize;
            bytes.zeroize();
            drop(input);
            let _ = tx.send(result);
        });
        if !matches!(rx.recv_timeout(Duration::from_secs(5)), Ok(Ok(()))) {
            return Err("传递应用代理配置超时或失败".into());
        }
        let stderr = core.child.stderr.take().ok_or("应用代理日志通道不可用")?;
        Ok((core, stderr))
    }
    pub fn alive(&mut self) -> bool {
        matches!(self.child.try_wait(), Ok(None))
    }
    pub fn exit_code(&mut self) -> String {
        self.child
            .try_wait()
            .ok()
            .flatten()
            .map(|s| s.to_string())
            .unwrap_or_else(|| "unknown".into())
    }
    pub fn stop(&mut self) -> Result<(), String> {
        if !self.alive() {
            return Ok(());
        }
        unsafe {
            libc::kill(self.child.id() as i32, libc::SIGINT);
        }
        let deadline = Instant::now() + Duration::from_secs(5);
        while self.alive() && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        if self.alive() {
            self.child.kill().map_err(|_| "无法停止应用代理内核")?;
        }
        self.child.wait().map_err(|_| "等待应用代理内核退出失败")?;
        Ok(())
    }
}
impl Drop for CoreProcess {
    fn drop(&mut self) {
        let _ = self.stop();
    }
}
pub fn ownership_mutex() -> Result<security::Ownership, String> {
    security::ownership_mutex()
}
pub struct CoreSignal {
    pub ready: AtomicBool,
    pub error: Mutex<Option<String>>,
}
pub fn watch_output(file: ChildStderr, secrets: Vec<String>) -> Arc<CoreSignal> {
    use std::io::BufRead;
    let signal = Arc::new(CoreSignal {
        ready: AtomicBool::new(false),
        error: Mutex::new(None),
    });
    let copy = signal.clone();
    std::thread::spawn(move || {
        let mut reader = std::io::BufReader::new(file);
        let mut discarding = false;
        loop {
            let mut bytes = Vec::new();
            if reader
                .by_ref()
                .take(16385)
                .read_until(b'\n', &mut bytes)
                .unwrap_or(0)
                == 0
            {
                break;
            }
            if discarding || bytes.len() > 16384 {
                discarding = !bytes.ends_with(b"\n");
                continue;
            }
            let mut line = String::from_utf8_lossy(&bytes).trim().to_owned();
            if line.contains("sing-box started") {
                copy.ready.store(true, Ordering::Release);
            }
            if line.contains("FATAL") || line.to_ascii_lowercase().contains("fatal[") {
                for s in &secrets {
                    if !s.is_empty() {
                        line = line.replace(s, "[redacted]");
                    }
                }
                line.truncate(line.floor_char_boundary(2048));
                if let Ok(mut error) = copy.error.lock() {
                    *error = Some(line);
                }
            }
        }
    });
    signal
}
