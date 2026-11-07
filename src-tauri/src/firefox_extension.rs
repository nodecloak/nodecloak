//! Signed extension delivery for manually used Firefox profiles.
use crate::profiles::Preferences;
use serde_json::Value;
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::{Path, PathBuf},
    time::{Duration, Instant},
};

const ID: &str = "fingerprint@nodecloak.com";
const HOST: &str = "com.nodecloak.fingerprint";
const XPI: &[u8] = include_bytes!("../resources/firefox/nodecloak-fingerprint-0.1.0.xpi");
const XPI_SHA256: &str = "a049892f7e3c27ffb748251612dae722f6bb0172cbd4d6342c885db4df080be2";

pub(crate) fn eligible(browser: &str, preferences: &Preferences) -> bool {
    if browser != "firefox"
        || !preferences
            .fingerprint
            .as_ref()
            .is_some_and(|f| f.requires_injection())
    {
        return false;
    }
    true
}
fn installed(value: &Value) -> bool {
    value["addons"].as_array().is_some_and(|items| {
        items.iter().any(|a| {
            a["id"] == ID
                && a["version"] == "0.1.0"
                && a["active"] == true
                && a["userDisabled"] == false
                && a["appDisabled"] == false
                && a["signedState"] == 2
                && a["foreignInstall"] == false
                && a["location"] == "app-profile"
        })
    })
}
fn ready_report(body: &[u8], expected_marker: bool) -> bool {
    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Report {
        marker: bool,
        webdriver: bool,
    }
    body.len() <= 128
        && serde_json::from_slice::<Report>(body)
            .is_ok_and(|r| r.marker == expected_marker && !r.webdriver)
}

fn read_state(profile: &Path) -> Result<Value, String> {
    let path = profile.join("extensions.json");
    crate::profiles::guard_tree(&path)?;
    match std::fs::metadata(&path) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Value::Null),
        Ok(m) if m.is_file() && m.len() <= 32 * 1024 * 1024 => {
            let bytes = std::fs::read(&path).map_err(|_| "无法读取 Firefox 扩展状态")?;
            serde_json::from_slice(&bytes).map_err(|_| "Firefox 扩展状态无效，原文件已保留".into())
        }
        _ => Err("无法读取 Firefox 扩展状态".into()),
    }
}

fn pinned_package(root: &Path) -> Result<PathBuf, String> {
    use sha2::{Digest, Sha256};
    let digest = Sha256::digest(XPI)
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if digest != XPI_SHA256 {
        return Err("内置 Firefox 指纹扩展校验失败".into());
    }
    let folder = root.join("firefox-privacy");
    crate::profiles::guard_tree(&folder)?;
    std::fs::create_dir_all(&folder).map_err(|_| "无法创建 Firefox 扩展目录")?;
    let path = folder.join("nodecloak-fingerprint-0.1.0.xpi");
    crate::profiles::guard_tree(&path)?;
    crate::engine::atomic_write(&path, XPI)?;
    Ok(path)
}

#[cfg(windows)]
fn registered_host_is_ours(existing: &Path, expected: &Path) -> bool {
    let resolve = |path: &Path| -> Option<PathBuf> {
        if !path.is_absolute() || crate::profiles::guard_tree(path).is_err() {
            return None;
        }
        // Debug launches can store a \\?\ path while an installed app uses
        // the ordinary spelling. Compare normalized paths, preserving the
        // guard against symlinked or foreign registrations. A deleted manifest
        // can be recreated only when its real parent and filename still match.
        path.canonicalize()
            .ok()
            .or_else(|| Some(path.parent()?.canonicalize().ok()?.join(path.file_name()?)))
    };
    resolve(existing)
        .zip(resolve(expected))
        .is_some_and(|(a, b)| a == b)
}

