use crate::platform;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

#[derive(Serialize, Deserialize, Clone)]
#[serde(rename_all = "camelCase")]
pub struct Check {
    pub id: String,
    pub status: String,
    pub value: String,
    pub detail: String,
    pub fixable: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Scan {
    pub platform: String,
    pub browser: String,
    pub browser_available: bool,
    pub profile_path: String,
    pub checked_at: String,
    pub checks: Vec<Check>,
    pub ip: Option<String>,
    pub location: Option<String>,
    pub latency: Option<u128>,
    pub cli_installed: bool,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Change {
    pub(crate) target: String,
    pointer: String,
    before: Option<Value>,
    after: Option<Value>,
}
#[derive(Serialize, Deserialize, Clone, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Record {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub profile_id: Option<String>,
    pub id: String,
    pub item_id: String,
    pub browser: String,
    pub created_at: String,
    pub status: String,
    pub message: String,
    pub changes: Vec<Change>,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Outcome {
    pub id: String,
    pub success: bool,
    pub message: String,
}
#[derive(Serialize)]
pub struct Readiness {
    pub ready: bool,
    pub message: String,
}
pub fn repair_readiness(root: &Path, browser: &str, ids: &[String]) -> Result<Readiness, String> {
    validate_browser(browser)?;
    if ids.iter().any(|id| id == "timezone") {
        for b in ["chrome", "edge", "firefox"] {
            if let Err(message) = platform::ensure_profile_closed(&profile(root, b)?) {
                return Ok(Readiness {
                    ready: false,
                    message: format!(
                        "{message} 更改系统时区前请退出专用窗口，避免沿用旧的时区缓存。"
                    ),
                });
            }
        }
    }
    if ids
        .iter()
        .any(|id| ["language", "webrtc", "dns", "tracking", "fonts"].contains(&id.as_str()))
    {
        if let Err(message) = platform::ensure_profile_closed(&profile(root, browser)?) {
            return Ok(Readiness {
                ready: false,
                message,
            });
        }
    }
    Ok(Readiness {
        ready: true,
        message: "专用浏览器已退出，可以继续修复。".into(),
    })
}

fn check(id: &str, status: &str, value: impl Into<String>, detail: &str, fixable: bool) -> Check {
    Check {
        id: id.into(),
        status: status.into(),
        value: value.into(),
        detail: detail.into(),
        fixable,
    }
}
fn validate_browser(browser: &str) -> Result<(), String> {
    if ["chrome", "edge", "firefox"].contains(&browser) {
        Ok(())
    } else {
        Err("不支持的浏览器".into())
    }
}
fn profile(root: &Path, browser: &str) -> Result<PathBuf, String> {
    validate_browser(browser)?;
    Ok(root.join(format!("browser-{browser}")))
}
fn target_path(root: &Path, browser: &str, target: &str) -> Result<PathBuf, String> {
    match target {
        "preferences" => Ok(profile(root, browser)?.join("Default/Preferences")),
        "localState" => Ok(profile(root, browser)?.join("Local State")),
        "launcher" => Ok(root.join(platform::launcher_name())),
        _ => Err("备份目标无效".into()),
    }
}
fn read_json(path: &Path) -> Result<Value, String> {
    if !path.exists() {
        return Ok(json!({}));
    }
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 16 * 1024 * 1024 {
        return Err("配置文件超出大小限制".into());
    }
    let bytes = fs::read(path).map_err(|e| e.to_string())?;
    let v: Value = serde_json::from_slice(&bytes)
        .map_err(|_| "配置文件不是有效的 JSON，请先检查文件；未覆盖原文件。".to_string())?;
    if !v.is_object() && path.file_name().and_then(|s| s.to_str()) != Some("history.json") {
        return Err("配置文件结构无效".into());
    }
    Ok(v)
}
pub(crate) fn atomic_write(path: &Path, bytes: &[u8]) -> Result<(), String> {
    let parent = path.parent().ok_or("文件路径无效")?;
    fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    let mut file = tempfile::NamedTempFile::new_in(parent).map_err(|e| e.to_string())?;
    file.write_all(bytes).map_err(|e| e.to_string())?;
    file.as_file().sync_all().map_err(|e| e.to_string())?;
    file.persist(path).map_err(|e| e.to_string())?;
    Ok(())
}
fn write_json(path: &Path, value: &Value) -> Result<(), String> {
    atomic_write(
        path,
        &serde_json::to_vec_pretty(value).map_err(|e| e.to_string())?,
    )
}
pub fn history(root: &Path) -> Result<Vec<Record>, String> {
    let path = root.join("history.json");
    if !path.exists() {
        return Ok(vec![]);
    }
    serde_json::from_value(read_json(&path)?)
        .map_err(|_| "修复记录损坏，请保留 history.json 并检查后重试。".into())
}
pub(crate) fn save_history(root: &Path, records: &[Record]) -> Result<(), String> {
    write_json(
        &root.join("history.json"),
        &serde_json::to_value(records).map_err(|e| e.to_string())?,
    )
}

pub fn scan_local(root: &Path, browser: &str) -> Result<Scan, String> {
    let dir = profile(root, browser)?;
    let available = platform::browser_path(browser).is_ok();
    let prefs = read_json(&dir.join("Default/Preferences"))?;
    let state = read_json(&dir.join("Local State"))?;
    let language = prefs
        .pointer("/intl/accept_languages")
        .and_then(Value::as_str);
    let selected_language = prefs
        .pointer("/intl/selected_languages")
        .and_then(Value::as_str);
    let forced_chinese = contains_chinese(prefs.pointer("/intl/forced_languages"));
    let language_ok =
        language == Some("en-US,en") && selected_language == Some("en-US,en") && !forced_chinese;
    let dnt = prefs.pointer("/enable_do_not_track") == Some(&json!(true));
    let rtc = prefs
        .pointer("/webrtc/ip_handling_policy")
        .and_then(Value::as_str)
        == Some("disable_non_proxied_udp");
    let dns = state
        .pointer("/dns_over_https/mode")
        .and_then(Value::as_str)
        == Some("secure")
        && state
            .pointer("/dns_over_https/templates")
            .and_then(Value::as_str)
            == Some("https://cloudflare-dns.com/dns-query");
    let zone = platform::timezone();
    let offset = platform::offset_minutes();
    let zone_problem = matches!(
        zone.as_deref(),
        Ok("China Standard Time" | "Asia/Shanghai" | "Asia/Urumqi")
    );
    let launcher = root.join(platform::launcher_name());
    let cli_ready =
        fs::read_to_string(launcher).ok().as_deref() == Some(platform::launcher_contents());
    let fonts = platform::font_evidence();
    let cli_installed = platform::cli_installed();
    let proxy = [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ]
    .iter()
    .any(|k| std::env::var(k).is_ok_and(|v| !v.is_empty()));
    let mut checks = vec![
        check("connection", "unknown", "等待网络检测", "检测 claude.ai 和 claude.com 的 HTTPS 响应；不登录、不读取账户。", false),
        check("route", "manual", if proxy { "检测到代理环境变量" } else { "未检测到代理环境变量" }, "本机请求不读取浏览器代理。出口 IP 来自 Cloudflare，仅反映本工具的网络路径；请在专用浏览器里复检 Claude 实际出口。", false),
        check("webrtc", if rtc { "configured" } else { "warning" }, if rtc { "已限制非代理 UDP" } else { "尚未设置隐私策略" }, "读取专用配置中的 WebRTC 策略，配置后仍需网页实测。此设置可能影响网页语音或视频通话。", available),
        check("dns", if dns { "configured" } else { "warning" }, if dns { "Cloudflare · 加密 DNS" } else { "未配置加密 DNS" }, "将专用浏览器设置为严格 DNS over HTTPS。企业策略或代理可能覆盖设置；不等同于已完成 DNS 泄露实测。", available),
        check("language", if language_ok { "configured" } else { "warning" }, language.unwrap_or("跟随浏览器默认设置"), if forced_chinese { "配置文件包含强制中文语言，普通偏好无法覆盖。系统或企业策略还需在浏览器 policy 页面确认。" } else { "同时核验 selected_languages 与 accept_languages。若网页仍显示中文，请确认使用专用窗口、完全退出后重启，并检查策略或扩展。" }, available && !forced_chinese),
        check("timezone", if zone.is_err() { "unknown" } else if zone_problem { "warning" } else { "healthy" }, zone.unwrap_or_else(|_| "读取失败".into()), "读取系统时区标识。修改时区只改变本地时间显示，不会校准电脑时钟；时间准确性请查看独立的系统时钟检测。修改会影响所有应用，可从记录恢复。", cfg!(any(target_os = "windows", target_os = "macos", target_os = "linux"))),
        check("offset", if offset.is_err() { "unknown" } else { "healthy" }, offset.map(offset_label).unwrap_or_else(|_| "读取失败".into()), "这是本机系统时区对应的 UTC 偏移。UTC+8 本身不是异常，也不能判断时钟是否准确；请查看系统时钟检测。", false),
        crate::clock_probe::unknown(),
        check("locale", "manual", "需要在专用浏览器中实测", "Intl 区域设置与首选语言不是同一个值。请打开本地复检页并导入报告；不能凭配置文件判定此项已通过。", false),
        check("cli", if cli_ready { "configured" } else if cli_installed { "warning" } else { "manual" }, if cli_ready { "专用启动器已准备" } else if cli_installed { "已安装 · 尚未准备启动器" } else { "未找到 Claude Code" }, "专用启动器设置 TZ=Asia/Singapore 与英文 locale。仅影响由启动器启动的进程；继承原有网络和 API 配置。", cli_installed),
        check("fonts", "manual", if fonts.is_empty() { "常见系统目录未发现匹配".into() } else { format!("系统目录发现 {} 个中文字体文件", fonts.len()) }, "可打开字体管理，主动选择可卸载的用户字体；系统字体与字体组件保留。此目录检查不是完整安装列表，也不能证明网页检测已通过。", false),
        check("tracking", if dnt { "configured" } else { "warning" }, if dnt { "DNT 已配置 · GPC 待网页复检" } else { "DNT 尚未开启 · GPC 待网页复检" }, "DNT 只是隐私偏好，不保证网站停止跟踪，也不关闭 Claude Code 遥测。GPC 是否支持需网页实测。", available),
        check("webgl", "manual", "需要在专用浏览器中实测", "GPU 型号与图形后端是设备特征，不替换渲染器字符串。", false),
        check("screen", "manual", "需要在专用浏览器中实测", "网页尺寸与像素比例可能受显示缩放和浏览器缩放影响。", false),
        check("networkInfo", "manual", "需要在专用浏览器中实测", "4g 是连接质量估计等级，并不证明使用了蜂窝网络。", false),
        check("plugins", "manual", "需要在专用浏览器中实测", "网页插件常是内置 PDF 查看器；逻辑处理器提示不是物理核心数。", false),
        check("emoji", "manual", "UA 推断与 Emoji 渲染需分开确认", "截图的 Microsoft style 来自 UA 中的 Windows 平台信号，不是 Emoji 像素测试。平台特征不能证明用户所在国家；不替换 UA 或系统字库。", false),
    ];
    if browser == "firefox" {
        for id in ["language", "webrtc", "dns", "tracking", "fonts"] {
            let configured = crate::firefox::configured(&dir, id)?;
            let value = match id {
                "fonts" => {
                    if configured {
                        "字体允许列表已配置 · 待网页复检"
                    } else {
                        "按需限制字体 · 保留电脑字体"
                    }
                }
                "webrtc" => {
                    if configured {
                        "专用 Firefox 已配置关闭 WebRTC"
                    } else {
                        "WebRTC 尚未限制"
                    }
                }
                "tracking" => {
                    if configured {
                        "GPC 已配置 · 待网页复检"
                    } else {
                        "GPC 尚未配置"
                    }
                }
                "language" => {
                    if configured {
                        "en-US,en"
                    } else {
                        "跟随 Firefox 默认设置"
                    }
                }
                _ => {
                    if configured {
                        "Cloudflare · 加密 DNS"
                    } else {
                        "未配置加密 DNS"
                    }
                }
            };
            let detail = match id {
                "fonts" => "仅限制此专用 Firefox 可用的系统字体，保留电脑字体和日常浏览器。部分中文显示可能变化；网页下载字体仍可使用。这是隐藏设置，需重启并网页复检，可从记录恢复。",
                "webrtc" => "关闭此专用 Firefox 的 WebRTC，网页音视频通话可能不可用，可从记录恢复。",
                "tracking" => "Firefox 使用 GPC 表达不出售或分享数据的偏好。新版本可能不提供 DNT；需网页复检。",
                _ => "读取专用 Firefox 的启动偏好与运行偏好；配置写入不等于实际网页检测通过。",
            };
            if let Some(c) = checks.iter_mut().find(|c| c.id == id) {
                *c = check(
                    id,
                    if configured {
                        "configured"
                    } else if id == "fonts" {
                        "manual"
                    } else {
                        "warning"
                    },
                    value,
                    detail,
                    available,
                );
            }
        }
    }
    Ok(Scan {
        platform: if cfg!(target_os = "windows") {
            "Windows"
        } else if cfg!(target_os = "macos") {
            "macOS"
        } else if cfg!(target_os = "linux") {
            "Linux"
        } else {
            "Unsupported"
        }
        .into(),
        browser: browser.into(),
        browser_available: available,
        profile_path: dir.display().to_string(),
        checked_at: chrono::Utc::now().to_rfc3339(),
        checks,
        ip: None,
        location: None,
        latency: None,
        cli_installed,
    })
}

pub async fn scan_network(mut scan: Scan) -> Scan {
    let client = match reqwest::Client::builder()
        .no_proxy()
        .timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::limited(3))
        .user_agent("NodeCloak/0.7 (environment diagnostics)")
        .build()
    {
        Ok(c) => c,
        Err(_) => return scan,
    };
    let start = std::time::Instant::now();
    // HTTP 200 can be an availability error or browser challenge, not a usable page.
    let (a, b, route) = tokio::join!(
        crate::claude_probe::probe(&client, "https://claude.ai"),
        crate::claude_probe::probe(&client, "https://claude.com"),
        crate::claude_probe::trace(&client, "https://claude.ai/cdn-cgi/trace")
    );
    let ok = a.reachable && b.reachable;
    let label = format!(
        "claude.ai {} · claude.com {} · {}",
        a.label,
        b.label,
        route.unwrap_or_else(|| "Claude 同域名出口未取得，需浏览器复检".into())
    );
    scan.latency = if ok {
        Some(start.elapsed().as_millis())
    } else {
        None
    };
    scan.checks[0] = check("connection", if ok { "healthy" } else { "manual" }, label, "本机 TCP HTTPS 页面检查（含 VPN / TUN，不读取浏览器代理）；地区不可用页面和验证挑战不会计为正常。通用 Cloudflare 出口不等于 Claude 分流出口。HTTP/3（QUIC）分流和账号可用性需在浏览器确认。", false);
    if let Ok((body, clock)) = crate::clock_probe::fetch(&client).await {
        if let Some(c) = scan.checks.iter_mut().find(|c| c.id == "clock") {
            *c = clock;
        }
        for line in body.lines() {
            if let Some(ip) = line.strip_prefix("ip=") {
                if ip.parse::<std::net::IpAddr>().is_ok() {
                    scan.ip = Some(ip.into());
                }
            }
            if let Some(loc) = line.strip_prefix("loc=") {
                if loc.len() == 2 && loc.chars().all(|c| c.is_ascii_uppercase()) {
                    scan.location = Some(loc.into());
                }
            }
        }
    }
    scan.checked_at = chrono::Utc::now().to_rfc3339();
    scan
}

fn set_pointer(value: &mut Value, pointer: &str, replacement: Option<Value>) -> Result<(), String> {
    let parts: Vec<&str> = pointer
        .strip_prefix('/')
        .ok_or("配置路径无效")?
        .split('/')
        .collect();
    if parts.is_empty() || parts.iter().any(|p| p.is_empty()) {
        return Err("配置路径无效".into());
    }
    let mut cursor = value;
    for part in &parts[..parts.len() - 1] {
        if !cursor.is_object() {
            return Err("配置字段结构不匹配，未覆盖原值".into());
        }
        cursor = cursor
            .as_object_mut()
            .unwrap()
            .entry(part.to_string())
            .or_insert_with(|| json!({}));
    }
    let object = cursor
        .as_object_mut()
        .ok_or("配置字段结构不匹配，未覆盖原值")?;
    if let Some(v) = replacement {
        object.insert(parts.last().unwrap().to_string(), v);
    } else {
        object.remove(*parts.last().unwrap());
    }
    Ok(())
}

fn contains_chinese(value: Option<&Value>) -> bool {
    let chinese = |text: &str| {
        text.split(',').any(|s| {
            let s = s.trim().to_ascii_lowercase();
            s == "zh" || s.starts_with("zh-")
        })
    };
    match value {
        Some(Value::String(s)) => chinese(s),
        Some(Value::Array(a)) => a.iter().any(|v| v.as_str().is_some_and(chinese)),
        _ => false,
    }
}

fn offset_label(minutes: i32) -> String {
    let east = -minutes;
    let sign = if east >= 0 { "+" } else { "-" };
    let remainder = east.abs() % 60;
    if remainder == 0 {
        format!("UTC{sign}{}", east.abs() / 60)
    } else {
        format!("UTC{sign}{}:{remainder:02}", east.abs() / 60)
    }
}

fn plan(
    root: &Path,
    browser: &str,
    id: &str,
    timezone_target: &str,
) -> Result<Vec<Change>, String> {
    if id == "timezone" {
        for b in ["chrome", "edge", "firefox"] {
            platform::ensure_profile_closed(&profile(root, b)?)?;
        }
        return Ok(vec![Change {
            target: "timezone".into(),
            pointer: String::new(),
            before: Some(json!(platform::timezone()?)),
            after: Some(json!(platform::target_timezone(timezone_target)?)),
        }]);
    }
    if id == "cli" {
        if !platform::cli_installed() {
            return Err("请先安装 Claude Code，再准备专用启动器".into());
        }
        let path = target_path(root, browser, "launcher")?;
        let before = if path.exists() {
            Some(json!(fs::read_to_string(path).map_err(|e| e.to_string())?))
        } else {
            None
        };
        return Ok(vec![Change {
            target: "launcher".into(),
            pointer: String::new(),
            before,
            after: Some(json!(platform::launcher_contents())),
        }]);
    }
    platform::browser_path(browser)?;
    platform::ensure_profile_closed(&profile(root, browser)?)?;
    if browser == "firefox" {
        return crate::firefox::fields(id)?
            .into_iter()
            .map(|(key, value)| {
                Ok(Change {
                    target: "firefoxPrefs".into(),
                    pointer: key.into(),
                    before: Some(crate::firefox::snapshot(&profile(root, browser)?, key)?),
                    after: Some(json!({"user":value,"runtime":value})),
                })
            })
            .collect();
    }
    if id == "language"
        && contains_chinese(
            read_json(&target_path(root, browser, "preferences")?)?
                .pointer("/intl/forced_languages"),
        )
    {
        return Err("配置文件包含强制中文语言，无法用普通偏好覆盖；未修改强制语言字段。".into());
    }
    let (target, fields) = match id {
        "language" => (
            "preferences",
            vec![
                ("/intl/accept_languages", json!("en-US,en")),
                ("/intl/selected_languages", json!("en-US,en")),
            ],
        ),
        "tracking" => ("preferences", vec![("/enable_do_not_track", json!(true))]),
        "webrtc" => (
            "preferences",
            vec![(
                "/webrtc/ip_handling_policy",
                json!("disable_non_proxied_udp"),
            )],
        ),
        "dns" => (
            "localState",
            vec![
                ("/dns_over_https/mode", json!("secure")),
                (
                    "/dns_over_https/templates",
                    json!("https://cloudflare-dns.com/dns-query"),
                ),
            ],
        ),
        _ => return Err("此项目需要手动处理，无法自动修复".into()),
    };
    let current = read_json(&target_path(root, browser, target)?)?;
    Ok(fields
        .into_iter()
        .map(|(pointer, after)| Change {
            target: target.into(),
            pointer: pointer.into(),
            before: current.pointer(pointer).cloned(),
            after: Some(after),
        })
        .collect())
}

fn apply_changes(root: &Path, browser: &str, changes: &[Change], undo: bool) -> Result<(), String> {
    if changes.is_empty() {
        return Err("记录缺少备份".into());
    }
    let target = changes[0].target.as_str();
    if changes.iter().any(|c| c.target != target) {
        return Err("记录包含不一致的目标".into());
    }
    if target == "firefoxPrefs" {
        if browser != "firefox" {
            return Err("Firefox 备份环境不匹配".into());
        }
        let fields = changes
            .iter()
            .map(|c| {
                Ok((
                    c.pointer.clone(),
                    c.before.clone().ok_or("Firefox 备份缺失")?,
                    c.after.clone().ok_or("Firefox 目标缺失")?,
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        return crate::firefox::apply(&profile(root, browser)?, &fields, undo);
    }
    if target == "userFont" {
        if !undo || changes.len() != 1 || changes[0].after != Some(json!({"removed":true})) {
            return Err("字体备份结构无效".into());
        }
        for b in ["chrome", "edge", "firefox"] {
            platform::ensure_profile_closed(&profile(root, b)?)?;
        }
        let snapshot: crate::fonts::Snapshot =
            serde_json::from_value(changes[0].before.clone().ok_or("字体备份缺失")?)
                .map_err(|_| "字体备份格式无效")?;
        return crate::fonts::apply(root, &changes[0].pointer, &snapshot, true);
    }
    // Validate the entire step before writing anything.
    let allowed = |c: &Change| match c.target.as_str() {
        "preferences" => [
            "/intl/accept_languages",
            "/intl/selected_languages",
            "/webrtc/ip_handling_policy",
            "/enable_do_not_track",
        ]
        .contains(&c.pointer.as_str()),
        "localState" => {
            ["/dns_over_https/mode", "/dns_over_https/templates"].contains(&c.pointer.as_str())
        }
        "timezone" | "launcher" => c.pointer.is_empty(),
        _ => false,
    };
    if !changes.iter().all(allowed) {
        return Err("备份字段不在允许列表中".into());
    }
    let desired = |c: &Change| {
        if undo {
            c.before.clone()
        } else {
            c.after.clone()
        }
    };
    if target == "timezone" {
        for b in ["chrome", "edge", "firefox"] {
            platform::ensure_profile_closed(&profile(root, b)?)?;
        }
        let c = &changes[0];
        let current = Some(json!(platform::timezone()?));
        let matches_snapshot = |snapshot: &Option<Value>| {
            current
                .as_ref()
                .and_then(Value::as_str)
                .zip(snapshot.as_ref().and_then(Value::as_str))
                .is_some_and(|(current, expected)| {
                    platform::timezone_names_match(current, expected)
                })
        };
        if undo && !matches_snapshot(&c.after) && !matches_snapshot(&c.before) {
            return Err("系统时区已被其他程序修改。为保留你的新设置，本次未撤销。".into());
        }
        platform::set_timezone(
            desired(c)
                .as_ref()
                .and_then(Value::as_str)
                .ok_or("备份时区无效")?,
        )?;
    } else if target == "launcher" {
        let path = target_path(root, browser, target)?;
        let current = if path.exists() {
            Some(json!(fs::read_to_string(&path).map_err(|e| e.to_string())?))
        } else {
            None
        };
        let c = &changes[0];
        if undo && current != c.after && current != c.before {
            return Err("启动器已被修改，本次未覆盖它。".into());
        }
        if let Some(v) = desired(c) {
            atomic_write(&path, v.as_str().ok_or("启动器备份无效")?.as_bytes())?;
        } else if path.exists() {
            fs::remove_file(&path).map_err(|e| e.to_string())?;
        }
        #[cfg(unix)]
        if path.exists() {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(&path, fs::Permissions::from_mode(0o700))
                .map_err(|e| e.to_string())?;
        }
    } else {
        platform::ensure_profile_closed(&profile(root, browser)?)?;
        let path = target_path(root, browser, target)?;
        let mut value = read_json(&path)?;
        for c in changes {
            let current = value.pointer(&c.pointer).cloned();
            if undo && current != c.after && current != c.before {
                return Err("此设置已被浏览器或其他程序修改，本次未覆盖。请先检查设置。".into());
            }
        }
        for c in changes {
            set_pointer(&mut value, &c.pointer, desired(c))?;
        }
        write_json(&path, &value)?;
        let verified = read_json(&path)?;
        if changes
            .iter()
            .any(|c| verified.pointer(&c.pointer).cloned() != desired(c))
        {
            return Err("配置写入后未通过验证；可从记录恢复".into());
        }
    }
    Ok(())
}

#[cfg(test)]
pub fn repair(
    root: &Path,
    browser: &str,
    ids: &[String],
    consent_timezone: bool,
    timezone_target: &str,
) -> Result<Vec<Outcome>, String> {
    repair_with_font_consent(root, browser, ids, consent_timezone, timezone_target, false)
}
pub fn repair_with_font_consent(
    root: &Path,
    browser: &str,
    ids: &[String],
    consent_timezone: bool,
    timezone_target: &str,
    consent_fonts: bool,
) -> Result<Vec<Outcome>, String> {
    validate_browser(browser)?;
    if ids.iter().any(|id| id == "fonts") && (browser != "firefox" || !consent_fonts) {
        return Err("字体限制仅适用于专用 Firefox，需要明确勾选确认；电脑字体不会删除。".into());
    }
    if ids.is_empty()
        || ids.len() > 7
        || ids.iter().any(|id| {
            ![
                "language", "webrtc", "dns", "timezone", "cli", "tracking", "fonts",
            ]
            .contains(&id.as_str())
        })
    {
        return Err("修复项目无效".into());
    }
    if ids.iter().any(|id| id == "timezone") && !consent_timezone {
        return Err("修改系统时区需要明确勾选确认".into());
    }
    if ids.iter().any(|id| id == "timezone") {
        platform::target_timezone(timezone_target)?;
    }
    let mut records = history(root)?;
    let mut outcomes = vec![];
    for id in ids {
        let changes = match plan(root, browser, id, timezone_target) {
            Ok(c) => c,
            Err(e) => {
                outcomes.push(Outcome {
                    id: id.clone(),
                    success: false,
                    message: e,
                });
                continue;
            }
        };
        if changes.iter().all(|c| c.before == c.after) {
            let verification = if id == "timezone" {
                platform::verify_timezone_target(timezone_target)
            } else {
                Ok("当前已是目标设置，无需修改".into())
            };
            outcomes.push(Outcome {
                id: id.clone(),
                success: verification.is_ok(),
                message: verification.unwrap_or_else(|e| e),
            });
            continue;
        }
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        records.push(Record {
            profile_id: None,
            id: format!("{stamp}-{id}"),
            item_id: id.clone(),
            browser: browser.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "pending".into(),
            message: "已备份原值，等待写入".into(),
            changes,
        });
        // Write-ahead journal: never mutate configuration unless the backup is durable.
        save_history(root, &records)?;
        let index = records.len() - 1;
        let result = apply_changes(root, browser, &records[index].changes, false).and_then(|_| {
            if id == "timezone" {
                platform::verify_timezone_target(timezone_target)
            } else {
                Ok("设置已写入并验证；请启动专用环境后复检".to_string())
            }
        });
        records[index].status = if result.is_ok() { "applied" } else { "failed" }.into();
        records[index].message = result.as_ref().cloned().unwrap_or_else(|e| e.clone());
        save_history(root, &records)?;
        outcomes.push(Outcome {
            id: id.clone(),
            success: result.is_ok(),
            message: records[index].message.clone(),
        });
    }
    Ok(outcomes)
}

pub fn undo(root: &Path, record_id: &str) -> Result<(), String> {
    let mut records = history(root)?;
    let index = records
        .iter()
        .position(|r| r.id == record_id)
        .ok_or("没有找到修复记录")?;
    if records[index].status == "undone" {
        return Err("这次修复已经撤销".into());
    }
    let item = &records[index];
    validate_browser(&item.browser)?;
    if records[index + 1..].iter().any(|r| {
        r.item_id == item.item_id
            && (r.item_id == "timezone"
                || r.item_id == "cli"
                || (r.item_id == "fonts"
                    && r.changes.iter().any(|c| c.target == "userFont")
                    && item.changes.iter().any(|c| c.target == "userFont"))
                || r.browser == item.browser)
            && r.status != "undone"
    }) {
        return Err("请先撤销这个项目较新的修复记录，再恢复较早的记录。".into());
    }
    apply_changes(root, &item.browser, &item.changes, true)?;
    records[index].status = "undone".into();
    records[index].message = "已恢复修复前的设置".into();
    save_history(root, &records)
}

pub fn remove_user_fonts(
    root: &Path,
    browser: &str,
    ids: &[String],
    consent: bool,
) -> Result<Vec<crate::fonts::FontOutcome>, String> {
    validate_browser(browser)?;
    if browser == "firefox" {
        return Err("Firefox 请使用字体可见性限制，保留电脑字体；不通过此入口卸载。".into());
    }
    if !consent {
        return Err("卸载用户字体需要明确确认影响及备份".into());
    }
    if ids.is_empty()
        || ids.len() > 32
        || ids
            .iter()
            .any(|id| id.len() != 64 || !id.chars().all(|c| c.is_ascii_hexdigit()))
    {
        return Err("字体选择无效".into());
    }
    for b in ["chrome", "edge", "firefox"] {
        platform::ensure_profile_closed(&profile(root, b)?)?;
    }
    let mut records = history(root)?;
    let mut results = vec![];
    let unique: std::collections::BTreeSet<_> = ids.iter().collect();
    for id in unique {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let token = format!("{stamp}-fonts-{id}");
        let snapshot = match crate::fonts::prepare(root, id, &token) {
            Ok(s) => s,
            Err(message) => {
                results.push(crate::fonts::FontOutcome {
                    id: id.clone(),
                    label: "所选用户字体".into(),
                    success: false,
                    message,
                });
                continue;
            }
        };
        records.push(Record {
            id: token.clone(),
            profile_id: None,
            item_id: "fonts".into(),
            browser: browser.into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "pending".into(),
            message: format!("已备份 {}，等待卸载", snapshot.label),
            changes: vec![Change {
                target: "userFont".into(),
                pointer: token.clone(),
                before: Some(serde_json::to_value(&snapshot).map_err(|e| e.to_string())?),
                after: Some(json!({"removed":true})),
            }],
        });
        save_history(root, &records)?;
        let result = crate::fonts::apply(root, &token, &snapshot, false);
        let message=result.as_ref().map(|_|format!("已卸载用户字体 {}，备份可恢复。请完全退出浏览器后复检；仍可见时请注销或重启电脑。系统字体保留。",snapshot.label)).unwrap_or_else(|e|e.clone());
        let index = records.len() - 1;
        records[index].status = if result.is_ok() { "applied" } else { "failed" }.into();
        records[index].message = message.clone();
        save_history(root, &records)?;
        results.push(crate::fonts::FontOutcome {
            id: id.clone(),
            label: snapshot.label,
            success: result.is_ok(),
            message,
        });
    }
    Ok(results)
}

pub fn launch_browser(root: &Path, browser: &str, destination: &str) -> Result<(), String> {
    validate_browser(browser)?;
    crate::cloud::browser_session::recover(&profile(root, browser)?)?;
    if browser == "firefox" {
        platform::ensure_profile_closed(&profile(root, browser)?)?;
    }
    if destination == "diagnostics" {
        let dir = profile(root, browser)?;
        let report_page = dir.join("browser-check.html");
        let page = include_str!("../resources/browser-check.html")
            .replace("__CLAUDE_READY_BROWSER__", browser);
        atomic_write(&report_page, page.as_bytes())?;
        let url = reqwest::Url::from_file_path(&report_page).map_err(|_| "复检页路径无效")?;
        return platform::spawn_browser(browser, &dir, url.as_str());
    }
    let url = match destination {
        "privacyDocs" => "https://code.claude.com/docs/en/data-usage".to_owned(),
        "claude" => "https://claude.ai".to_owned(),
        "verify" => "http://claudedone.com/".to_owned(),
        "dns" => {
            if browser == "firefox" {
                "about:preferences#privacy".to_owned()
            } else {
                "chrome://settings/security".to_owned()
            }
        }
        _ => crate::browser_sites::destination_url(root, destination).ok_or("不支持的目标页面或目录已更新，请重试")?,
    };
    let dir = profile(root, browser)?;
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    platform::spawn_browser(browser, &dir, &url)
}

pub fn launch_cli(root: &Path) -> Result<(), String> {
    let path = root.join(platform::launcher_name());
    if fs::read_to_string(&path).ok().as_deref() != Some(platform::launcher_contents()) {
        return Err("请先修复 Claude Code 环境，准备专用启动器。".into());
    }
    platform::launch_cli(&path)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn system_offset_is_read_only_and_not_a_repair_recommendation() {
        let root = tempfile::tempdir().unwrap();
        let scan = scan_local(root.path(), "chrome").unwrap();
        let offset = scan.checks.iter().find(|c| c.id == "offset").unwrap();
        assert!(
            !offset.fixable,
            "An offset is derived from the system timezone"
        );
        assert!(matches!(offset.status.as_str(), "healthy" | "unknown"));
    }
    #[test]
    fn timezone_detection_does_not_claim_the_clock_is_synchronized() {
        let root = tempfile::tempdir().unwrap();
        let scan = scan_local(root.path(), "chrome").unwrap();
        let clock = scan
            .checks
            .iter()
            .find(|c| c.id == "clock")
            .expect("Clock accuracy needs its own check");
        assert_eq!(clock.status, "unknown");
        assert!(!clock.fixable);
        assert!(scan
            .checks
            .iter()
            .find(|c| c.id == "timezone")
            .unwrap()
            .detail
            .contains("不会校准"));
    }
    #[test]
    fn firefox_font_restriction_requires_own_consent_without_modifying_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(repair_with_font_consent(
            dir.path(),
            "firefox",
            &["fonts".into()],
            false,
            "singapore",
            false
        )
        .is_err());
        assert!(repair_with_font_consent(
            dir.path(),
            "edge",
            &["fonts".into()],
            false,
            "singapore",
            true
        )
        .is_err());
        assert!(remove_user_fonts(dir.path(), "firefox", &["a".repeat(64)], true).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn firefox_history_can_restore_its_preferences_without_touching_other_browser() {
        let dir = tempfile::tempdir().unwrap();
        let p = profile(dir.path(), "firefox").unwrap();
        let before = crate::firefox::snapshot(&p, "font.system.whitelist").unwrap();
        let change = Change {
            target: "firefoxPrefs".into(),
            pointer: "font.system.whitelist".into(),
            before: Some(before.clone()),
            after: Some(
                json!({"user":crate::firefox::FONT_LIST,"runtime":crate::firefox::FONT_LIST}),
            ),
        };
        let mut records = vec![Record {
            profile_id: None,
            id: "ff-font-test".into(),
            item_id: "fonts".into(),
            browser: "firefox".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            status: "pending".into(),
            message: "test".into(),
            changes: vec![change],
        }];
        save_history(dir.path(), &records).unwrap();
        apply_changes(dir.path(), "firefox", &records[0].changes, false).unwrap();
        records[0].status = "applied".into();
        save_history(dir.path(), &records).unwrap();
        assert!(scan_local(dir.path(), "firefox")
            .unwrap()
            .checks
            .iter()
            .any(|c| c.id == "fonts" && c.status == "configured"));
        undo(dir.path(), "ff-font-test").unwrap();
        assert_eq!(
            crate::firefox::snapshot(&p, "font.system.whitelist").unwrap(),
            before
        );
        assert_eq!(history(dir.path()).unwrap()[0].status, "undone");
        assert!(!profile(dir.path(), "chrome").unwrap().exists());
        assert!(!profile(dir.path(), "edge").unwrap().exists());
    }
    #[test]
    fn pointer_changes_preserve_other_preferences() {
        let mut v = json!({"intl":{"other":true},"cookies":{"keep":true}});
        set_pointer(&mut v, "/intl/accept_languages", Some(json!("en-US,en"))).unwrap();
        set_pointer(&mut v, "/intl/accept_languages", None).unwrap();
        assert_eq!(v, json!({"intl":{"other":true},"cookies":{"keep":true}}));
    }
    #[test]
    fn invalid_structure_is_not_overwritten() {
        let mut v = json!({"intl": "unexpected"});
        assert!(set_pointer(&mut v, "/intl/accept_languages", Some(json!("en"))).is_err());
        assert_eq!(v, json!({"intl": "unexpected"}));
    }
    #[test]
    fn arbitrary_paths_and_ids_are_rejected() {
        let root = Path::new("test-root");
        assert!(profile(root, "../escape").is_err());
        assert!(target_path(root, "chrome", "../../secret").is_err());
        assert!(repair(root, "chrome", &["fonts".into()], false, "singapore").is_err());
        assert!(repair(root, "chrome", &["timezone".into()], false, "utc").is_err());
        assert!(repair(root, "chrome", &["timezone".into()], true, "arbitrary").is_err());
    }
    #[test]
    fn undo_preserves_unrelated_fields_and_detects_conflicts() {
        let dir = tempfile::tempdir().unwrap();
        let path = target_path(dir.path(), "chrome", "localState").unwrap();
        write_json(
            &path,
            &json!({"unrelated": 42, "dns_over_https":{"mode":"secure"}}),
        )
        .unwrap();
        let changes = vec![Change {
            target: "localState".into(),
            pointer: "/dns_over_https/mode".into(),
            before: None,
            after: Some(json!("secure")),
        }];
        apply_changes(dir.path(), "chrome", &changes, true).unwrap();
        assert_eq!(read_json(&path).unwrap()["unrelated"], json!(42));
        assert!(read_json(&path)
            .unwrap()
            .pointer("/dns_over_https/mode")
            .is_none());
        write_json(&path, &json!({"dns_over_https":{"mode":"automatic"}})).unwrap();
        assert!(apply_changes(dir.path(), "chrome", &changes, true).is_err());
        assert_eq!(
            read_json(&path).unwrap().pointer("/dns_over_https/mode"),
            Some(&json!("automatic"))
        );
    }
    #[test]
    fn malformed_json_is_not_replaced() {
        let dir = tempfile::tempdir().unwrap();
        let p = dir.path().join("Preferences");
        fs::write(&p, b"broken").unwrap();
        assert!(read_json(&p).is_err());
        assert_eq!(fs::read(p).unwrap(), b"broken");
    }
    #[test]
    fn repair_journals_backups_and_restores_preferences() {
        // Only app-owned configuration under an automatically created temp directory is changed.
        if platform::browser_path("edge").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let root = dir.path();
        let prefs = target_path(root, "edge", "preferences").unwrap();
        write_json(
            &prefs,
            &json!({"intl":{"accept_languages":"zh-CN"},"unrelated":true}),
        )
        .unwrap();
        let ids = vec![
            "language".into(),
            "webrtc".into(),
            "dns".into(),
            "tracking".into(),
        ];
        let outcomes = repair(root, "edge", &ids, false, "singapore").unwrap();
        assert!(outcomes.iter().all(|o| o.success));
        let records = history(root).unwrap();
        assert_eq!(records.len(), 4);
        assert!(records.iter().all(|r| r.status == "applied"));
        assert_eq!(records[0].changes[0].before, Some(json!("zh-CN")));
        let mut current = read_json(&prefs).unwrap();
        assert_eq!(
            current.pointer("/intl/selected_languages"),
            Some(&json!("en-US,en"))
        );
        assert_eq!(current.pointer("/enable_do_not_track"), Some(&json!(true)));
        current["new_unrelated_preference"] = json!("keep me");
        write_json(&prefs, &current).unwrap();
        for r in records.iter().rev() {
            undo(root, &r.id).unwrap();
        }
        let restored = read_json(&prefs).unwrap();
        assert_eq!(
            restored.pointer("/intl/accept_languages"),
            Some(&json!("zh-CN"))
        );
        assert_eq!(restored["new_unrelated_preference"], json!("keep me"));
        assert!(restored.pointer("/webrtc/ip_handling_policy").is_none());
        assert!(restored.pointer("/intl/selected_languages").is_none());
        assert!(restored.pointer("/enable_do_not_track").is_none());
        assert!(history(root).unwrap().iter().all(|r| r.status == "undone"));
    }
    #[test]
    fn readiness_blocks_a_locked_profile_without_writing_settings() {
        let dir = tempfile::tempdir().unwrap();
        let dedicated = profile(dir.path(), "edge").unwrap();
        fs::create_dir_all(&dedicated).unwrap();
        fs::write(dedicated.join("SingletonLock"), "test lock").unwrap();
        // File existence is not native lock ownership on macOS.
        let lock_owner=fs::OpenOptions::new().read(true).write(true).open(dedicated.join("SingletonLock")).unwrap();
        fs2::FileExt::lock_exclusive(&lock_owner).unwrap();
        let result = repair_readiness(dir.path(), "edge", &["dns".into(), "cli".into()]).unwrap();
        assert!(!result.ready);
        assert!(history(dir.path()).unwrap().is_empty());
        assert!(!dedicated.join("Default/Preferences").exists());
        assert!(
            repair_readiness(dir.path(), "edge", &["cli".into()])
                .unwrap()
                .ready
        );
        assert!(
            !repair_readiness(dir.path(), "chrome", &["timezone".into()])
                .unwrap()
                .ready
        );
    }
    #[test]
    fn font_removal_requires_consent_and_valid_ids_before_any_write() {
        let dir = tempfile::tempdir().unwrap();
        assert!(remove_user_fonts(dir.path(), "edge", &["a".repeat(64)], false).is_err());
        assert!(remove_user_fonts(dir.path(), "edge", &["../simsun.ttc".into()], true).is_err());
        assert_eq!(fs::read_dir(dir.path()).unwrap().count(), 0);
    }
    #[test]
    fn timezones_and_offsets_are_independent() {
        assert_ne!(
            platform::target_timezone("utc").unwrap(),
            platform::target_timezone("singapore").unwrap()
        );
        assert_eq!(offset_label(-480), "UTC+8");
        assert_eq!(offset_label(0), "UTC+0");
        assert_eq!(offset_label(-330), "UTC+5:30");
        assert_eq!(offset_label(210), "UTC-3:30");
    }
    #[test]
    fn forced_languages_are_reported_and_not_overwritten() {
        assert!(contains_chinese(Some(&json!(["en-US", "zh-CN"]))));
        assert!(!contains_chinese(Some(&json!(["en-US", "en"]))));
        if platform::browser_path("edge").is_err() {
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let path = target_path(dir.path(), "edge", "preferences").unwrap();
        let original = json!({"intl":{"accept_languages":"zh-CN","selected_languages":"zh-CN","forced_languages":["zh-CN"]}});
        write_json(&path, &original).unwrap();
        let scan = scan_local(dir.path(), "edge").unwrap();
        assert!(
            !scan
                .checks
                .iter()
                .find(|c| c.id == "language")
                .unwrap()
                .fixable
        );
        let results = repair(dir.path(), "edge", &["language".into()], false, "singapore").unwrap();
        assert!(!results[0].success);
        assert_eq!(read_json(&path).unwrap(), original);
        assert!(history(dir.path()).unwrap().is_empty());
    }
}
