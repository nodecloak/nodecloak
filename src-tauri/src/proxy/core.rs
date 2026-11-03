//! Per-profile owned processes. Configuration travels only through private pipes.
use super::*;
use serde_json::Value;
use std::sync::atomic::{AtomicU8, Ordering};
#[cfg(any(windows, test))]
fn sid_from_csv(text: &str) -> Option<&str> {
    text.split('"').find(|value| {
        let Some(suffix) = value.strip_prefix("S-1-") else {
            return false;
        };
        let parts: Vec<_> = suffix.split('-').collect();
        (2..=16).contains(&parts.len())
            && parts
                .iter()
                .all(|part| !part.is_empty() && part.bytes().all(|b| b.is_ascii_digit()))
            && parts[0].parse::<u64>().is_ok_and(|v| v <= 0xffff_ffff_ffff)
            && parts[1..].iter().all(|part| part.parse::<u32>().is_ok())
    })
}
use std::{
    io::{Read, Write},
    process::{Child, ChildStdin, Stdio},
    time::Instant,
};
#[cfg(test)]
mod checks;

#[cfg(windows)]
struct ChildJob(windows_sys::Win32::Foundation::HANDLE);
#[cfg(windows)]
impl ChildJob {
    fn attach(child: &Child) -> Result<Self, String> {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::*;
        unsafe {
            let handle = CreateJobObjectW(std::ptr::null(), std::ptr::null());
            if handle.is_null() {
                return Err("无法隔离代理内核进程".into());
            }
            let job = Self(handle);
            let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = std::mem::zeroed();
            limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
            if SetInformationJobObject(
                handle,
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            ) == 0
                || AssignProcessToJobObject(handle, child.as_raw_handle() as _) == 0
            {
                return Err("无法隔离代理内核进程".into());
            }
            Ok(job)
        }
    }
}
#[cfg(windows)]
impl Drop for ChildJob {
    fn drop(&mut self) {
        unsafe {
            windows_sys::Win32::Foundation::CloseHandle(self.0);
        }
    }
}

pub struct Core {
    child: Mutex<Child>,
    input: Mutex<Option<ChildStdin>>,
    status: Arc<AtomicU8>,
}
impl Core {
    fn running(&self) -> bool {
        self.child
            .lock()
            .map(|mut c| matches!(c.try_wait(), Ok(None)))
            .unwrap_or(false)
    }
    pub fn alive(&self) -> bool {
        self.status.load(Ordering::Acquire) == 1 && self.running()
    }
    pub fn spawn(config: &Value) -> Result<Self, String> {
        let exe = host_exe()?;
        let mut command = crate::platform::command(exe.to_str().ok_or("代理内核程序路径无效")?);
        command
            .arg("--nodecloak-proxy-core")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null());
        // Child dialers must not inherit a proxy environment and loop through
        // another application's global proxy variables.
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            command.env_remove(key);
        }
        let child = command.spawn().map_err(|_| "无法启动副本代理内核")?;
        let mut core = Self {
            child: Mutex::new(child),
            input: Mutex::new(None),
            status: Arc::new(AtomicU8::new(0)),
        };
        let mut output = core
            .child
            .get_mut()
            .map_err(|_| "代理内核状态不可用")?
            .stdout
            .take()
            .ok_or("代理内核状态通道不可用")?;
        let status = core.status.clone();
        std::thread::spawn(move || {
            let mut message = [0u8; 1];
            if output.read_exact(&mut message).is_ok() && message == [b'R'] {
                status.store(1, Ordering::Release);
                // EOF is a second independent indication that the helper/core
                // has exited. No secrets or upstream log text cross this pipe.
                let _ = output.read(&mut message);
            }
            status.store(2, Ordering::Release);
        });
        let mut input = core
            .child
            .get_mut()
            .map_err(|_| "代理内核状态不可用")?
            .stdin
            .take()
            .ok_or("代理内核配置通道不可用")?;
        let bytes = serde_json::to_vec(config).map_err(|_| "无法生成节点配置")?;
        input
            .write_all(&(bytes.len() as u32).to_le_bytes())
            .and_then(|_| input.write_all(&bytes))
            .map_err(|_| "无法传递节点配置")?;
        *core.input.get_mut().map_err(|_| "代理内核状态不可用")? = Some(input);
        Ok(core)
    }
}
fn host_exe() -> Result<std::path::PathBuf, String> {
    #[cfg(test)]
    if let Some(path) = std::env::var_os("NODECLOAK_TEST_PROXY_HOST") {
        return Ok(path.into());
    }
    std::env::current_exe().map_err(|_| "无法定位内置代理程序".into())
}
impl Drop for Core {
    fn drop(&mut self) {
        if let Ok(input) = self.input.get_mut() {
            input.take();
        }
        if let Ok(child) = self.child.get_mut() {
            let deadline = Instant::now() + Duration::from_secs(5);
            while matches!(child.try_wait(), Ok(None)) && Instant::now() < deadline {
                std::thread::sleep(Duration::from_millis(20));
            }
            if matches!(child.try_wait(), Ok(None)) {
                let _ = child.kill();
            }
            let _ = child.wait();
        }
    }
}