fn register_host(root: &Path) -> Result<(), String> {
    let folder = root.join("firefox-privacy");
    let path = folder.join("com.nodecloak.fingerprint.json");
    let exe = std::env::current_exe().map_err(|_| "无法定位 Firefox 扩展宿主")?;
    // The lib-test harness has no app main() native-host dispatch. Installed
    // browser fixtures supply a separately built app executable explicitly.
    #[cfg(test)]
    let exe = std::env::var_os("NODECLOAK_TEST_FIREFOX_NATIVE_HOST")
        .map(PathBuf::from)
        .unwrap_or(exe);
    crate::profiles::guard_tree(&folder)?;
    std::fs::create_dir_all(&folder).map_err(|_| "无法创建 Firefox 扩展宿主目录")?;
    crate::profiles::guard_tree(&path)?;
    let content = serde_json::to_vec(&serde_json::json!({"name":HOST,"description":"NodeCloak profile privacy host","path":exe,"type":"stdio","allowed_extensions":[ID]})).map_err(|_| "扩展宿主配置无效")?;
    #[cfg(windows)]
    {
        use winreg::{enums::*, RegKey};
        let key_name = format!(r"Software\Mozilla\NativeMessagingHosts\{HOST}");
        for hive in [HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE] {
            for view in [KEY_WOW64_64KEY, KEY_WOW64_32KEY] {
                match RegKey::predef(hive).open_subkey_with_flags(&key_name, KEY_READ | view) {
                    Ok(key) => {
                        let existing: String = key
                            .get_value("")
                            .map_err(|_| "Firefox 扩展宿主已被其他配置占用")?;
                        if !registered_host_is_ours(Path::new(&existing), &path) {
                            return Err("Firefox 扩展宿主已被其他配置占用，原配置已保留".into());
                        }
                    }
                    Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                    Err(_) => return Err("无法检查 Firefox 扩展宿主注册".into()),
                }
            }
        }
        let (key, disposition) = RegKey::predef(HKEY_CURRENT_USER)
            .create_subkey_with_flags(&key_name, KEY_READ | KEY_WRITE | KEY_WOW64_64KEY)
            .map_err(|_| "无法注册 Firefox 扩展宿主")?;
        if disposition == REG_OPENED_EXISTING_KEY {
            let existing: String = key
                .get_value("")
                .map_err(|_| "Firefox 扩展宿主已被其他配置占用")?;
            if !registered_host_is_ours(Path::new(&existing), &path) {
                return Err("Firefox 扩展宿主注册发生变化，原配置已保留".into());
            }
        }
        crate::engine::atomic_write(&path, &content)?;
        key.set_value("", &path.to_string_lossy().to_string())
            .map_err(|_| "无法保存 Firefox 扩展宿主注册")?;
    }
    #[cfg(not(windows))]
    {
        let home = std::env::var_os("HOME").ok_or("无法定位 Firefox 扩展宿主目录")?;
        #[cfg(target_os = "macos")]
        let target = PathBuf::from(home).join("Library/Application Support/Mozilla/NativeMessagingHosts/com.nodecloak.fingerprint.json");
        #[cfg(not(target_os = "macos"))]
        let target = PathBuf::from(home)
            .join(".mozilla/native-messaging-hosts/com.nodecloak.fingerprint.json");
        crate::profiles::guard_tree(&target)?;
        if target.exists() {
            let old: Value = serde_json::from_slice(
                &std::fs::read(&target).map_err(|_| "无法读取扩展宿主注册")?,
            )
            .map_err(|_| "扩展宿主注册无效，原文件已保留")?;
            if old["name"] != HOST
                || old["description"] != "NodeCloak profile privacy host"
                || old["allowed_extensions"] != serde_json::json!([ID])
            {
                return Err("Firefox 扩展宿主已被其他配置占用，原配置已保留".into());
            }
        }
        std::fs::create_dir_all(target.parent().unwrap())
            .map_err(|_| "无法创建扩展宿主注册目录")?;
        crate::engine::atomic_write(&target, &content)?;
        crate::engine::atomic_write(&path, &content)?;
    }
    Ok(())
}

/// Install/uninstall only this signed companion, using Firefox's AddonManager.
/// The installation-only browser stays on about:blank and exits before browsing.
fn bootstrap(
    profile: &Path,
    package: Option<&Path>,
    mode: &str,
    relay: Option<u16>,
) -> Result<(), String> {
    crate::platform::ensure_profile_closed(profile)?;
    let profile = crate::platform::prepare_firefox_profile(profile)?;
    crate::firefox::prepare_startup(&profile)?;
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配 Firefox 安装端口")?;
    let port = listener
        .local_addr()
        .map_err(|_| "无法读取 Firefox 安装端口")?
        .port();
    drop(listener);
    let before = ["marionette.port", "remote.prefs.recommended"]
        .map(|key| crate::firefox::snapshot(&profile, key).map(|value| (key.to_owned(), value)));
    let before = before.into_iter().collect::<Result<Vec<_>, _>>()?;
    let changes = before
        .iter()
        .map(|(key, old)| {
            let value = if key == "marionette.port" {
                serde_json::json!(port)
            } else {
                serde_json::json!(false)
            };
            (
                key.clone(),
                old.clone(),
                serde_json::json!({"user":value,"runtime":value}),
            )
        })
        .collect::<Vec<_>>();
    crate::firefox::apply(&profile, &changes, false)?;
    let result = (|| {
        crate::platform::spawn_firefox_addon_setup(&profile, mode, relay)?;
        let deadline = Instant::now() + Duration::from_secs(25);
        let mut socket = loop {
            if let Ok(s) = TcpStream::connect_timeout(
                &format!("127.0.0.1:{port}").parse().unwrap(),
                Duration::from_millis(200),
            ) {
                break s;
            }
            if Instant::now() >= deadline {
                return Err("Firefox 扩展安装接口未就绪，请检查 Firefox 版本并重试".into());
            }
            std::thread::sleep(Duration::from_millis(100));
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(25)))
            .map_err(|_| "无法配置扩展安装接口")?;
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| "无法配置扩展安装接口")?;
        let hello = read_packet(&mut socket)?;
        if hello["marionetteProtocol"] != 3 {
            return Err("Firefox 扩展安装协议不受支持".into());
        }
        marionette(
            &mut socket,
            1,
            "WebDriver:NewSession",
            serde_json::json!({"capabilities":{"alwaysMatch":{"acceptInsecureCerts":false}}}),
        )?;
        if let Some(path) = package {
            let result = marionette(
                &mut socket,
                2,
                "Addon:Install",
                serde_json::json!({"path":path,"temporary":false}),
            )?;
            if result["value"] != ID {
                return Err("Firefox 返回的扩展标识无效".into());
            }
        } else {
            marionette(
                &mut socket,
                2,
                "Addon:Uninstall",
                serde_json::json!({"id":ID}),
            )?;
        }
        marionette(
            &mut socket,
            3,
            "Marionette:Quit",
            serde_json::json!({"flags":["eAttemptQuit"]}),
        )?;
        wait_setup_exit(socket, &profile, Duration::from_secs(30))
            .map_err(|error| format!("Firefox 扩展安装窗口尚未退出，请关闭后重试：{error}"))?;
        Ok(())
    })();
    if result.is_err() {
        let _ = crate::platform::close_profile(&profile);
        let _ = crate::platform::force_close_profile(&profile);
    }
    let restore = before
        .into_iter()
        .map(|(key, old)| crate::firefox::snapshot(&profile, &key).map(|now| (key, now, old)))
        .collect::<Result<Vec<_>, _>>()?;
    crate::firefox::apply(&profile, &restore, false)?;
    result
}