pub async fn start(up: Upstream) -> Result<(Upstream, Arc<Core>), String> {
    // Retry the bind/reservation handoff race. Readiness must come from the
    // owned child, not from a process that happens to answer on this port.
    for _ in 0..3 {
        let reservation = TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0))
            .await
            .map_err(|_| "无法分配节点代理端口")?;
        let port = reservation
            .local_addr()
            .map_err(|_| "无法读取节点代理端口")?
            .port();
        let username = uuid::Uuid::new_v4().to_string();
        let password = uuid::Uuid::new_v4().to_string();
        let config = node::config(&up, port, &username, &password)?;
        drop(reservation);
        let core = Arc::new(Core::spawn(&config)?);
        let internal = Upstream {
            config: ProxyConfig {
                mode: "http".into(),
                host: "127.0.0.1".into(),
                port,
                username,
                ..Default::default()
            },
            password,
        };
        let deadline = Instant::now() + Duration::from_secs(5);
        while core.running() && Instant::now() < deadline {
            // An authenticated local CONNECT that is rejected by the proxy
            // proves the listener belongs to this generated configuration,
            // without dialing any user node or public website.
            if core.alive() && ready(&internal).await {
                return Ok((internal, core));
            }
            tokio::time::sleep(Duration::from_millis(40)).await;
        }
        drop(core);
    }
    Err("副本代理内核未就绪；请重新导入节点或重启副本".into())
}
async fn ready(up: &Upstream) -> bool {
    // The reserved health domain is rejected by a local route rule before any
    // outbound dial. Require both a rejected wrong credential and accepted
    // generated credential as a functional check after the owned helper reports
    // that this exact inbound successfully bound. HTTP responses alone cannot
    // establish ownership of a released ephemeral port.
    let check = async {
        for (auth, expected) in [
            (String::new(), b"HTTP/1.1 407".as_slice()),
            (credentials(up), b"HTTP/1.1 200".as_slice()),
        ] {
            let mut socket = TcpStream::connect(("127.0.0.1", up.config.port))
                .await
                .ok()?;
            socket.write_all(format!("CONNECT nodecloak-core-health.invalid:443 HTTP/1.1\r\nHost: nodecloak-core-health.invalid:443\r\n{auth}\r\n").as_bytes()).await.ok()?;
            let mut status = vec![0; expected.len()];
            socket.read_exact(&mut status).await.ok()?;
            if status != expected {
                return Some(false);
            }
        }
        Some(true)
    };
    matches!(
        tokio::time::timeout(Duration::from_millis(200), check).await,
        Ok(Some(true))
    )
}