#[cfg(test)]
pub(crate) fn install_session_fixture(root:&Path,profile:&Path,settings:&crate::fingerprint::Settings)->Result<(),String>{
    assert!(std::env::var_os("NODECLOAK_TEST_REUSE_FIREFOX_HOST").is_some(),"Opt in to the existing NodeCloak native host; this check never edits its registration");
    assert!(root.starts_with(std::env::temp_dir().canonicalize().unwrap()));
    assert!(profile.starts_with(root));
    crate::engine::atomic_write(&profile.join("nodecloak-fingerprint-extension.json"),&serde_json::to_vec(settings).unwrap())?;
    bootstrap(profile,Some(&pinned_package(root)?),"direct",None)
}

fn wait_setup_exit(socket: TcpStream, profile: &Path, timeout: Duration) -> Result<(), String> {
    // Quit acknowledges shutdown start, not completion. Release the installation
    // transport before waiting for Firefox and its helpers to finish flushing.
    let _ = socket.shutdown(std::net::Shutdown::Both);
    drop(socket);
    let started = Instant::now();
    let result = crate::platform::wait_profile_closed(profile, timeout);
    let detail = format!("installation shutdown {}; elapsed_ms={}; main_processes={:?}",
        if result.is_ok() { "complete" } else { "timed out" },
        started.elapsed().as_millis(),
        crate::platform::profile_process_ids(profile).ok().map(|ids| ids.len()));
    crate::browser_control::log_startup_endpoint(profile, "firefox", "extension_setup_shutdown", &detail);
    result
}

fn read_packet(socket: &mut TcpStream) -> Result<Value, String> {
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        socket
            .read_exact(&mut byte)
            .map_err(|_| "Firefox 扩展安装连接中断")?;
        if byte[0] == b':' {
            break;
        }
        if !byte[0].is_ascii_digit() || header.len() >= 8 {
            return Err("Firefox 扩展安装响应无效".into());
        }
        header.push(byte[0]);
    }
    let size: usize = std::str::from_utf8(&header)
        .ok()
        .and_then(|s| s.parse().ok())
        .filter(|n| *n > 0 && *n <= 1024 * 1024)
        .ok_or("Firefox 扩展安装响应过大")?;
    let mut bytes = vec![0; size];
    socket
        .read_exact(&mut bytes)
        .map_err(|_| "Firefox 扩展安装连接中断")?;
    serde_json::from_slice(&bytes).map_err(|_| "Firefox 扩展安装响应无效".into())
}
fn marionette(socket: &mut TcpStream, id: u32, name: &str, args: Value) -> Result<Value, String> {
    let body = serde_json::to_vec(&serde_json::json!([0, id, name, args]))
        .map_err(|_| "扩展安装请求无效")?;
    socket
        .write_all(format!("{}:", body.len()).as_bytes())
        .and_then(|_| socket.write_all(&body))
        .map_err(|_| "无法发送扩展安装请求")?;
    let result = read_packet(socket)?;
    if result.as_array().is_none_or(|a| a.len() != 4)
        || result[0] != 1
        || result[1] != id
        || !result[2].is_null()
    {
        // Native messages may contain paths or session ids. Keep them out of UI/logs.
        return Err("Firefox 拒绝扩展安装操作，请更新 Firefox 后重试".into());
    }
    Ok(result[3].clone())
}

pub(crate) fn prepare(
    root: &Path,
    profile: &Path,
    preferences: &Preferences,
    use_extension: bool,
    mode: &str,
    relay: Option<u16>,
) -> Result<(), String> {
    let state = read_state(profile)?;
    if use_extension {
        let settings = preferences
            .fingerprint
            .as_ref()
            .ok_or("缺少 Firefox 指纹设置")?;
        crate::fingerprint::validate(settings)?;
        let file = profile.join("nodecloak-fingerprint-extension.json");
        crate::profiles::guard_tree(&file)?;
        std::fs::create_dir_all(profile).map_err(|_| "无法创建 Firefox 副本目录")?;
        crate::engine::atomic_write(
            &file,
            &serde_json::to_vec(settings).map_err(|_| "Firefox 指纹设置无效")?,
        )?;
        register_host(root)?;
        if !installed(&state) {
            bootstrap(profile, Some(&pinned_package(root)?), mode, relay)?;
        }
        if !installed(&read_state(profile)?) {
            return Err("Firefox 指纹扩展未启用，请重试安装或更新 Firefox".into());
        }
    } else if state["addons"]
        .as_array()
        .is_some_and(|items| items.iter().any(|a| a["id"] == ID))
    {
        bootstrap(profile, None, mode, relay)?;
    }
    Ok(())
}

/// Start at a local readiness page. No destination is released unless the
/// first page script confirms the extension and a normal native browser state.
pub(crate) fn launch(
    profile: &Path,
    destination: &str,
    mode: &str,
    relay: Option<u16>,
) -> Result<(), String> {
    guarded_launch(profile, destination, true, |start| {
        crate::platform::spawn_browser_with_proxy("firefox", profile, start, mode, relay)
    })
}

pub(crate) fn guarded_launch<T>(
    profile: &Path,
    destination: &str,
    expected_marker: bool,
    spawn: impl FnOnce(&str) -> Result<T, String>,
) -> Result<T, String> {
    guarded_launch_with_timeout(
        profile,
        destination,
        expected_marker,
        spawn,
        Duration::from_secs(25),
    )
}

fn guarded_launch_with_timeout<T>(
    profile: &Path,
    destination: &str,
    expected_marker: bool,
    spawn: impl FnOnce(&str) -> Result<T, String>,
    timeout: Duration,
) -> Result<T, String> {
    guarded_launch_impl(profile,destination,expected_marker,spawn,timeout,false)
}

pub(crate) fn guarded_resume_launch<T>(profile:&Path, expected_marker:bool, spawn:impl FnOnce(&str)->Result<T,String>)->Result<T,String>{
    guarded_launch_impl(profile,"about:blank",expected_marker,spawn,Duration::from_secs(25),true)
}