/// Internal helper entered before Tauri, tray or browser initialization.
pub fn run_stdio() -> Result<(), String> {
    let mut input = std::io::stdin().lock();
    let mut size = [0u8; 4];
    input
        .read_exact(&mut size)
        .map_err(|_| "代理配置通道已关闭")?;
    let length = u32::from_le_bytes(size) as usize;
    if !(1..=32768).contains(&length) {
        return Err("代理配置大小无效".into());
    }
    let mut bytes = vec![0; length];
    input
        .read_exact(&mut bytes)
        .map_err(|_| "代理配置未完整传输")?;
    let config: Value = serde_json::from_slice(&bytes).map_err(|_| "代理配置格式无效")?;
    // Only generated loopback HTTP configs may enter this host; never accept
    // arbitrary TUN, route manipulation, services, API or DNS listeners.
    validate_helper_config(&config)?;
    let directory = tempfile::Builder::new()
        .prefix("nodecloak-core-")
        .tempdir()
        .map_err(|_| "无法准备内置代理程序")?;
    #[cfg(windows)]
    {
        let sid = crate::platform::command("whoami.exe")
            .args(["/user", "/fo", "csv", "/nh"])
            .output()
            .map_err(|_| "无法限制代理目录访问")?;
        let text = String::from_utf8_lossy(&sid.stdout);
        let sid = sid_from_csv(&text).ok_or("无法限制代理目录访问")?;
        if !crate::platform::command("icacls.exe")
            .arg(directory.path())
            .args(["/inheritance:r", "/grant:r", &format!("*{sid}:(OI)(CI)F")])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "无法限制代理目录访问")?
            .success()
        {
            return Err("无法限制代理目录访问".into());
        }
    }
    let binary = directory.path().join(if cfg!(windows) {
        "sing-box.exe"
    } else {
        "sing-box"
    });
    let mut decompressed = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!(concat!(env!("OUT_DIR"), "/core.gz"))[..])
        .read_to_end(&mut decompressed)
        .map_err(|_| "内置代理程序损坏")?;
    use sha2::{Digest, Sha256};
    if format!("{:x}", Sha256::digest(&decompressed))
        != include_str!(concat!(env!("OUT_DIR"), "/core.sha256")).trim()
    {
        return Err("内置代理程序校验失败".into());
    }
    std::fs::write(&binary, decompressed).map_err(|_| "无法准备内置代理程序")?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&binary, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法设置代理程序权限")?;
    }
    let mut child = crate::platform::command(binary.to_str().ok_or("代理程序路径无效")?)
        .args(["run", "--disable-color", "-c", "stdin"])
        .current_dir(directory.path())
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|_| "无法启动节点内核")?;
    #[cfg(windows)]
    let _job = match ChildJob::attach(&child) {
        Ok(job) => job,
        Err(error) => {
            let _ = child.kill();
            let _ = child.wait();
            return Err(error);
        }
    };
    let write = child
        .stdin
        .take()
        .ok_or("节点配置通道不可用")
        .and_then(|mut pipe| pipe.write_all(&bytes).map_err(|_| "无法传递节点配置"));
    use zeroize::Zeroize;
    bytes.zeroize();
    if let Err(error) = write {
        let _ = child.kill();
        let _ = child.wait();
        return Err(error.into());
    }
    let stderr = child.stderr.take().ok_or("节点启动状态通道不可用")?;
    let expected = format!(
        "inbound/http[profile]: tcp server started at 127.0.0.1:{}",
        config["inbounds"][0]["listen_port"]
            .as_u64()
            .ok_or("节点端口无效")?
    );
    let (startup_tx, startup_rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        use std::io::BufRead;
        let mut reader = std::io::BufReader::new(stderr);
        let mut bound = false;
        loop {
            let mut line = Vec::new();
            // Bound allocation even if a peer provokes an oversized log line.
            let read = reader.by_ref().take(32768).read_until(b'\n', &mut line);
            if !matches!(read,Ok(n) if n>0) || line.len() == 32768 {
                break;
            }
            let text = String::from_utf8_lossy(&line);
            if text
                .trim_end()
                .split_once("] ")
                .is_some_and(|(_, message)| message == expected)
            {
                bound = true;
            }
            if bound && text.contains("] sing-box started (") {
                let _ = startup_tx.send(());
                break;
            }
        }
        // Logs can contain remote endpoints. Drain without storing, displaying
        // or forwarding them; only the startup event crosses the private pipe.
        let _ = std::io::copy(&mut reader, &mut std::io::sink());
    });
    // The parent keeps this pipe open for the lifetime of its relay. Normal
    // drop and an abrupt parent exit both produce EOF and close only this core.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let mut rest = [0u8; 1];
        let _ = std::io::stdin().read(&mut rest);
        let _ = tx.send(());
    });
    drop(input);
    let mut announced = false;
    loop {
        if !announced && startup_rx.try_recv().is_ok() {
            if std::io::stdout()
                .write_all(b"R")
                .and_then(|_| std::io::stdout().flush())
                .is_err()
            {
                let _ = child.kill();
                let _ = child.wait();
                return Err("代理状态通道已关闭".into());
            }
            announced = true;
        }
        if rx.recv_timeout(Duration::from_millis(50)).is_ok() {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(());
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                return if status.success() {
                    Ok(())
                } else {
                    Err("节点内核已退出".into())
                }
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("无法读取节点内核状态".into());
            }
            _ => {}
        }
    }
}
fn validate_helper_config(config: &Value) -> Result<(), String> {
    // Reconstruct the typed connection and require byte-for-value equivalence
    // with our generator. Even this private entry point cannot accept extra
    // dialer, file-output, API, routing, interface or DNS-listener options.
    let text = |v: &Value| v.as_str().unwrap_or_default().to_owned();
    let port = |v: &Value| {
        v.as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .filter(|n| *n > 0)
            .ok_or("节点端口无效")
    };
    let outbound = &config["outbounds"][0];
    let tls = &outbound["tls"];
    let transport = &outbound["transport"];
    let mut options = node::NodeOptions::default();
    if !tls.is_null() {
        options.tls = if tls["reality"].is_null() {
            "tls"
        } else {
            "reality"
        }
        .into();
        options.server_name = text(&tls["server_name"]);
        options.fingerprint = text(&tls["utls"]["fingerprint"]);
        options.public_key = text(&tls["reality"]["public_key"]);
        options.short_id = text(&tls["reality"]["short_id"]);
        if let Some(alpn) = tls.get("alpn") {
            options.alpn = serde_json::from_value(alpn.clone()).map_err(|_| "节点 ALPN 无效")?;
        }
    }
    options.flow = text(&outbound["flow"]);
    if !transport.is_null() {
        options.transport = text(&transport["type"]);
        options.path = text(&transport["path"]);
        options.service_name = text(&transport["service_name"]);
        options.host = text(if options.transport == "ws" {
            &transport["headers"]["Host"]
        } else {
            &transport["host"]
        });
    }
    if outbound["type"] == "vmess" {
        options.security = text(&outbound["security"]);
        options.alter_id = outbound["alter_id"]
            .as_u64()
            .and_then(|n| u16::try_from(n).ok())
            .ok_or("节点 Alter ID 无效")?;
    }
    let first = config["outbounds"]
        .get(1)
        .map(|first| {
            Ok::<_, String>(LocalProxy {
                mode: match first["type"].as_str() {
                    Some("socks") => "socks5",
                    Some("http") => {
                        if first["tls"].is_null() {
                            "http"
                        } else {
                            "https"
                        }
                    }
                    _ => return Err("前置代理类型无效".into()),
                }
                .into(),
                host: text(&first["server"]),
                port: port(&first["server_port"])?,
            })
        })
        .transpose()?;
    let up = Upstream {
        config: ProxyConfig {
            mode: text(&outbound["type"]),
            host: text(&outbound["server"]),
            port: port(&outbound["server_port"])?,
            node: Some(options),
            pre_proxy: first,
            ..Default::default()
        },
        password: text(&outbound["uuid"]),
    };
    let inbound = &config["inbounds"][0];
    let user = text(&inbound["users"][0]["username"]);
    let password = text(&inbound["users"][0]["password"]);
    if user.is_empty()
        || password.is_empty()
        || node::config(&up, port(&inbound["listen_port"])?, &user, &password)? != *config
    {
        return Err("仅允许副本本机代理配置".into());
    }
    Ok(())
}