fn guarded_launch_impl<T>(profile:&Path,destination:&str,expected_marker:bool,spawn:impl FnOnce(&str)->Result<T,String>,timeout:Duration,resume:bool)->Result<T,String>{
    // Internal preferences and the existing file-based diagnostics cannot be
    // reached by redirecting an HTTP content principal. They have no HTTP(S)
    // document to protect; the installed companion still gates later web loads.
    if !resume && reqwest::Url::parse(destination).is_ok_and(|url| matches!(url.scheme(), "about" | "file")) {
        return spawn(destination);
    }
    let stage = |event: &str, detail: &str| {
        crate::browser_control::log_startup_endpoint(profile, "firefox", event, detail);
    };
    stage(
        "readiness_started",
        "waiting for local startup page and native protection report",
    );
    let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "无法启动 Firefox 指纹检测页")?;
    let host = listener
        .local_addr()
        .map_err(|_| "无法读取 Firefox 检测地址")?
        .to_string();
    let token = crate::fingerprint::seed()?;
    let start_path = format!("/start/{token}");
    let ready_path = format!("/ready/{token}");
    let origin = format!("http://{host}");
    listener
        .set_nonblocking(true)
        .map_err(|_| "无法配置 Firefox 指纹检测页")?;
    let started = spawn(&format!("{origin}{start_path}"))?;
    let deadline = Instant::now() + timeout;
    let mut page_requested = false;
    while Instant::now() < deadline {
        let (mut stream, _) = match listener.accept() {
            Ok(pair) => pair,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                std::thread::sleep(Duration::from_millis(50));
                continue;
            }
            Err(_) => return Err("Firefox 指纹检测连接失败".into()),
        };
        // Accepted sockets inherit the listener's nonblocking mode on Windows.
        // A valid HTTP header/body can span packets; read_exact must wait for
        // those bytes under the bounded socket timeout rather than drop them.
        stream
            .set_nonblocking(false)
            .map_err(|_| "无法配置检测连接")?;
        stream
            .set_read_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "无法配置检测连接")?;
        stream
            .set_write_timeout(Some(Duration::from_secs(2)))
            .map_err(|_| "无法配置检测连接")?;
        let mut bytes = Vec::new();
        while bytes.len() < 8192 && !bytes.ends_with(b"\r\n\r\n") {
            let mut byte = [0];
            if stream.read_exact(&mut byte).is_err() {
                break;
            }
            bytes.push(byte[0]);
        }
        if !bytes.ends_with(b"\r\n\r\n") {
            continue;
        }
        let Ok(header) = std::str::from_utf8(&bytes) else {
            continue;
        };
        let mut lines = header.lines();
        let request = lines.next().unwrap_or("");
        let mut headers = std::collections::HashMap::new();
        let mut duplicate = false;
        for line in lines.filter(|line| !line.is_empty()) {
            let Some((name, value)) = line.split_once(':') else {
                duplicate = true;
                break;
            };
            if headers
                .insert(name.trim().to_ascii_lowercase(), value.trim())
                .is_some()
            {
                duplicate = true;
            }
        }
        if duplicate
            || headers.get("host") != Some(&host.as_str())
            || headers.contains_key("transfer-encoding")
        {
            continue;
        }
        if request == format!("GET {start_path} HTTP/1.1") {
            page_requested = true;
            stage(
                "readiness_page_requested",
                "local startup document requested",
            );
            #[cfg(test)]
            eprintln!("Synthetic Firefox startup gate document requested");
            let script = format!(
                r#"fetch({},{{method:'POST',headers:{{'Content-Type':'application/json'}},body:JSON.stringify({{marker:!!globalThis[Symbol.for('nodecloak.readback.v1')],webdriver:navigator.webdriver}})}}).then(async r=>{{const v=await r.json();if(!r.ok)throw Error(v.error);if(v.url)location.replace(v.url)}}).catch(()=>document.getElementById('status').textContent='指纹扩展未就绪，请返回 NodeCloak 查看启动提示。');"#,
                serde_json::json!(ready_path)
            );
            let html = format!("<!doctype html><meta charset=utf-8><title>NodeCloak</title><p id=status>正在加载副本指纹…</p><script nonce=\"{token}\">{script}</script>");
            let csp = format!("Content-Security-Policy: default-src 'none'; script-src 'nonce-{token}'; connect-src 'self'\r\n");
            let _ = reply(
                &mut stream,
                200,
                "text/html; charset=utf-8",
                html.as_bytes(),
                &csp,
            );
        } else if request == format!("POST {ready_path} HTTP/1.1") {
            if headers.get("origin") != Some(&origin.as_str())
                || headers.get("content-type") != Some(&"application/json")
            {
                continue;
            }
            let Some(length) = headers
                .get("content-length")
                .and_then(|s| s.parse::<usize>().ok())
                .filter(|n| *n > 0 && *n <= 128)
            else {
                continue;
            };
            let mut body = vec![0; length];
            if stream.read_exact(&mut body).is_err() {
                continue;
            }
            #[cfg(test)]
            eprintln!("Synthetic Firefox startup gate received protection report");
            if !ready_report(&body, expected_marker) {
                stage(
                    "readiness_report_rejected",
                    "protection marker or native automation state did not match",
                );
                let _ = reply(
                    &mut stream,
                    409,
                    "application/json",
                    br#"{"error":"Profile privacy is not ready"}"#,
                    "",
                );
                return Err(
                    "Firefox 指纹扩展未就绪，或仍处于自动化启动状态；请重新启动副本".into(),
                );
            }
            let body = serde_json::to_vec(&if resume {serde_json::json!({"ready":true})}else{serde_json::json!({"url":destination})})
                .map_err(|_| "启动页面地址无效")?;
            reply(&mut stream, 200, "application/json", &body, "")?;
            stage(
                "readiness_ready",
                "protection report accepted; destination released",
            );
            return Ok(started);
        } else {
            let _ = reply(&mut stream, 404, "text/plain", b"Not found", "");
        }
    }
    if page_requested {
        stage(
            "readiness_report_missing",
            "startup document served but no valid report received before deadline",
        );
        Err("Firefox 启动检测超时：检测页已打开，但未收到检测结果；请重启副本或查看启动日志".into())
    } else {
        stage(
            "readiness_page_missing",
            "no local startup document request received before deadline",
        );
        Err("Firefox 启动检测超时：未收到检测页请求；请重启副本或查看启动日志".into())
    }
}

fn reply(
    stream: &mut TcpStream,
    status: u16,
    kind: &str,
    body: &[u8],
    extra: &str,
) -> Result<(), String> {
    let header = format!("HTTP/1.1 {status} Response\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nCache-Control: no-store\r\nConnection: close\r\n{extra}\r\n", body.len());
    stream
        .write_all(header.as_bytes())
        .and_then(|_| stream.write_all(body))
        .map_err(|_| "Firefox 指纹检测响应失败".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(windows)]
    #[test]
    fn setup_disconnects_before_waiting_for_browser_profile_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let directory = tempfile::tempdir().unwrap();
        let lock = std::fs::OpenOptions::new().create(true).write(true).truncate(true)
            .share_mode(0).open(directory.path().join("parent.lock")).unwrap();
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let socket = TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        let peer = std::thread::spawn(move || {
            let (mut peer, _) = listener.accept().unwrap();
            peer.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
            let mut byte = [0];
            let eof = peer.read(&mut byte);
            drop(lock);
            eof
        });
        let result = wait_setup_exit(socket, directory.path(), Duration::from_secs(3));
        assert_eq!(peer.join().unwrap().unwrap(), 0);
        assert!(result.is_ok(), "setup must disconnect before waiting for profile release: {result:?}");
    }
    use serde_json::json;

    #[cfg(windows)]
    #[test]
    fn same_windows_native_manifest_survives_normal_and_extended_path_spelling() {
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("com.nodecloak.fingerprint.json");
        std::fs::write(&file, b"{}").unwrap();
        let extended = file.canonicalize().unwrap();
        let normal = PathBuf::from(extended.to_string_lossy().strip_prefix(r"\\?\").unwrap());
        assert!(registered_host_is_ours(&extended, &normal));
        assert!(registered_host_is_ours(&normal, &extended));
        let foreign = root.path().join("foreign.json");
        std::fs::write(&foreign, b"{}").unwrap();
        assert!(!registered_host_is_ours(&foreign, &normal));
        std::fs::remove_file(&file).unwrap();
        assert!(registered_host_is_ours(&extended, &normal));
        assert!(!registered_host_is_ours(&foreign, &normal));
        assert!(!registered_host_is_ours(
            Path::new("com.nodecloak.fingerprint.json"),
            &normal
        ));
    }

    #[test]
    fn missing_detection_page_is_distinguished_from_an_unready_extension() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        // A failed regional handoff produces no GET at all, so it must not be
        // reported as proof that an installed extension is disabled.
        let result = guarded_launch_with_timeout(
            &profile,
            "https://example.test/private-destination",
            true,
            |_| Ok(()),
            Duration::from_millis(20),
        );
        assert!(result.unwrap_err().contains("未收到检测页请求"));
        let log = std::fs::read_to_string(profile.join("nodecloak-control.jsonl")).unwrap();
        assert!(log.contains("readiness_page_missing"));
        assert!(!log.contains("private-destination"));
    }

    #[test]
    fn startup_gate_accepts_a_report_whose_body_arrives_after_its_headers() {
        let root = tempfile::tempdir().unwrap();
        let profile = root.path().canonicalize().unwrap();
        let mut peer = None;
        let result = guarded_launch_with_timeout(&profile, "https://example.test/", false, |start| {
            let url = reqwest::Url::parse(start).unwrap();
            peer = Some(std::thread::spawn(move || -> Result<Vec<u8>, std::io::Error> {
                let host = format!("127.0.0.1:{}", url.port().unwrap());
                let path = url.path().replacen("/start/", "/ready/", 1);
                let mut document = TcpStream::connect(&host)?;
                document.set_read_timeout(Some(Duration::from_secs(3)))?;
                write!(document, "GET {} HTTP/1.1\r\nHost: {host}\r\n\r\n", url.path())?;
                let mut html = Vec::new();
                document.read_to_end(&mut html)?;
                assert!(html.starts_with(b"HTTP/1.1 200"));
                let mut socket = TcpStream::connect(&host)?;
                socket.set_read_timeout(Some(Duration::from_secs(3)))?;
                let body = br#"{"marker":false,"webdriver":false}"#;
                write!(socket, "POST {path} HTTP/1.1\r\nHost: {host}\r\nOrigin: http://{host}\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n", body.len())?;
                socket.flush()?;
                // TCP does not guarantee that the JSON body shares the headers' packet.
                std::thread::sleep(Duration::from_millis(250));
                socket.write_all(body)?;
                let mut response = Vec::new();
                socket.read_to_end(&mut response)?;
                Ok(response)
            }));
            Ok(())
        }, Duration::from_secs(2));
        let response = peer.unwrap().join().unwrap();
        assert!(result.is_ok(), "fragmented report was dropped: {result:?}");
        let response = String::from_utf8(response.unwrap()).unwrap();
        assert!(response.starts_with("HTTP/1.1 200"));
        assert!(response.contains("https://example.test/"));
    }

    #[test]
    fn manual_extension_keeps_noise_and_does_not_silently_drop_regional_emulation() {
        let mut p = Preferences::default();
        p.fingerprint = Some(crate::fingerprint::fresh().unwrap());
        assert!(eligible("firefox", &p));
        assert!(!eligible("chrome", &p));
        assert!(!eligible("edge", &p));
        p.regional.timezone_mode = "custom".into();
        p.regional.timezone = "Asia/Tokyo".into();
        assert!(eligible("firefox", &p));
        p.regional.timezone_mode = "system".into();
        p.fingerprint.as_mut().unwrap().enabled = false;
        assert!(!eligible("firefox", &p));
    }

    #[test]
    fn only_the_active_permanent_signed_companion_satisfies_installation() {
        let valid = json!({"addons":[{"id":"fingerprint@nodecloak.com","version":"0.1.0",
            "active":true,"userDisabled":false,"appDisabled":false,"signedState":2,
            "foreignInstall":false,"location":"app-profile"}]});
        assert!(installed(&valid));
        for (key, value) in [
            ("id", json!("other@example.test")),
            ("version", json!("0.0.1")),
            ("active", json!(false)),
            ("userDisabled", json!(true)),
            ("appDisabled", json!(true)),
            ("signedState", json!(0)),
            ("foreignInstall", json!(true)),
            ("location", json!("app-temporary")),
        ] {
            let mut invalid = valid.clone();
            invalid["addons"][0][key] = value;
            assert!(!installed(&invalid), "accepted invalid field {key}");
        }
        assert!(!installed(&json!({})));
    }

    #[test]
    fn readiness_requires_protection_without_a_webdriver_session() {
        assert!(ready_report(br#"{"marker":true,"webdriver":false}"#, true));
        assert!(ready_report(
            br#"{"marker":false,"webdriver":false}"#,
            false
        ));
        assert!(!ready_report(
            br#"{"marker":true,"webdriver":false}"#,
            false
        ));
        for body in [
            br#"{"marker":false,"webdriver":false}"#.as_slice(),
            br#"{"marker":true,"webdriver":true}"#,
            br#"{"marker":"true","webdriver":false}"#,
            br#"{"marker":true,"webdriver":false,"url":"https://example.test"}"#,
            b"{}",
            b"not json",
        ] {
            assert!(!ready_report(body, true));
        }
    }
}
