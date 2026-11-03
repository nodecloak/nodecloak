//! Portable login state. Browser APIs run against a closed, locally staged profile.
//! Network requests are fulfilled with an empty document, never with website scripts.
use crate::{engine, platform, profiles};
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::{BTreeSet, HashMap, HashSet},
    net::TcpStream,
    path::{Path, PathBuf},
    process::{Child, Stdio},
    time::{Duration, Instant},
};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};

pub(super) const PARTITIONED_STORAGE_ERROR: &str =
    "此副本包含尚未支持迁移的分区网站存储，已保留本机数据";
const FIREFOX_WEB_CONTEXT_ERROR: &str =
    "Firefox 隔离网站存储页面受特殊内容限制，已保留本机数据";

pub fn origin(v: &str) -> Result<String, String> {
    let u = reqwest::Url::parse(v).map_err(|_| "网站来源无效")?;
    if !matches!(u.scheme(), "http" | "https")
        || u.host_str().is_none()
        || !u.username().is_empty()
        || u.password().is_some()
        || !matches!(u.path(), "" | "/")
        || u.query().is_some()
        || u.fragment().is_some()
    {
        return Err("网站来源无效".into());
    }
    Ok(u.origin().ascii_serialization())
}
fn canonical_storage(state: &mut Value) -> Result<(), String> {
    let local = state["local"].as_array_mut().ok_or("网站存储无效")?;
    local.sort_by(|a, b| a[0].as_str().cmp(&b[0].as_str()));
    let databases = state["databases"].as_array_mut().ok_or("网站存储无效")?;
    databases.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
    for db in databases {
        let stores = db["stores"].as_array_mut().ok_or("网站存储无效")?;
        stores.sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        for store in stores {
            store["indexes"]
                .as_array_mut()
                .ok_or("网站存储无效")?
                .sort_by(|a, b| a["name"].as_str().cmp(&b["name"].as_str()));
        }
    }
    Ok(())
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Cookie {
    pub name: String,
    pub value: String,
    pub domain: String,
    pub path: String,
    pub secure: bool,
    pub http_only: bool,
    pub same_site: String,
    pub expires: f64,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<Partition>,
    #[serde(
        default = "default_true",
        skip_serializing_if = "is_true",
        rename = "partitionedAttribute"
    )]
    pub partitioned_attribute: bool,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "firefoxPartitionKey"
    )]
    pub firefox_partition_key: Option<String>,
}
fn default_true() -> bool {
    true
}
fn is_true(v: &bool) -> bool {
    *v
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Partition {
    pub top_level_site: String,
    pub has_cross_site_ancestor: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Site {
    pub origin: String,
    pub state: Value,
    #[serde(
        default,
        skip_serializing_if = "Option::is_none",
        rename = "partitionKey"
    )]
    pub partition_key: Option<Partition>,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Eq, PartialOrd, Ord)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct SiteKey {
    pub origin: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub partition_key: Option<Partition>,
}
impl Site {
    fn key(&self) -> SiteKey {
        SiteKey {
            origin: self.origin.clone(),
            partition_key: self.partition_key.clone(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Snapshot {
    pub schema: u8,
    pub cookies: Vec<Cookie>,
    pub origins: Vec<Site>,
}
impl Snapshot {
    fn from_parts(cookies: Vec<Cookie>, origins: Vec<Site>) -> Result<Self, String> {
        let schema = if cookies
            .iter()
            .any(|c| c.partition_key.is_some() || c.same_site == "Default")
            || origins.iter().any(|s| s.partition_key.is_some())
        {
            2
        } else {
            1
        };
        Self::parse(json!({"schema":schema,"cookies":cookies,"origins":origins}))
    }
    pub fn empty() -> Self {
        Self {
            schema: 1,
            cookies: Vec::new(),
            origins: Vec::new(),
        }
    }
    pub fn parse(v: Value) -> Result<Self, String> {
        if serde_json::to_vec(&v).map_err(|_| "登录数据无效")?.len() > super::session_limits::SNAPSHOT_BYTES {
            return Err(super::session_limits::TOO_LARGE.into());
        };
        let mut s: Self = serde_json::from_value(v).map_err(|_| "登录数据格式无效")?;
        if ![1, 2].contains(&s.schema) || s.cookies.len() > 10000 || s.origins.len() > 500 {
            return Err("登录数据版本或数量超过限制".into());
        }
        for c in &s.cookies {
            let domain = c.domain.strip_prefix('.').unwrap_or(&c.domain);
            if domain.is_empty()
                || domain.len() > 253
                || domain
                    .bytes()
                    .any(|b| !b.is_ascii_alphanumeric() && !b".-:".contains(&b))
                || c.name.len() > 4096
                || c.value.len() > 16384
                || c.path.len() > 4096
                || !c.path.starts_with('/')
                || !c.expires.is_finite()
                || !(matches!(c.same_site.as_str(), "Lax" | "Strict" | "None")
                    || s.schema >= 2 && c.same_site == "Default")
            {
                return Err("Cookie 范围或内容无效".into());
            };
            if let Some(p) = &c.partition_key {
                if origin(&p.top_level_site)? != p.top_level_site {
                    return Err("Cookie 分区无效".into());
                }
            }
            if !c.partitioned_attribute && (s.schema < 2 || c.partition_key.is_none()) {
                return Err("Cookie 分区属性无效".into());
            }
            if let Some(native) = &c.firefox_partition_key {
                if s.schema < 2 || native.len() > 4096 {
                    return Err("Cookie 原生分区无效".into());
                }
                let raw = super::session_storage::firefox_partition(native)?
                    .ok_or("Cookie 原生分区无效")?;
                let p = c.partition_key.as_ref().ok_or("Cookie 原生分区无效")?;
                if raw.top_level_site != p.top_level_site {
                    return Err("Cookie 原生分区与快照不一致".into());
                }
                let scheme = reqwest::Url::parse(&p.top_level_site)
                    .map_err(|_| "Cookie 分区无效")?
                    .scheme()
                    .to_owned();
                let cross = !super::session_storage::same_site(
                    &format!("{scheme}://{}", c.domain.trim_start_matches('.')),
                    &p.top_level_site,
                );
                if raw.has_cross_site_ancestor && cross {
                    return Err(PARTITIONED_STORAGE_ERROR.into());
                }
                if (raw.has_cross_site_ancestor || cross) != p.has_cross_site_ancestor {
                    return Err("Cookie 原生分区与快照不一致".into());
                }
            }
        }
        let mut seen = BTreeSet::new();
        for site in &s.origins {
            if origin(&site.origin)? != site.origin
                || !seen.insert(site.key())
                || !site.state.is_object()
            {
                return Err("网站存储来源无效".into());
            }
            if let Some(p) = &site.partition_key {
                if s.schema < 2
                    || origin(&p.top_level_site)? != p.top_level_site
                    || !p.has_cross_site_ancestor
                {
                    return Err("网站存储分区无效".into());
                }
            }
        }
        s.cookies.sort_by(|a, b| {
            (&a.domain, &a.path, &a.name, &a.partition_key).cmp(&(
                &b.domain,
                &b.path,
                &b.name,
                &b.partition_key,
            ))
        });
        s.origins.sort_by_key(Site::key);
        for site in &mut s.origins {
            canonical_storage(&mut site.state)?;
        }
        Ok(s)
    }
    pub fn bytes(&self) -> Result<zeroize::Zeroizing<Vec<u8>>, String> {
        Ok(zeroize::Zeroizing::new(
            serde_json::to_vec(self).map_err(|_| "无法读取登录数据")?,
        ))
    }
}
const PROFILE_IO_ERROR: &str = "登录同步暂时无法读取本机浏览器数据";
pub(super) fn profile_io_error(relative: &Path, operation: &'static str, error: &std::io::Error) -> String {
    // Directory names can contain website origins, extension ids and credentials.
    // Keep fixed storage categories and OS codes; never persist a path or error body.
    let area = if relative.as_os_str().is_empty() { "profile" }
        else if relative.starts_with("Default/IndexedDB") { "indexed_db" }
        else if relative.starts_with("Default/Local Storage") { "local_storage" }
        else if relative.starts_with("storage") { "website_storage" }
        else if relative.starts_with("Default/Network") || relative.starts_with("Default/Cookies")
            || relative.starts_with("cookies.sqlite") { "cookies" }
        else { "browser_files" };
    format!("{PROFILE_IO_ERROR}（operation={operation}, area={area}, kind={:?}, os_code={:?}；本机数据已保留）", error.kind(), error.raw_os_error())
}
fn retryable_profile_io(error: &str) -> bool {
    error.starts_with(PROFILE_IO_ERROR) && ["os_code=Some(32)", "os_code=Some(33)",
        "kind=NotFound", "kind=Interrupted", "kind=WouldBlock"].iter().any(|code| error.contains(code))
}
fn copy_tree(src: &Path, dst: &Path, budget: &mut u64) -> Result<(), String> {
    copy_tree_inner(src, dst, Path::new(""), false, budget)
}
fn copy_tree_inner(src: &Path, dst: &Path, relative: &Path, chromium: bool, budget: &mut u64) -> Result<(), String> {
    profiles::guard_tree(src)?;
    std::fs::create_dir_all(dst).map_err(|e| profile_io_error(relative, "create_temporary_directory", &e))?;
    for entry in std::fs::read_dir(src).map_err(|e| profile_io_error(relative, "read_directory", &e))? {
        let e = entry.map_err(|e| profile_io_error(relative, "read_directory_entry", &e))?;
        let name = e.file_name();
        let n = name.to_string_lossy();
        if matches!(
            n.as_ref(),
            "parent.lock"
                | "lock"
                | ".parentlock"
                | "DevToolsActivePort"
                | "WebDriverBiDiServer.json"
                | "Cache"
                | "Code Cache"
                | "GPUCache"
                | "cache2"
                | "startupCache"
                | "ShaderCache"
                | "GrShaderCache"
                | "DawnCache"
                | "Crashpad"
        ) || n.starts_with("Singleton")
        {
            continue;
        }
        // Regenerable Chromium metrics/GPU caches must not block login capture.
        // Scope these names to browser-owned locations, not arbitrary website data.
        if chromium && (relative.as_os_str().is_empty() || relative == Path::new("Default"))
            && matches!(n.as_ref(), "BrowserMetrics" | "DawnGraphiteCache" | "DawnWebGPUCache") {
            continue;
        }
        let from = e.path();
        profiles::guard_tree(&from)?;
        let child = relative.join(&name);
        let to = dst.join(name);
        let meta = std::fs::symlink_metadata(&from).map_err(|e| profile_io_error(&child, "read_file_metadata", &e))?;
        if meta.is_dir() {
            copy_tree_inner(&from, &to, &child, chromium, budget)?
        } else if meta.is_file() {
            *budget = budget.checked_add(meta.len()).ok_or("副本过大")?;
            if *budget > 2 << 30 {
                return Err("副本数据超过 2 GiB，请先清理缓存再同步".into());
            };
            std::fs::copy(from, to).map_err(|e| profile_io_error(&child, "copy_file", &e))?;
        } else {
            return Err("副本包含不支持的文件".into());
        }
    }
    Ok(())
}
fn disk_origins(browser: &str, path: &Path) -> Result<BTreeSet<SiteKey>, String> {
    let mut out = BTreeSet::new();
    if browser == "firefox" {
        let dir = path.join("storage/default");
        if dir.exists() {
            for e in std::fs::read_dir(dir).map_err(|_| "无法读取网站存储")? {
                let n = e
                    .map_err(|_| "网站存储无效")?
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                if !n.starts_with("https+++") && !n.starts_with("http+++") {
                    continue;
                };
                out.insert(super::session_storage::firefox_key(&n)?);
            }
        }
    } else {
        super::session_storage::quota_keys(path, &mut out)?;
        let dir = path.join("Default/IndexedDB");
        if dir.exists() {
            for e in std::fs::read_dir(dir).map_err(|_| "无法读取网站存储")? {
                let n = e
                    .map_err(|_| "网站存储无效")?
                    .file_name()
                    .to_string_lossy()
                    .into_owned();
                let key = n.split(".indexeddb.").next().unwrap_or("");
                if let Some((scheme, rest)) = key.split_once('_') {
                    if matches!(scheme, "http" | "https") {
                        if let Some((host, port)) = rest.rsplit_once('_') {
                            let u = if port == "0" {
                                format!("{scheme}://{host}")
                            } else {
                                format!("{scheme}://{host}:{port}")
                            };
                            out.insert(SiteKey {
                                origin: origin(&u)?,
                                partition_key: None,
                            });
                        }
                    }
                }
            }
        }
        let dir = path.join("Default/Local Storage/leveldb");
        if dir.join("CURRENT").exists() {
            use rusty_leveldb::{LdbIterator, Options, DB};
            let mut opts = Options::default();
            opts.create_if_missing = false;
            let mut db = DB::open(&dir, opts).map_err(|_| "无法枚举网站本地存储")?;
            let mut iter = db.new_iter().map_err(|_| "无法读取网站本地存储")?;
            let mut count = 0;
            while let Some((key, _)) = iter.next() {
                count += 1;
                if count > 100000 {
                    return Err("网站存储记录过多".into());
                };
                let raw = if key.starts_with(b"META:") {
                    &key[5..]
                } else if key.starts_with(b"_") {
                    key[1..].split(|b| *b == 0).next().unwrap_or(&[])
                } else {
                    continue;
                };
                if let Ok(s) = std::str::from_utf8(raw) {
                    if s.starts_with("https://") || s.starts_with("http://") {
                        if let Some(key) = super::session_storage::chromium_web_key(s)? {
                            out.insert(key);
                        }
                    }
                }
            }
        }
    };
    if out.len() > 500 {
        return Err("副本网站数量超过限制".into());
    };
    Ok(out)
}
fn isolated_transport_error(method: &str, error: &tungstenite::Error) -> String {
    // Protocol bodies, URLs and browser messages can contain login secrets.
    let detail = match error {
        tungstenite::Error::Io(error) => format!("I/O kind={:?}, os_code={:?}", error.kind(), error.raw_os_error()),
        tungstenite::Error::Protocol(_) => "WebSocket protocol error".into(),
        tungstenite::Error::Http(response) => format!("HTTP status={}", response.status().as_u16()),
        tungstenite::Error::ConnectionClosed | tungstenite::Error::AlreadyClosed => "WebSocket closed".into(),
        _ => "WebSocket transport error".into(),
    };
    format!("隔离浏览器连接中断（操作 {method}，{detail}）")
}
pub(super) fn capture_transport_failure(error: &str) -> bool {
    ["隔离浏览器连接中断", "隔离浏览器连接已关闭", "无法启动隔离浏览器", "无法连接隔离浏览器", "隔离浏览器未正常关闭，取消同步"].contains(&error)
        || error.starts_with("隔离浏览器连接中断（")
        || error.starts_with("隔离浏览器连接已关闭（")
        || error.starts_with("浏览器登录数据操作 ") && error.ends_with(" 超时")
}
pub(super) fn capture_local_launch_failure(error: &str) -> bool {
    capture_transport_failure(error) || error.starts_with(PROFILE_IO_ERROR) || error == FIREFOX_WEB_CONTEXT_ERROR
        || error == "浏览器拒绝登录数据操作 browsingContext.navigate"
        || error.starts_with("浏览器拒绝登录数据操作 browsingContext.navigate（")
        || error == "无法进入隔离网站存储" || error.starts_with("无法进入隔离网站存储（")
        || ["网站存储超过同步容量限制，本机数据已保留", "网站存储包含不可迁移的值，保留本机数据",
            "隔离网站存储被浏览器阻止，本机数据已保留"].contains(&error)
}
fn codec_result(value: Value) -> Result<Value, String> {
    if value["ok"] == true && value.get("value").is_some() { return Ok(value["value"].clone()); }
    Err(match value["code"].as_str() {
        Some("NC_LIMIT") => "网站存储超过同步容量限制，本机数据已保留",
        Some("non-exportable CryptoKey" | "NC_UNSUPPORTED_VALUE") => "网站存储包含不可迁移的值，保留本机数据",
        Some("NC_STORAGE_BLOCKED") => "隔离网站存储被浏览器阻止，本机数据已保留",
        _ => "网站存储处理失败，保留本机数据",
    }.into())
}
struct Browser {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    child: Child,
    next: u64,
    firefox: bool,
    target: String,
    session: Option<String>,
    closed: bool,
    path: PathBuf,
    waiting: HashSet<u64>,
    replies: HashMap<u64, Value>,
    frames: HashMap<String, String>,
    eval_target: Option<String>,
    eval_context: Option<(String, i64)>,
}
#[cfg(test)]
#[path = "session_close_checks.rs"]
mod close_checks;
impl Browser {
    fn launch(browser: &str, path: &Path) -> Result<Self, String> {
        let exe = platform::browser_path(browser)?;
        let mut cmd = platform::command(&exe.to_string_lossy());
        cmd.stdout(Stdio::null())
            .stderr(Stdio::null())
            .stdin(Stdio::null());
        if browser == "firefox" {
            cmd.args(["--headless", "--no-remote", "--profile"])
                .arg(path)
                .args(["--remote-debugging-port", "0", "about:blank"]);
        } else {
            cmd.arg("--headless=new");
            cmd.arg(format!("--user-data-dir={}", browser_launch_path(path)))
                .args([
                    "--remote-debugging-address=127.0.0.1",
                    "--remote-debugging-port=0",
                    "--no-first-run",
                    "--no-default-browser-check",
                    "--disable-background-networking",
                    "--disable-extensions",
                    "--disable-sync",
                    "--proxy-server=http://127.0.0.1:9",
                    "--proxy-bypass-list=<-loopback>",
                    "about:blank",
                ]);
        }
        let mut child = cmd.spawn().map_err(|_| "无法启动隔离浏览器")?;
        let endpoint = match crate::browser_control::endpoint(browser, path) {
            Ok(v) => v,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = platform::force_close_profile(path);
                return Err("无法连接隔离浏览器".into());
            }
        };
        let (mut socket, _) = match tungstenite::connect(endpoint.as_str()) {
            Ok(v) => v,
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                let _ = platform::force_close_profile(path);
                return Err("无法连接隔离浏览器".into());
            }
        };
        socket.set_config(|config|{
            config.max_message_size=Some(super::session_limits::BROWSER_BYTES);
            config.max_frame_size=Some(super::session_limits::BROWSER_BYTES);
        });
        if let MaybeTlsStream::Plain(s) = socket.get_mut() {
            s.set_read_timeout(Some(Duration::from_millis(100)))
                .map_err(|_| "无法设置超时")?;
            s.set_write_timeout(Some(Duration::from_secs(30)))
                .map_err(|_| "无法设置超时")?;
        }
        let mut b = Self {
            socket,
            child,
            next: 0,
            firefox: browser == "firefox",
            target: String::new(),
            session: None,
            closed: false,
            path: path.to_owned(),
            waiting: HashSet::new(),
            replies: HashMap::new(),
            frames: HashMap::new(),
            eval_target: None,
            eval_context: None,
        };
        if b.firefox {
            b.call(
                "session.new",
                json!({"capabilities":{"alwaysMatch":{"acceptInsecureCerts":false}}}),
                false,
            )?;
            let v = b.call(
                "browsingContext.create",
                json!({"type":"tab","background":true}),
                false,
            )?;
            b.target = v["context"].as_str().ok_or("无法创建隔离页面")?.into();
            b.call(
                "session.subscribe",
                json!({"events":["network.beforeRequestSent"]}),
                false,
            )?;
            b.call(
                "network.addIntercept",
                json!({"phases":["beforeRequestSent"]}),
                false,
            )?;
        } else {
            let v = b.call("Target.createTarget", json!({"url":"about:blank"}), false)?;
            b.target = v["targetId"].as_str().ok_or("无法创建隔离页面")?.into();
            let v = b.call(
                "Target.attachToTarget",
                json!({"targetId":b.target,"flatten":true}),
                false,
            )?;
            b.session = Some(v["sessionId"].as_str().ok_or("无法连接隔离页面")?.into());
            b.call(
                "Network.setBypassServiceWorker",
                json!({"bypass":true}),
                true,
            )?;
            b.call("Fetch.enable", json!({"patterns":[{"urlPattern":"*"}]}), true)?;
            b.call("Target.setAutoAttach",json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,"filter":[{"type":"iframe","exclude":false},{"exclude":true}]}),true)?;
        }
        Ok(b)
    }
    fn send(&mut self, method: &str, params: Value, target: bool) -> Result<u64, String> {
        let session = if target { self.session.clone() } else { None };
        self.send_to(method, params, session.as_deref())
    }
    fn send_to(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<u64, String> {
        self.next += 1;
        let mut v = json!({"id":self.next,"method":method,"params":params});
        if let Some(s) = session {
            v["sessionId"] = json!(s)
        }
        self.socket
            .send(Message::Text(v.to_string().into()))
            .map_err(|error| isolated_transport_error(method, &error))?;
        Ok(self.next)
    }
    fn call(&mut self, method: &str, params: Value, target: bool) -> Result<Value, String> {
        let session = if target { self.session.clone() } else { None };
        self.call_to(method, params, session.as_deref())
    }
    fn call_to(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        let id = self.send_to(method, params, session)?;
        self.waiting.insert(id);
        let result = self.wait_for(id, method);
        self.waiting.remove(&id);
        result
    }
    fn wait_for(&mut self, id: u64, method: &str) -> Result<Value, String> {
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(120) {
            if let Some(v) = self.replies.remove(&id) {
                return Self::result(v, method);
            }
            match self.socket.read() {
                Ok(Message::Text(text)) => {
                    if text.len() > super::session_limits::BROWSER_BYTES {
                        return Err("浏览器响应过大".into());
                    };
                    let v: Value = serde_json::from_str(&text).map_err(|_| "浏览器响应无效")?;
                    if v["id"] == id {
                        return Self::result(v, method);
                    }
                    if let Some(other) = v["id"].as_u64() {
                        if self.waiting.contains(&other) {
                            self.replies.insert(other, v);
                        }
                        continue;
                    }
                    if v["method"] == "Target.attachedToTarget" {
                        let s = v["params"]["sessionId"]
                            .as_str()
                            .ok_or("分区页面会话无效")?
                            .to_owned();
                        let target = v["params"]["targetInfo"]["targetId"]
                            .as_str()
                            .ok_or("分区页面无效")?
                            .to_owned();
                        self.frames.insert(target, s.clone());
                        self.call_to(
                            "Network.setBypassServiceWorker",
                            json!({"bypass":true}),
                            Some(&s),
                        )?;
                        self.call_to(
                            "Fetch.enable",
                            json!({"patterns":[{"urlPattern":"*"}]}),
                            Some(&s),
                        )?;
                        self.call_to("Target.setAutoAttach",json!({"autoAttach":true,"waitForDebuggerOnStart":true,"flatten":true,"filter":[{"type":"iframe","exclude":false},{"exclude":true}]}),Some(&s))?;
                        self.call_to("Runtime.runIfWaitingForDebugger", json!({}), Some(&s))?;
                    }
                    if v["method"] == "Target.detachedFromTarget" {
                        if let Some(target) = v["params"]["targetId"].as_str() {
                            self.frames.remove(target);
                        }
                    }
                    if v["method"] == "Fetch.requestPaused" {
                        self.send_to("Fetch.fulfillRequest",json!({"requestId":v["params"]["requestId"],"responseCode":200,"responseHeaders":[{"name":"Content-Type","value":"text/html"},{"name":"Cache-Control","value":"no-store"}],"body":B64.encode(b"<!doctype html><title>NodeCloak sync</title>")}),v["sessionId"].as_str())?;
                    }
                    if v["method"] == "network.beforeRequestSent"
                        && v["params"]["isBlocked"] == true
                    {
                        self.send("network.provideResponse",json!({"request":v["params"]["request"]["request"],"statusCode":200,"headers":[{"name":"Content-Type","value":{"type":"string","value":"text/html"}}],"body":{"type":"base64","value":B64.encode(b"<!doctype html><title>NodeCloak sync</title>")}}),false)?;
                    }
                }
                Ok(Message::Close(frame)) => return Err(format!("隔离浏览器连接已关闭（操作 {method}，code={}）", frame.map(|f| u16::from(f.code)).unwrap_or(1005))),
                Ok(_) => {}
                Err(tungstenite::Error::Io(e))
                    if matches!(
                        e.kind(),
                        std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                    ) => {}
                Err(error) => return Err(isolated_transport_error(method, &error)),
            }
        }
        Err(format!("浏览器登录数据操作 {method} 超时"))
    }
    fn result(v: Value, method: &str) -> Result<Value, String> {
        if v.get("error").is_some() || v["type"] == "error" {
            if v["error"] == "unsupported operation" && v["message"].as_str()
                .is_some_and(|m|m.to_ascii_lowercase().contains("system access")) {
                return Err(FIREFOX_WEB_CONTEXT_ERROR.into());
            }
            // Never include browser messages or URLs: they may contain login data.
            let codes=["unknown error","invalid argument","no such frame","no such window",
                "no such context","unsupported operation","unknown command","invalid session id"];
            let code=v["error"].as_str().filter(|code|codes.contains(code)).unwrap_or("other");
            let message=v["message"].as_str().unwrap_or("");
            let reasons=["NS_ERROR_ABORT","NS_BINDING_ABORTED","NS_ERROR_UNKNOWN_HOST",
                "NS_ERROR_CONNECTION_REFUSED","NS_ERROR_PROXY_CONNECTION_REFUSED",
                "NS_ERROR_MALFORMED_URI","NS_ERROR_DOM_BAD_URI","NS_ERROR_DOM_SECURITY_ERR",
                "NS_ERROR_NET_INTERRUPT","NS_ERROR_NET_RESET","NS_ERROR_OFFLINE"];
            let reason=reasons.iter().find(|reason|message.contains(**reason));
            return Err(format!("浏览器拒绝登录数据操作 {method}（code={code}{}）",
                reason.map(|reason|format!("，reason={reason}")).unwrap_or_default()));
        }
        Ok(v["result"].clone())
    }
    fn eval(&mut self, expression: &str) -> Result<Value, String> {
        if self.firefox {
            let context = self.eval_target.as_ref().unwrap_or(&self.target).clone();
            let v=self.call("script.evaluate",json!({"expression":format!("(async()=>JSON.stringify(await ({expression})))()"),"target":{"context":context},"awaitPromise":true}),false)?;
            if v["type"] == "exception" {
                return Err("网站存储包含不可迁移的值，保留本机数据".into());
            };
            serde_json::from_str(v["result"]["value"].as_str().ok_or("网站存储响应无效")?)
                .map_err(|_| "网站存储响应无效".into())
        } else {
            let mut params =
                json!({"expression":expression,"awaitPromise":true,"returnByValue":true});
            let session = if let Some((s, id)) = &self.eval_context {
                params["contextId"] = json!(id);
                Some(s.clone())
            } else {
                self.session.clone()
            };
            let v = self.call_to("Runtime.evaluate", params, session.as_deref())?;
            if v.get("exceptionDetails").is_some() {
                return Err("网站存储包含不可迁移的值，保留本机数据".into());
            };
            Ok(v["result"]["value"].clone())
        }
    }
    fn site(&mut self, scope: &SiteKey) -> Result<(), String> {
        self.eval_target = None;
        self.eval_context = None;
        let site = scope
            .partition_key
            .as_ref()
            .map(|p| p.top_level_site.as_str())
            .unwrap_or(&scope.origin);
        let url = format!("{site}/.__nodecloak_sync_{}", uuid::Uuid::new_v4().simple());
        if self.firefox {
            self.call(
                "browsingContext.navigate",
                json!({"context":self.target,"url":url,"wait":"none"}),
                false,
            )?;
        } else {
            self.call("Page.navigate", json!({"url":url}), true)?;
        }
        let start = Instant::now();
        let mut last_error = None;
        loop {
            let expression = format!(
                "location.href==={} && document.readyState!=='loading'",
                json!(url)
            );
            match self.eval(&expression) {
                Ok(value) if value == json!(true) => break,
                Err(error) if error == FIREFOX_WEB_CONTEXT_ERROR => return Err(error),
                Err(error) => last_error = Some(error),
                _ => {}
            }
            if start.elapsed() > Duration::from_secs(15) {
                return Err(format!("无法进入隔离网站存储（{}；{}）",if self.firefox {"Firefox"} else {"Chromium"},last_error.as_deref().unwrap_or("页面未完成加载")));
            };
            std::thread::sleep(Duration::from_millis(50));
        }
        if let Some(p) = &scope.partition_key {
            let a = reqwest::Url::parse(&scope.origin).map_err(|_| "网站来源无效")?;
            let top = reqwest::Url::parse(&p.top_level_site).map_err(|_| "网站分区无效")?;
            let same_site = a.scheme() == top.scheme()
                && (a.host_str() == top.host_str()
                    || a.host_str().is_some_and(|h| {
                        h.ends_with(&format!(".{}", top.host_str().unwrap_or("")))
                    }));
            if p.has_cross_site_ancestor && (self.firefox || same_site) {
                self.frame(&format!(
                    "https://nc-{}.invalid",
                    uuid::Uuid::new_v4().simple()
                ))?;
            }
            self.frame(&scope.origin)?;
            if !self.firefox {
                let (session, _) = self.eval_context.clone().ok_or("分区页面尚未连接")?;
                let target = self.eval_target.clone().ok_or("分区页面尚未连接")?;
                let actual = self.call_to(
                    "Storage.getStorageKeyForFrame",
                    json!({"frameId":target}),
                    Some(&session),
                )?;
                let actual = super::session_storage::chromium_key(
                    actual["storageKey"].as_str().ok_or("网站分区键无效")?,
                )?;
                if actual != *scope {
                    return Err("浏览器网站分区与快照不一致，保留本机数据".into());
                }
            }
        }
        let code = format!(
            "(()=>{{{};return true;}})()",
            include_str!("session_codec.js")
        );
        self.eval(&code)?;
        Ok(())
    }
    fn codec(&mut self, expression: &str) -> Result<Value, String> {
        // Only fixed codec error identifiers leave the isolated page. Browser
        // exceptions can otherwise include database names and website secrets.
        let wrapped = format!("(async()=>{{try{{return {{ok:true,value:await ({expression})}};}}catch(e){{const codes=['NC_LIMIT','NC_UNSUPPORTED_VALUE','non-exportable CryptoKey','NC_STORAGE_BLOCKED'];return {{ok:false,code:codes.includes(e.message)?e.message:'NC_CODEC_FAILED'}};}}}})()");
        codec_result(self.eval(&wrapped)?)
    }
    fn frame(&mut self, site: &str) -> Result<(), String> {
        let url = format!(
            "{site}/.__nodecloak_frame_{}",
            uuid::Uuid::new_v4().simple()
        );
        let expression=format!("(()=>{{const f=document.createElement('iframe');f.src={};document.documentElement.appendChild(f);return true;}})()",json!(url));
        self.eval(&expression)?;
        let started = Instant::now();
        let parent_session = self
            .eval_context
            .as_ref()
            .map(|(s, _)| s.clone())
            .or_else(|| self.session.clone());
        while started.elapsed() < Duration::from_secs(15) {
            if self.firefox {
                let tree = self.call(
                    "browsingContext.getTree",
                    json!({"root":self.target,"maxDepth":8}),
                    false,
                )?;
                if let Some(context) = find_frame(&tree["contexts"][0], &url, true) {
                    self.eval_target = Some(context);
                    if self
                        .eval(&format!(
                            "location.href==={} && document.readyState!=='loading'",
                            json!(url)
                        ))
                        .ok()
                        == Some(json!(true))
                    {
                        return Ok(());
                    }
                }
            } else {
                let tree = self.call("Page.getFrameTree", json!({}), true)?;
                let mut found = find_frame(&tree["frameTree"], &url, false).map(|id| {
                    let s = self
                        .frames
                        .get(&id)
                        .cloned()
                        .or_else(|| parent_session.clone());
                    (id, s)
                });
                if found.is_none() {
                    for s in self.frames.values().cloned().collect::<Vec<_>>() {
                        if let Ok(tree) = self.call_to("Page.getFrameTree", json!({}), Some(&s)) {
                            if let Some(id) = find_frame(&tree["frameTree"], &url, false) {
                                found = Some((id, Some(s)));
                                break;
                            }
                        }
                    }
                }
                if let Some((id, Some(s))) = found {
                    let world=self.call_to("Page.createIsolatedWorld",json!({"frameId":id,"worldName":"nodecloak-session","grantUniveralAccess":false}),Some(&s))?;
                    self.eval_context = Some((
                        s,
                        world["executionContextId"]
                            .as_i64()
                            .ok_or("分区执行上下文无效")?,
                    ));
                    self.eval_target = Some(id);
                    if self
                        .eval(&format!(
                            "location.href==={} && document.readyState!=='loading'",
                            json!(url)
                        ))
                        .ok()
                        == Some(json!(true))
                    {
                        return Ok(());
                    }
                }
            }
            std::thread::sleep(Duration::from_millis(30));
        }
        Err("无法进入隔离分区网站存储".into())
    }
    fn cookies(&mut self) -> Result<Vec<Cookie>, String> {
        let v = self.call(
            if self.firefox {
                "storage.getCookies"
            } else {
                "Storage.getCookies"
            },
            json!({}),
            false,
        )?;
        let list = v["cookies"].as_array().ok_or("Cookie 响应无效")?;
        let mut result = Vec::new();
        for c in list {
            if c["partitionKeyOpaque"] == true {
                return Err("此副本包含不可迁移的 Cookie 分区".into());
            };
            if self.firefox && c["value"]["type"] != "string" {
                return Err("此副本包含不可迁移的二进制 Cookie".into());
            };
            result.push(Cookie {
                name: c["name"].as_str().ok_or("Cookie 无效")?.into(),
                value: if self.firefox {
                    c["value"]["value"].as_str()
                } else {
                    c["value"].as_str()
                }
                .ok_or("Cookie 无效")?
                .into(),
                domain: c["domain"].as_str().ok_or("Cookie 无效")?.into(),
                path: c["path"].as_str().ok_or("Cookie 无效")?.into(),
                secure: c["secure"] == true,
                http_only: c["httpOnly"] == true,
                same_site: match c["sameSite"].as_str().unwrap_or("Default") {
                    "strict" | "Strict" => "Strict",
                    "lax" | "Lax" => "Lax",
                    "default" | "Default" => "Default",
                    _ => "None",
                }
                .into(),
                expires: if self.firefox {
                    c["expiry"].as_f64().unwrap_or(-1.)
                } else {
                    c["expires"].as_f64().unwrap_or(-1.)
                },
                partition_key: c
                    .get("partitionKey")
                    .filter(|v| v.is_object())
                    .map(|v| serde_json::from_value(v.clone()).map_err(|_| "Cookie 分区无效"))
                    .transpose()?,
                partitioned_attribute: true,
                firefox_partition_key: None,
            });
        }
        Ok(result)
    }
    fn set_cookies(&mut self, cookies: &[Cookie]) -> Result<(), String> {
        self.call(
            if self.firefox {
                "storage.deleteCookies"
            } else {
                "Storage.clearCookies"
            },
            json!({}),
            false,
        )?;
        if self.firefox {
            for c in cookies.iter().filter(|c| c.partition_key.is_none()) {
                let mut v = json!({"name":c.name,"value":{"type":"string","value":c.value},"domain":c.domain,"path":c.path,"secure":c.secure,"httpOnly":c.http_only,"sameSite":c.same_site.to_lowercase()});
                if c.expires > 0. {
                    // BiDi accepts integer seconds; the closed native database
                    // restores its millisecond precision before final readback.
                    v["expiry"] = json!(c.expires.floor() as u64)
                };
                self.call("storage.setCookie", json!({"cookie":v}), false)?;
            }
        } else {
            let mut list = Vec::new();
            for c in cookies {
                if !c.partitioned_attribute {
                    return Err("此 Cookie 分区属性仅支持 Firefox，保留本机数据".into());
                }
                let mut v = serde_json::to_value(c).map_err(|_| "Cookie 无效")?;
                v.as_object_mut().unwrap().remove("firefoxPartitionKey");
                if c.same_site == "Default" {
                    v.as_object_mut().unwrap().remove("sameSite");
                }
                if c.expires < 0. {
                    v.as_object_mut().unwrap().remove("expires");
                }
                list.push(v)
            }
            self.call("Storage.setCookies", json!({"cookies":list}), false)?;
        };
        Ok(())
    }
    fn close(&mut self) -> Result<(), String> {
        let start = Instant::now();
        // The close acknowledgement is not process completion. Firefox's remote
        // server also needs its WebSocket connection released to finish shutdown.
        // Keep this handshake inside the existing shutdown budget, rather than
        // using call(), whose normal storage-operation timeout is 120 seconds.
        if let MaybeTlsStream::Plain(tcp) = self.socket.get_mut() {
            let _ = tcp.set_read_timeout(Some(Duration::from_millis(100)));
            let _ = tcp.set_write_timeout(Some(Duration::from_secs(1)));
        }
        let method = if self.firefox {
            "browser.close"
        } else {
            "Browser.close"
        };
        if let Ok(id) = self.send(method, json!({}), false) {
            while start.elapsed() < Duration::from_secs(1) {
                match self.socket.read() {
                    Ok(Message::Text(text)) => {
                        if serde_json::from_str::<Value>(&text)
                            .ok()
                            .is_some_and(|v| v["id"] == id)
                        {
                            break;
                        }
                    }
                    Ok(Message::Close(_)) => break,
                    Err(tungstenite::Error::Io(e))
                        if matches!(e.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock) => {}
                    Err(_) => break,
                    _ => {}
                }
            }
        }
        let _ = self.socket.close(None);
        let _ = self.socket.flush();
        if let MaybeTlsStream::Plain(tcp) = self.socket.get_mut() {
            let _ = tcp.shutdown(std::net::Shutdown::Both);
        }
        while start.elapsed() < Duration::from_secs(15) {
            if self
                .child
                .try_wait()
                .map_err(|_| "无法等待浏览器关闭")?
                .is_some()
                && platform::profile_process_ids(&self.path)?.is_empty()
            {
                platform::ensure_profile_closed(&self.path)?;
                self.closed = true;
                return Ok(());
            };
            std::thread::sleep(Duration::from_millis(100));
        }
        Err("隔离浏览器未正常关闭，取消同步".into())
    }
}
fn browser_launch_path(path: &Path) -> String {
    let value = path.to_string_lossy();
    #[cfg(windows)]
    {
        // Chromium's IndexedDB SQLite backend rejects a verbatim Windows path.
        // This changes only the CLI spelling of the same validated staged path.
        if let Some(unc) = value.strip_prefix(r"\\?\UNC\") {
            return format!(r"\\{unc}");
        }
        if let Some(local) = value.strip_prefix(r"\\?\") {
            if local.as_bytes().get(1) == Some(&b':') {
                return local.into();
            }
        }
    }
    value.into_owned()
}
impl Drop for Browser {
    fn drop(&mut self) {
        if !self.closed {
            let _ = self.child.kill();
            let _ = self.child.wait();
            let _ = platform::force_close_profile(&self.path);
        }
    }
}
fn find_frame(tree: &Value, url: &str, firefox: bool) -> Option<String> {
    let node = if firefox { tree } else { &tree["frame"] };
    if node["url"] == url {
        return node[if firefox { "context" } else { "id" }]
            .as_str()
            .map(str::to_owned);
    }
    tree[if firefox { "children" } else { "childFrames" }]
        .as_array()
        .and_then(|children| {
            children
                .iter()
                .find_map(|child| find_frame(child, url, firefox))
        })
}
struct Stage {
    temp: tempfile::TempDir,
    path: PathBuf,
    firefox_prefs: Vec<(String, Option<Vec<u8>>)>,
    firefox_extensions: Option<PathBuf>,
}
const FIREFOX_EXTENSION_STATE: [&str;3] = ["extensions","extensions.json","addonStartup.json.lz4"];
impl Stage {
    fn new(browser: &str, profile: &Path) -> Result<Self, String> {
        profiles::guard_tree(profile)?;
        platform::ensure_profile_closed(profile)?;
        let parent = profile.parent().ok_or("副本目录无效")?;
        std::fs::create_dir_all(parent).map_err(|e| profile_io_error(Path::new(""), "create_temporary_parent", &e))?;
        let temp = tempfile::Builder::new()
            .prefix(".nodecloak-sync-")
            .tempdir_in(parent)
            .map_err(|e| profile_io_error(Path::new(""), "create_temporary_directory", &e))?;
        let path = temp.path().join("profile");
        match std::fs::symlink_metadata(profile) {
            Ok(_) => copy_tree_inner(profile, &path, Path::new(""), browser != "firefox", &mut 0)?,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound =>
                std::fs::create_dir(&path).map_err(|e| profile_io_error(Path::new(""), "create_temporary_directory", &e))?,
            Err(e) => return Err(profile_io_error(Path::new(""), "read_profile_metadata", &e)),
        }
        let mut firefox_prefs = Vec::new();
        let mut firefox_extensions = None;
        if browser == "firefox" {
            // Offline storage documents must not run profile extensions. In
            // particular, the privacy companion cannot authenticate a temporary
            // clone and correctly cancels its navigation. Keep all packages and
            // metadata outside the codec profile, restoring them before commit.
            let backup=temp.path().join("extension-state");
            std::fs::create_dir(&backup).map_err(|_|"无法保存临时副本扩展")?;
            for name in FIREFOX_EXTENSION_STATE {
                let source=path.join(name);
                if source.exists() {
                    std::fs::rename(source,backup.join(name)).map_err(|_|"无法隔离临时副本扩展")?;
                }
            }
            firefox_extensions=Some(backup);
            for name in ["user.js", "prefs.js"] {
                firefox_prefs.push((name.into(), std::fs::read(path.join(name)).ok()));
            }
            let mut user = firefox_prefs[0].1.clone().unwrap_or_default();
            user.extend_from_slice(b"\nuser_pref(\"network.proxy.type\",1);user_pref(\"network.proxy.http\",\"127.0.0.1\");user_pref(\"network.proxy.http_port\",9);user_pref(\"network.proxy.ssl\",\"127.0.0.1\");user_pref(\"network.proxy.ssl_port\",9);user_pref(\"network.proxy.no_proxies_on\",\"\");user_pref(\"network.proxy.allow_hijacking_localhost\",true);user_pref(\"dom.serviceWorkers.enabled\",false);user_pref(\"browser.sessionstore.resume_from_crash\",false);user_pref(\"browser.startup.page\",0);\n");
            // The offline codec serves empty intercepted documents, including
            // Mozilla web origins. Give those documents ordinary web content
            // capabilities instead of privileged Mozilla actors. Never grant
            // system access to BiDi. Both preferences are restored byte-for-byte
            // before staged data can replace the real profile.
            user.extend_from_slice(b"user_pref(\"browser.tabs.remote.separatePrivilegedMozillaWebContentProcess\",false);\nuser_pref(\"browser.tabs.remote.separatedMozillaDomains\",\"\");\n");
            user.extend_from_slice(b"user_pref(\"extensions.enabledScopes\",0);\n");
            engine::atomic_write(&path.join("user.js"), &user)?;
        }
        Ok(Self {
            temp,
            path,
            firefox_prefs,
            firefox_extensions,
        })
    }
    fn restore_prefs(&self) -> Result<(), String> {
        if let Some(backup)=&self.firefox_extensions {
            for name in FIREFOX_EXTENSION_STATE {
                let target=self.path.join(name);
                profiles::guard_tree(&target)?;
                if target.is_dir() {
                    std::fs::remove_dir_all(&target).map_err(|_|"无法清理临时扩展状态")?;
                } else if target.exists() {
                    std::fs::remove_file(&target).map_err(|_|"无法清理临时扩展状态")?;
                }
                let source=backup.join(name);
                if source.is_dir() {copy_tree(&source,&target,&mut 0)?;}
                else if source.exists() {
                    std::fs::copy(source,target).map_err(|_|"无法恢复副本扩展")?;
                }
            }
        }
        for (n, v) in &self.firefox_prefs {
            let p = self.path.join(n);
            if let Some(v) = v {
                engine::atomic_write(&p, v)?
            } else if p.exists() {
                std::fs::remove_file(p).map_err(|_| "无法恢复副本设置")?
            }
        }
        for n in ["DevToolsActivePort", "WebDriverBiDiServer.json"] {
            let p = self.path.join(n);
            if p.exists() {
                std::fs::remove_file(p).map_err(|_| "无法清理调试标记")?
            }
        }
        Ok(())
    }
    fn commit(self, profile: &Path) -> Result<(), String> {
        platform::ensure_profile_closed(profile)?;
        self.restore_prefs()?;
        let backup = self.temp.path().join("previous");
        let journal = journal_path(profile)?;
        let name = self
            .temp
            .path()
            .file_name()
            .ok_or("临时目录无效")?
            .to_string_lossy()
            .into_owned();
        engine::atomic_write(
            &journal,
            &serde_json::to_vec(&json!({"stage":name,"hadOriginal":profile.exists()}))
                .map_err(|_| "恢复记录无效")?,
        )?;
        // Keep the staged directory on failure/crash; recover() handles the two rename boundaries.
        let _keep = self.temp.keep();
        if profile.exists() {
            std::fs::rename(profile, &backup).map_err(|_| "无法备份当前副本")?
        };
        if std::fs::rename(&self.path, profile).is_err() {
            if backup.exists() {
                let _ = std::fs::rename(&backup, profile);
            };
            return Err("无法替换副本，已保留原数据".into());
        };
        std::fs::remove_file(journal).map_err(|_| "无法清理恢复记录")?;
        let stage = backup.parent().unwrap();
        profiles::guard_tree(stage)?;
        std::fs::remove_dir_all(stage).map_err(|_| "无法清理本机临时备份")?;
        Ok(())
    }
}
fn journal_path(profile: &Path) -> Result<PathBuf, String> {
    let name = profile
        .file_name()
        .and_then(|s| s.to_str())
        .ok_or("副本目录无效")?;
    Ok(profile.with_file_name(format!(".nodecloak-session-{name}.json")))
}
pub fn recover(profile: &Path) -> Result<(), String> {
    let journal = journal_path(profile)?;
    profiles::guard_tree(&journal)?;
    if !journal.exists() {
        return Ok(());
    };
    platform::ensure_profile_closed(profile)?;
    let v: Value =
        serde_json::from_slice(&std::fs::read(&journal).map_err(|_| "无法读取恢复记录")?)
            .map_err(|_| "恢复记录无效")?;
    let n = v["stage"].as_str().ok_or("恢复记录无效")?;
    if !n.starts_with(".nodecloak-sync-")
        || n.bytes()
            .any(|c| !c.is_ascii_alphanumeric() && !b".-_".contains(&c))
    {
        return Err("恢复目录无效".into());
    };
    let stage = profile.parent().unwrap().join(n);
    profiles::guard_tree(&stage)?;
    let backup = stage.join("previous");
    if !profile.exists() {
        let source = if backup.exists() {
            backup
        } else if v["hadOriginal"] == false && stage.join("profile").exists() {
            stage.join("profile")
        } else {
            return Err("本机副本恢复数据缺失".into());
        };
        std::fs::rename(&source, profile).map_err(|_| "无法恢复原副本")?
    };
    std::fs::remove_file(journal).map_err(|_| "无法清理恢复记录")?;
    if stage.exists() {
        std::fs::remove_dir_all(stage).map_err(|_| "无法清理恢复目录")?
    };
    Ok(())
}
pub fn export_profile(browser: &str, profile: &Path) -> Result<Snapshot, String> {
    recover(profile)?;
    capture_with_retry(|| export_staged_profile(browser, profile), |attempt, error| {
        crate::browser_control::log_capture_failure(profile, browser, attempt, error);
    })
}
fn capture_with_retry(
    mut capture: impl FnMut() -> Result<Snapshot, String>,
    mut failed: impl FnMut(usize, &str),
) -> Result<Snapshot, String> {
    for attempt in 1..=2 {
        match capture() {
            Ok(snapshot) => return Ok(snapshot),
            Err(error) => {
                failed(attempt, &error);
                if attempt == 2 || !(capture_transport_failure(&error) || retryable_profile_io(&error)) { return Err(error); }
                if retryable_profile_io(&error) { std::thread::sleep(Duration::from_millis(150)); }
            }
        }
    }
    unreachable!()
}
fn export_staged_profile(browser: &str, profile: &Path) -> Result<Snapshot, String> {
    let stage = Stage::new(browser, profile)?;
    let origins = disk_origins(browser, &stage.path)?;
    let mut local = std::collections::BTreeMap::new();
    if browser != "firefox" {
        for key in &origins {
            if super::session_local::eligible(&stage.path, key)? {
                local.insert(key.clone(), super::session_local::read(&stage.path, key)?);
            }
        }
    }
    let mut b = Browser::launch(browser, &stage.path)?;
    let mut cookies = b.cookies()?;
    let mut sites = Vec::new();
    for o in origins {
        let state = if let Some(state) = local.remove(&o) { state } else {
            b.site(&o)?;
            b.codec("__ncSessionCodec.exportOrigin()")?
        };
        if state["local"].as_array().is_some_and(|v| !v.is_empty())
            || state["databases"].as_array().is_some_and(|v| !v.is_empty())
        {
            sites.push(Site {
                origin: o.origin,
                partition_key: o.partition_key,
                state,
            });
        }
    }
    b.close()?;
    if browser == "firefox" {
        cookies = super::session_storage::firefox_cookies(&stage.path)?;
    }
    Snapshot::from_parts(cookies, sites)
}
pub fn restore_profile(
    browser: &str,
    profile: &Path,
    snapshot: &Snapshot,
) -> Result<Snapshot, String> {
    let snapshot = Snapshot::parse(serde_json::to_value(snapshot).map_err(|_| "登录数据无效")?)?;
    recover(profile)?;
    let stage = Stage::new(browser, profile)?;
    let mut origins = disk_origins(browser, &stage.path)?;
    if browser == "firefox" {
        // Reject unknown Cookie-only containers before BiDi can clear them.
        super::session_storage::firefox_cookies(&stage.path)?;
    }
    origins.extend(snapshot.origins.iter().map(Site::key));
    let mut local = Vec::new();
    let mut documents = Vec::new();
    for key in origins {
        let state = snapshot.origins.iter().find(|s| s.key() == key)
            .map(|s| s.state.clone()).unwrap_or_else(|| json!({"local":[],"databases":[]}));
        if browser != "firefox" && state["databases"].as_array().is_some_and(Vec::is_empty)
            && super::session_local::eligible(&stage.path, &key)? {
            local.push((key, state));
        } else { documents.push((key, state)); }
    }
    let mut b = Browser::launch(browser, &stage.path)?;
    for (o, state) in documents {
        b.site(&o)?;
        let expression = format!(
            "__ncSessionCodec.importOrigin({})",
            serde_json::to_string(&state).map_err(|_| "网站存储无效")?
        );
        b.codec(&expression)?;
    }
    b.set_cookies(&snapshot.cookies)?;
    let cookies = b.cookies()?;
    let expected = snapshot
        .cookies
        .iter()
        .filter(|c| browser != "firefox" || c.partition_key.is_none())
        .map(|c| {
            let mut c = c.clone();
            if browser != "firefox" {
                c.firefox_partition_key = None;
            }
            c
        })
        .collect::<Vec<_>>();
    verify_cookies(&expected, &cookies)?;
    let mut sites = Vec::new();
    for site in &snapshot.origins {
        if local.iter().any(|(key, _)| *key == site.key()) { continue; }
        b.site(&site.key())?;
        let mut state = b.codec("__ncSessionCodec.exportOrigin()")?;
        canonical_storage(&mut state)?;
        if state != site.state {
            return Err("网站存储写入复检不一致，取消导入并保留本机数据".into());
        };
        sites.push(Site {
            origin: site.origin.clone(),
            partition_key: site.partition_key.clone(),
            state,
        });
    }
    b.close()?;
    // Native DB handles are opened only after every browser process has closed.
    // All changes remain staged until both native and browser readback succeed.
    for (key, expected) in local {
        super::session_local::write(&stage.path, &key, &expected)?;
        let actual = super::session_local::read(&stage.path, &key)?;
        if actual != expected {
            return Err("网站存储写入复检不一致，取消导入并保留本机数据".into());
        }
        if snapshot.origins.iter().any(|s| s.key() == key) {
            sites.push(Site { origin: key.origin, partition_key: key.partition_key, state: actual });
        }
    }
    let mut cookies = cookies;
    if browser == "firefox" {
        super::session_storage::write_firefox_partitioned_cookies(&stage.path, &snapshot.cookies)?;
        // Reopen the isolated profile so Firefox validates and loads its native
        // cookie records before trusting the persisted partition identities.
        let mut check = Browser::launch(browser, &stage.path)?;
        let _ = check.cookies()?;
        check.close()?;
        cookies = super::session_storage::firefox_cookies(&stage.path)?;
        let keys = disk_origins(browser, &stage.path)?;
        for site in &snapshot.origins {
            if !keys.contains(&site.key()) {
                return Err("Firefox 未保留全部网站存储分区，保留本机数据".into());
            }
        }
        verify_cookies(&snapshot.cookies, &cookies)?;
    }
    let actual = Snapshot::from_parts(cookies, sites)?;
    stage.commit(profile)?;
    Ok(actual)
}
fn verify_cookies(expected: &[Cookie], actual: &[Cookie]) -> Result<(), String> {
    for c in expected
        .iter()
        .filter(|c| c.expires < 0. || c.expires > chrono::Utc::now().timestamp() as f64)
    {
        if !actual.iter().any(|v| {
            v.name == c.name
                && v.value == c.value
                && v.domain == c.domain
                && v.path == c.path
                && v.secure == c.secure
                && v.http_only == c.http_only
                && v.same_site == c.same_site
                && v.partition_key == c.partition_key
                && v.partitioned_attribute == c.partitioned_attribute
                && (c.firefox_partition_key.is_none()
                    || v.firefox_partition_key == c.firefox_partition_key)
                && (if c.expires < 0. {
                    v.expires < 0.
                } else {
                    v.expires > 0. && (v.expires - c.expires).abs() < 1.
                })
        }) {
            return Err("浏览器未保留全部 Cookie，取消导入并保留本机数据".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;
    #[test]
    fn staging_reports_io_operation_and_code_without_profile_paths_or_contents() {
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let source = canonical.join("private-user-and-site.invalid");
        std::fs::write(&source, b"private-login-token").unwrap();
        let error = copy_tree(&source, &canonical.join("staged"), &mut 0).unwrap_err();
        assert!(error.contains("operation=read_directory"), "{error}");
        assert!(error.contains("kind=") && error.contains("os_code="), "{error}");
        assert!(!error.contains("private-user") && !error.contains("private-login"));
        assert_eq!(std::fs::read(source).unwrap(), b"private-login-token");
        let error = profile_io_error(Path::new("Default/IndexedDB/https_private-site-token_0"), "copy_file",
            &std::io::Error::new(std::io::ErrorKind::PermissionDenied, "private-OS-error-body"));
        assert!(error.contains("area=indexed_db") && error.contains("kind=PermissionDenied"));
        assert!(!error.contains("private-site") && !error.contains("private-OS"));
    }
    #[test]
    fn staging_omits_only_chromium_regenerable_caches_at_browser_locations() {
        for browser in ["chrome", "edge", "firefox"] {
            let root = tempfile::tempdir().unwrap();
            let source = root.path().canonicalize().unwrap().join("profile");
            for parent in ["", "Default", "Default/IndexedDB/private-store"] {
                for name in ["BrowserMetrics", "DawnGraphiteCache", "DawnWebGPUCache"] {
                    let directory = source.join(parent).join(name);
                    std::fs::create_dir_all(&directory).unwrap();
                    std::fs::write(directory.join("sentinel"), b"keep-real-data").unwrap();
                }
            }
            std::fs::write(source.join("cookie-sentinel"), b"keep-login").unwrap();
            #[cfg(windows)]
            let _cache_locks = {
                use std::os::windows::fs::OpenOptionsExt;
                let mut locks = Vec::new();
                if browser != "firefox" {
                    for parent in ["", "Default"] {
                        for name in ["BrowserMetrics", "DawnGraphiteCache", "DawnWebGPUCache"] {
                            let directory = source.join(parent).join(name);
                            locks.push(std::fs::OpenOptions::new().read(true).share_mode(0)
                                .custom_flags(0x02000000).open(&directory).unwrap());
                            assert_eq!(std::fs::read_dir(directory).err().unwrap().raw_os_error(), Some(32));
                        }
                    }
                }
                locks
            };
            let stage = Stage::new(browser, &source).unwrap();
            for name in ["BrowserMetrics", "DawnGraphiteCache", "DawnWebGPUCache"] {
                for parent in ["", "Default"] {
                    assert_eq!(stage.path.join(parent).join(name).exists(), browser == "firefox", "{browser}: {parent}/{name}");
                    assert!(source.join(parent).join(name).join("sentinel").exists());
                }
                assert_eq!(std::fs::read(stage.path.join("Default/IndexedDB/private-store").join(name).join("sentinel")).unwrap(), b"keep-real-data");
            }
            assert_eq!(std::fs::read(stage.path.join("cookie-sentinel")).unwrap(), b"keep-login");
        }
    }
    #[test]
    fn staging_retries_transient_sharing_failure_once_but_not_access_denied() {
        for (code, expected_calls) in [(32, 2), (5, 1)] {
            let error = format!("登录同步暂时无法读取本机浏览器数据（operation=read_directory, area=browser_files, kind=PermissionDenied, os_code=Some({code})；本机数据已保留）");
            let mut calls = 0;
            let mut failures = Vec::new();
            let result = capture_with_retry(|| {
                calls += 1;
                if calls == 1 { Err(error.clone()) } else { Ok(Snapshot::empty()) }
            }, |attempt, message| failures.push((attempt, message.to_owned())));
            assert_eq!(calls, expected_calls);
            assert_eq!(result.is_ok(), expected_calls == 2);
            assert_eq!(failures, [(1, error)]);
        }
    }
    #[test]
    fn snapshots_accept_more_than_eight_mib_and_reject_over_sixty_four_without_trimming() {
        let value="x".repeat(9<<20);
        let data=json!({"schema":1,"cookies":[],"origins":[{"origin":"https://large-fixture.invalid","state":{"local":[["login",value]],"databases":[]}}]});
        let snapshot=Snapshot::parse(data).expect("large snapshots must be supported");
        assert_eq!(snapshot.origins[0].state["local"][0][1].as_str().unwrap().len(),9<<20);
        let data=json!({"schema":1,"cookies":[],"origins":[{"origin":"https://large-fixture.invalid","state":{"local":[["login","x".repeat(64<<20)]],"databases":[]}}]});
        assert!(Snapshot::parse(data).err().unwrap().contains("64 MiB"));
    }
    #[test]
    fn offline_firefox_stage_restores_extensions_and_metadata_after_commit() {
        let root=tempfile::tempdir().unwrap();
        let canonical=root.path().canonicalize().unwrap();
        let profile=canonical.join("profile");
        std::fs::create_dir_all(profile.join("extensions")).unwrap();
        let originals=[("extensions/fingerprint@nodecloak.com.xpi",b"signed-extension".as_slice()),
            ("extensions.json",b"original extension metadata".as_slice()),
            ("addonStartup.json.lz4",b"original startup cache".as_slice()),
            ("user.js",b"user_pref(\"custom.sentinel\",true);".as_slice()),
            ("prefs.js",b"original preferences".as_slice()),
            ("cookie-sentinel",b"original login data".as_slice())];
        for (name,bytes) in originals {std::fs::write(profile.join(name),bytes).unwrap();}
        let stage=Stage::new("firefox",&profile).unwrap();
        for name in ["extensions","extensions.json","addonStartup.json.lz4"] {
            assert!(!stage.path.join(name).exists(),"offline codec still loads {name}");
        }
        for (name,bytes) in originals {assert_eq!(std::fs::read(profile.join(name)).unwrap(),bytes);}
        std::fs::create_dir_all(stage.path.join("extensions")).unwrap();
        std::fs::write(stage.path.join("extensions/unwanted-generated.xpi"),b"generated").unwrap();
        std::fs::write(stage.path.join("extensions.json"),b"generated metadata").unwrap();
        stage.commit(&profile).unwrap();
        for (name,bytes) in originals {assert_eq!(std::fs::read(profile.join(name)).unwrap(),bytes,"changed {name}");}
        assert!(!profile.join("extensions/unwanted-generated.xpi").exists());
        let blank=canonical.join("blank");
        std::fs::create_dir(&blank).unwrap();
        let stage=Stage::new("firefox",&blank).unwrap();
        std::fs::write(stage.path.join("extensions.json"),b"generated metadata").unwrap();
        stage.commit(&blank).unwrap();
        for name in ["extensions","extensions.json","addonStartup.json.lz4","user.js","prefs.js"] {
            assert!(!blank.join(name).exists(),"created missing original {name}");
        }
    }
    #[test]
    fn navigation_rejection_retains_only_safe_protocol_diagnostics() {
        let error=Browser::result(json!({"type":"error","error":"unknown error",
            "message":"Navigation failed NS_ERROR_ABORT https://private.example/token"}),"browsingContext.navigate").unwrap_err();
        assert!(error.contains("code=unknown error"),"{error}");
        assert!(error.contains("reason=NS_ERROR_ABORT"),"{error}");
        assert!(!error.contains("private.example"));
        let error=Browser::result(json!({"type":"error","error":"private-secret","message":"private-secret"}),"browsingContext.navigate").unwrap_err();
        assert!(error.contains("code=other"));
        assert!(!error.contains("private-secret"));
    }
    #[test]
    fn codec_results_preserve_values_and_only_report_fixed_error_categories() {
        assert_eq!(codec_result(json!({"ok":true,"value":{"local":[["key","value"]],"databases":[]}})).unwrap()["local"][0][1], "value");
        for (code, expected) in [("NC_LIMIT","网站存储超过同步容量限制，本机数据已保留"),
            ("NC_UNSUPPORTED_VALUE","网站存储包含不可迁移的值，保留本机数据"),
            ("non-exportable CryptoKey","网站存储包含不可迁移的值，保留本机数据"),
            ("NC_STORAGE_BLOCKED","隔离网站存储被浏览器阻止，本机数据已保留"),
            ("private-database-name","网站存储处理失败，保留本机数据")] {
            assert_eq!(codec_result(json!({"ok":false,"code":code,"message":"private-browser-payload"})).unwrap_err(),expected);
        }
    }
    #[test]
    fn firefox_system_access_rejection_is_actionable_without_exposing_browser_messages() {
        let error = Browser::result(json!({"type":"error","error":"unsupported operation",
            "message":"System access is required; private-browser-payload"}), "script.evaluate").unwrap_err();
        assert_eq!(error, "Firefox 隔离网站存储页面受特殊内容限制，已保留本机数据");
        assert!(!error.contains("private-browser-payload"));
    }
    #[test]
    fn export_retries_transport_failure_once_without_retrying_invalid_storage() {
        for error in ["隔离浏览器连接中断", "无法读取网站存储"] {
            let mut calls = 0;
            let mut failures = vec![];
            let result = capture_with_retry(|| {
                calls += 1;
                if calls == 1 { Err(error.into()) } else { Ok(Snapshot::empty()) }
            }, |attempt, message| failures.push((attempt, message.to_owned())));
            if capture_transport_failure(error) {
                assert!(result.is_ok());
                assert_eq!(calls, 2);
            } else {
                assert_eq!(result.err().as_deref(), Some(error));
                assert_eq!(calls, 1);
            }
            assert_eq!(failures, vec![(1, error.to_owned())]);
        }
        let mut calls = 0;
        assert!(capture_with_retry(|| { calls += 1; Err("隔离浏览器连接中断".into()) }, |_, _| {}).is_err());
        assert_eq!(calls, 2);
    }
    #[test]
    fn isolated_transport_diagnostics_do_not_expose_browser_payloads() {
        let error = std::io::Error::new(std::io::ErrorKind::ConnectionReset, "secret-browser-payload");
        let message = isolated_transport_error("Storage.getCookies", &error.into());
        assert!(message.contains("Storage.getCookies"));
        assert!(message.contains("ConnectionReset"));
        assert!(!message.contains("secret-browser-payload"));
    }
    #[test]
    #[ignore = "Read-only source profile diagnostic; launches only a staged copy, never prints website data"]
    fn staged_profile_navigation_diagnostic() {
        let browser = std::env::var("NODECLOAK_DIAGNOSTIC_BROWSER").unwrap();
        let source = PathBuf::from(std::env::var_os("NODECLOAK_DIAGNOSTIC_PROFILE").unwrap());
        let stage = Stage::new(&browser, &source).unwrap();
        let scopes = disk_origins(&browser, &stage.path).unwrap();
        let mut b = Browser::launch(&browser, &stage.path).unwrap();
        println!("staged navigation: {} namespaces", scopes.len());
        for (index, scope) in scopes.iter().enumerate() {
            if let Err(error) = b.site(scope) {
                let expected = scope.partition_key.as_ref().map(|p| &p.top_level_site).unwrap_or(&scope.origin);
                let probe = b.eval(&format!("({{protocol:location.protocol,originMatches:location.origin==={},readyState:document.readyState,errorPage:document.documentURI.startsWith('about:neterror'),visibility:document.visibilityState}})",json!(expected)));
                println!("failed namespace {index}: {error}; partitioned={}; expected_scheme={}; readiness={probe:?}; alive={:?}",scope.partition_key.is_some(),reqwest::Url::parse(expected).unwrap().scheme(),b.call("browsingContext.getTree",json!({"root":b.target,"maxDepth":0}),false).map(|v| v["contexts"].as_array().map(|a|a.len())));
                panic!("isolated navigation failed");
            }
            println!("namespace {index} navigation succeeded");
            if std::env::var_os("NODECLOAK_DIAGNOSTIC_CODEC").is_some() {
                let probe = b.eval("(async()=>{try{await __ncSessionCodec.exportOrigin();return {ok:true};}catch(e){const codes=['NC_LIMIT','NC_UNSUPPORTED_VALUE','non-exportable CryptoKey','NC_STORAGE_BLOCKED','NC_INVALID_GRAPH'];return {ok:false,code:codes.includes(e.message)?e.message:'unknown',securityError:e.name==='SecurityError'};}})()").unwrap();
                println!("namespace {index} codec probe: {probe}");
                assert_eq!(probe["ok"], true, "namespace {index} codec failed: {probe}");
            }
        }
        b.close().unwrap();
    }
    #[test]
    #[ignore = "Read-only source profile diagnostic; launches only a staged copy, never prints website data"]
    fn staged_profile_capture_diagnostic() {
        let browser = std::env::var("NODECLOAK_DIAGNOSTIC_BROWSER").unwrap();
        let source = PathBuf::from(std::env::var_os("NODECLOAK_DIAGNOSTIC_PROFILE").unwrap());
        let stage = Stage::new(&browser, &source).unwrap();
        let discovered = disk_origins(&browser, &stage.path).unwrap();
        println!("discovery complete: {} website namespaces", discovered.len());
        if browser != "firefox" {
            for (index, key) in discovered.iter().enumerate() {
                if super::super::session_local::eligible(&stage.path, key).unwrap() {
                    super::super::session_local::read(&stage.path, key).unwrap_or_else(|error| panic!("native namespace {index}: {error}"));
                    println!("native namespace {index} decoded");
                }
            }
        }
        drop(stage);
        let snapshot = export_profile(&browser, &source).unwrap();
        println!("complete export: {} cookies, {} nonempty website namespaces", snapshot.cookies.len(), snapshot.origins.len());
        assert!(snapshot.origins.iter().all(|site| discovered.contains(&site.key())));
        if browser == "edge" && discovered.iter().any(|key| key.origin == "https://ntp.msn.com") {
            assert!(snapshot.origins.iter().any(|site| site.origin == "https://ntp.msn.com"), "NTP storage was dropped");
        }
    }
    #[test]
    #[ignore = "Offline installed Firefox protected-web storage roundtrip with synthetic data"]
    fn installed_firefox_mozilla_web_storage_roundtrip() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let profile = dir.path().join("firefox-storage");
        std::fs::create_dir_all(&profile).unwrap();
        let original = b"user_pref(\"browser.tabs.remote.separatePrivilegedMozillaWebContentProcess\",true);\nuser_pref(\"browser.tabs.remote.separatedMozillaDomains\",\"addons.mozilla.org,accounts.firefox.com\");\nuser_pref(\"custom.sentinel\",true);\n";
        std::fs::write(profile.join("user.js"), original).unwrap();
        let original_prefs = b"user_pref(\"custom.prefs.sentinel\",\"keep\");\n";
        std::fs::write(profile.join("prefs.js"), original_prefs).unwrap();
        // Real browser import/export must preserve addon data as well as the
        // packages themselves, even though the codec never executes addons.
        let addon_state=[
            ("extensions/fingerprint@nodecloak.com.xpi",include_bytes!("../../resources/firefox/nodecloak-fingerprint-0.1.0.xpi").as_slice()),
            ("extensions.json",br#"{"schemaVersion":39,"addons":[{"id":"fingerprint@nodecloak.com","active":true,"userDisabled":false}]}"#.as_slice()),
            ("addonStartup.json.lz4",b"synthetic original addon startup metadata".as_slice()),
            ("browser-extension-data/fingerprint@nodecloak.com/storage.js",br#"{"sentinel":"keep-extension-data"}"#.as_slice()),
            ("extension-settings.json",br#"{"version":3,"prefs":{},"syntheticSentinel":"keep-settings"}"#.as_slice()),
            ("extension-preferences.json",br#"{"version":1,"syntheticSentinel":"keep-preferences"}"#.as_slice()),
        ];
        for (name,bytes) in addon_state {
            let file=profile.join(name);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file,bytes).unwrap();
        }
        let input = Snapshot::parse(json!({"schema":1,"cookies":[],"origins":[
            {"origin":"https://accounts.firefox.com","state":{"local":[["synthetic-account","keep-account"]],"databases":[]}},
            {"origin":"https://addons.mozilla.org","state":{"local":[["synthetic-addon","keep-addon"]],"databases":[]}}
        ]})).unwrap();
        let restored = restore_profile("firefox", &profile, &input).unwrap();
        assert_eq!(restored.bytes().unwrap(), input.bytes().unwrap());
        assert_eq!(std::fs::read(profile.join("user.js")).unwrap(), original);
        assert_eq!(std::fs::read(profile.join("prefs.js")).unwrap(), original_prefs);
        for (name,bytes) in addon_state {assert_eq!(std::fs::read(profile.join(name)).unwrap(),bytes,"import changed addon state {name}");}
        let captured = export_profile("firefox", &profile).unwrap();
        assert_eq!(captured.bytes().unwrap(), input.bytes().unwrap());
        assert_eq!(std::fs::read(profile.join("user.js")).unwrap(), original);
        assert_eq!(std::fs::read(profile.join("prefs.js")).unwrap(), original_prefs);
        for (name,bytes) in addon_state {assert_eq!(std::fs::read(profile.join(name)).unwrap(),bytes,"export changed addon state {name}");}
        let stage = Stage::new("firefox", &profile).unwrap();
        let mut browser = Browser::launch("firefox", &stage.path).unwrap();
        browser.site(&SiteKey{origin:"https://large.example".into(),partition_key:None}).unwrap();
        browser.eval("(()=>{localStorage.setItem('large-cache','x'.repeat(1048577));return true;})()").unwrap();
        let state = browser.codec("__ncSessionCodec.exportOrigin()").unwrap();
        assert_eq!(state["local"][0][0], "large-cache");
        assert_eq!(state["local"][0][1].as_str().unwrap().len(), 1048577);
        assert_eq!(browser.codec("__ncSessionCodec.pack('x'.repeat((64<<20)+1))").unwrap_err(), "网站存储超过同步容量限制，本机数据已保留");
        browser.close().unwrap();
    }
    #[test]
    fn cookie_readback_rejects_changed_expiry_and_native_partition_attributes() {
        let input=Snapshot::parse(json!({"schema":2,"cookies":[{"name":"session","value":"token","domain":"login.example","path":"/","secure":true,"httpOnly":true,"sameSite":"None","expires":chrono::Utc::now().timestamp()+604800,"partitionKey":{"topLevelSite":"https://shop.example","hasCrossSiteAncestor":true},"firefoxPartitionKey":"^partitionKey=%28https%2Cshop.example%29"}],"origins":[]})).unwrap();
        for change in 0..3 {
            let mut actual = input.cookies.clone();
            match change {
                0 => actual[0].expires += 86400.,
                1 => actual[0].partitioned_attribute = false,
                _ => actual[0].firefox_partition_key = None,
            }
            assert!(
                verify_cookies(&input.cookies, &actual).is_err(),
                "changed field {change}"
            );
        }
        assert!(verify_cookies(&input.cookies, &input.cookies).is_ok());
    }
    #[test]
    fn partitioned_snapshot_preserves_same_origin_in_distinct_top_level_sites() {
        let value = json!({"schema":2,"cookies":[],"origins":[
            {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://shop.example","hasCrossSiteAncestor":true},"state":{"local":[["session","shop"]],"databases":[]}},
            {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://work.example","hasCrossSiteAncestor":true},"state":{"local":[["session","work"]],"databases":[]}},
            {"origin":"https://login.example","state":{"local":[["session","direct"]],"databases":[]}}
        ]});
        let snapshot = Snapshot::parse(value).unwrap();
        let output: Value = serde_json::from_slice(&snapshot.bytes().unwrap()).unwrap();
        assert_eq!(output["origins"].as_array().unwrap().len(), 3);
        assert!(output["origins"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |s| s["partitionKey"]["topLevelSite"] == "https://shop.example"
                    && s["state"]["local"][0][1] == "shop"
            ));
        assert!(output["origins"]
            .as_array()
            .unwrap()
            .iter()
            .any(
                |s| s["partitionKey"]["topLevelSite"] == "https://work.example"
                    && s["state"]["local"][0][1] == "work"
            ));
        assert!(Snapshot::parse(json!({"schema":1,"cookies":[],"origins":[]})).is_ok());
        let root = super::super::crypto::random_key().unwrap();
        let ctx = super::super::crypto::Context {
            account: "synthetic-account",
            object: "synthetic-profile",
            kind: "session",
            revision: 1,
            epoch: 1,
            deleted: false,
            generation: 1,
            writer: "synthetic-device",
        };
        let envelope = super::super::crypto::seal(&root, &ctx, &snapshot.bytes().unwrap()).unwrap();
        let plain = super::super::crypto::open(&root, &ctx, &envelope).unwrap();
        let recovered = Snapshot::parse(serde_json::from_slice(&plain).unwrap()).unwrap();
        assert_eq!(recovered.bytes().unwrap(), snapshot.bytes().unwrap());
    }
    #[test]
    fn partitioned_firefox_storage_discovery_keeps_top_level_sites_separate() {
        let dir = tempfile::tempdir().unwrap();
        for name in [
            "https+++login.example",
            "https+++login.example^partitionKey=%28https%2Cshop.example%29",
            "https+++login.example^partitionKey=%28https%2Cwork.example%29",
        ] {
            std::fs::create_dir_all(dir.path().join("storage/default").join(name)).unwrap();
        }
        let found = serde_json::to_value(disk_origins("firefox", dir.path()).unwrap()).unwrap();
        assert_eq!(
            found,
            json!([
                {"origin":"https://login.example"},
                {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://shop.example","hasCrossSiteAncestor":true}},
                {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://work.example","hasCrossSiteAncestor":true}}
            ])
        );
    }
    #[test]
    #[ignore = "Offline installed-browser partition migration using synthetic temporary profiles only"]
    fn installed_browsers_preserve_partitioned_cookie_localstorage_and_indexeddb() {
        for browser in ["chrome", "edge", "firefox"].into_iter().filter(|name| {
            std::env::var("NODECLOAK_TEST_BROWSER")
                .map(|v| v == *name)
                .unwrap_or(true)
        }) {
            let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let part = |top: &str| json!({"topLevelSite":top,"hasCrossSiteAncestor":true});
            let state = |value: &str| json!({"local":[["session",value]],"databases":[{"name":"session-db","version":1,"stores":[{"name":"tokens","keyPath":null,"autoIncrement":false,"indexes":[],"rows":[[{"root":"session","nodes":[]},{"root":value,"nodes":[]}]]}]}]});
            let cookie = |value: &str, partition: Value| {
                let mut c = json!({"name":"session","value":value,"domain":"login.example","path":"/","secure":true,"httpOnly":true,"sameSite":"None","expires":chrono::Utc::now().timestamp()+604800});
                if !partition.is_null() {
                    if browser == "firefox" {
                        let host = partition["topLevelSite"]
                            .as_str()
                            .unwrap()
                            .strip_prefix("https://")
                            .unwrap();
                        c["firefoxPartitionKey"] = json!(format!(
                            "^partitionKey=%28https%2C{host}{}%29",
                            if value == "bounce" { "%2Cf" } else { "" }
                        ));
                        if value == "work" {
                            c["partitionedAttribute"] = json!(false);
                        }
                    }
                    c["partitionKey"] = partition;
                }
                c
            };
            let mut input=Snapshot::parse(json!({"schema":2,"cookies":[cookie("direct",Value::Null),cookie("shop",part("https://shop.example")),cookie("work",part("https://work.example"))],"origins":[
                {"origin":"https://login.example","state":state("direct")},
                {"origin":"https://login.example","partitionKey":part("https://shop.example"),"state":state("shop")},
                {"origin":"https://login.example","partitionKey":part("https://work.example"),"state":state("work")}
            ]})).unwrap();
            let bounce = part("https://login.example");
            input
                .cookies
                .push(serde_json::from_value(cookie("bounce", bounce.clone())).unwrap());
            input.origins.push(serde_json::from_value(json!({"origin":"https://login.example","partitionKey":bounce,"state":state("bounce")})).unwrap());
            let mut idb_only = state("idb-only");
            idb_only["local"] = json!([]);
            input.origins.push(serde_json::from_value(json!({"origin":"https://db.example","partitionKey":part("https://work.example"),"state":idb_only})).unwrap());
            let visible = input
                .cookies
                .iter()
                .map(|c| {
                    let mut c = c.clone();
                    c.name = "visible".into();
                    c.http_only = false;
                    c
                })
                .collect::<Vec<_>>();
            input.cookies.extend(visible);
            let mut default_cookie: Cookie =
                serde_json::from_value(cookie("default-policy", Value::Null)).unwrap();
            default_cookie.name = "default-policy".into();
            default_cookie.same_site = "Default".into();
            input.cookies.push(default_cookie);
            input = Snapshot::parse(serde_json::to_value(input).unwrap()).unwrap();
            let source = dir.path().join("source");
            restore_profile(browser, &source, &input)
                .unwrap_or_else(|e| panic!("{browser} initial partition import: {e}"));
            let exported = export_profile(browser, &source)
                .unwrap_or_else(|e| panic!("{browser} partition export: {e}"));
            assert_eq!(
                exported.cookies.len(),
                input.cookies.len(),
                "{browser} cookie count"
            );
            for site in &input.origins {
                let actual = exported.origins.iter().find(|s| s.key() == site.key());
                assert!(
                    actual.is_some(),
                    "{browser} missing namespace {}",
                    serde_json::to_value(site.key()).unwrap()
                );
                assert_eq!(
                    actual.unwrap().state,
                    site.state,
                    "{browser} namespace {}",
                    serde_json::to_value(site.key()).unwrap()
                );
            }
            assert_eq!(
                exported.bytes().unwrap().as_slice(),
                input.bytes().unwrap().as_slice(),
                "{browser} initial partition readback"
            );
            let target = dir.path().join("target");
            restore_profile(browser, &target, &exported)
                .unwrap_or_else(|e| panic!("{browser} second-profile restore: {e}"));
            let second = export_profile(browser, &target).unwrap();
            assert_eq!(
                second.bytes().unwrap().as_slice(),
                input.bytes().unwrap().as_slice(),
                "{browser} partition isolation across restart"
            );
            // Native document visibility must agree with the persisted metadata.
            let stage = Stage::new(browser, &target).unwrap();
            let mut check = Browser::launch(browser, &stage.path).unwrap();
            for site in input
                .origins
                .iter()
                .filter(|s| s.origin == "https://login.example")
            {
                check.site(&site.key()).unwrap();
                let visible = check.eval("document.cookie").unwrap();
                let raw = visible.as_str().unwrap();
                let expected = site.state["local"][0][1].as_str().unwrap();
                assert!(
                    raw.split("; ").any(|s| s == format!("visible={expected}")),
                    "{browser} {expected} native Cookie visibility: {raw}"
                );
                // Native policies may expose unpartitioned cookies as well.
                // Other partitions must remain isolated from this document.
                for other in ["shop", "work", "bounce"]
                    .into_iter()
                    .filter(|v| *v != expected)
                {
                    assert!(
                        !raw.split("; ").any(|s| s == format!("visible={other}")),
                        "{browser} leaked {other} into {expected}"
                    );
                }
            }
            check.close().unwrap();
            drop(stage);
            let mut invalid = input.clone();
            invalid.origins[0].state["local"] = json!([["session", "changed-before-failure"]]);
            invalid.origins.last_mut().unwrap().state["databases"][0]["stores"][0]["rows"][0][1] =
                json!({"root":{"r":99},"nodes":[]});
            assert!(
                restore_profile(browser, &target, &invalid).is_err(),
                "{browser} invalid structured clone accepted"
            );
            assert_eq!(
                export_profile(browser, &target).unwrap().bytes().unwrap(),
                input.bytes().unwrap(),
                "{browser} failed staged import changed original profile"
            );
            input.origins.retain(|s| {
                s.partition_key
                    .as_ref()
                    .is_none_or(|p| p.top_level_site != "https://shop.example")
            });
            input.cookies.retain(|c| {
                c.partition_key
                    .as_ref()
                    .is_none_or(|p| p.top_level_site != "https://shop.example")
            });
            restore_profile(browser, &target, &input).unwrap();
            let removed = export_profile(browser, &target).unwrap();
            assert_eq!(
                removed.bytes().unwrap().as_slice(),
                input.bytes().unwrap().as_slice(),
                "{browser} deleted partition cleared without changing other partitions"
            );
            println!("{browser}: partitioned Cookie, LocalStorage and IndexedDB migration/isolated deletion passed");
        }
    }
    #[test]
    fn browser_internal_partitions_do_not_block_website_storage_discovery() {
        for source in ["quota", "local_storage"] {
            let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let keys = [
                "https://assistant.example/",
                "https://login.example/^0https://shop.example",
                "https://assistant.example/^0edge://copilot-ntp",
                "https://assistant.example/^0edge://discover-chat-v2",
                "https://assistant.example/^0chrome://new-tab-page",
                "https://assistant.example/^0chrome-extension://aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ];
            if source == "quota" {
                std::fs::create_dir_all(dir.path().join("Default/WebStorage")).unwrap();
                let db = rusqlite::Connection::open(dir.path().join("Default/WebStorage/QuotaManager")).unwrap();
                db.execute_batch("CREATE TABLE buckets(storage_key TEXT NOT NULL,name TEXT NOT NULL)").unwrap();
                for key in keys {
                    db.execute("INSERT INTO buckets VALUES(?1,'_default')", [key]).unwrap();
                }
                db.execute("INSERT INTO buckets VALUES(?1,'internal-named')", ["https://assistant.example/^0edge://copilot-ntp"]).unwrap();
            } else {
                let path = dir.path().join("Default/Local Storage/leveldb");
                std::fs::create_dir_all(&path).unwrap();
                let mut db = rusty_leveldb::DB::open(&path, rusty_leveldb::Options::default()).unwrap();
                for key in keys {
                    db.put(format!("META:{key}").as_bytes(), b"synthetic metadata").unwrap();
                }
                db.flush().unwrap();
            }
            for browser in ["chrome", "edge"] {
                let found = serde_json::to_value(disk_origins(browser, dir.path())
                    .unwrap_or_else(|error| panic!("{browser}: {source}: {error}"))).unwrap();
                assert_eq!(found, json!([
                    {"origin":"https://assistant.example"},
                    {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://shop.example","hasCrossSiteAncestor":true}}
                ]), "{browser}: {source}: browser-internal partitions must not be flattened into website state");
            }
        }
        assert!(Snapshot::parse(json!({"schema":2,"cookies":[],"origins":[{"origin":"https://assistant.example","partitionKey":{"topLevelSite":"edge://copilot-ntp","hasCrossSiteAncestor":true},"state":{"local":[],"databases":[]}}]})).is_err());
    }
    #[test]
    fn partitioned_chromium_local_storage_discovery_keeps_top_level_sites_separate() {
        use rusty_leveldb::{Options, DB};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Default/Local Storage/leveldb");
        std::fs::create_dir_all(&path).unwrap();
        {
            let mut db = DB::open(&path, Options::default()).unwrap();
            for key in [
                "META:https://login.example/",
                "META:https://login.example/^0https://shop.example",
                "META:https://login.example/^0https://work.example",
            ] {
                db.put(key.as_bytes(), b"metadata").unwrap();
            }
            db.flush().unwrap();
        }
        let found = serde_json::to_value(disk_origins("chrome", dir.path()).unwrap()).unwrap();
        assert_eq!(
            found,
            json!([
                {"origin":"https://login.example"},
                {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://shop.example","hasCrossSiteAncestor":true}},
                {"origin":"https://login.example","partitionKey":{"topLevelSite":"https://work.example","hasCrossSiteAncestor":true}}
            ])
        );
    }
    #[test]
    #[ignore = "Installed Firefox fractional-expiry contract using only synthetic temporary Cookies"]
    fn installed_firefox_preserves_fractional_cookie_expiry() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let expiry = chrono::Utc::now().timestamp() as f64 + 604800.125;
        let input=Snapshot::parse(json!({"schema":2,"cookies":[{"name":"fractional","value":"synthetic","domain":"login.example","path":"/","secure":true,"httpOnly":true,"sameSite":"None","expires":expiry}],"origins":[]})).unwrap();
        let path = dir.path().join("profile");
        let actual = restore_profile("firefox", &path, &input).unwrap();
        assert_eq!(actual.cookies[0].expires, expiry);
        let exported = export_profile("firefox", &path).unwrap();
        assert_eq!(exported.cookies[0].expires, expiry);
    }
    #[test]
    fn interrupted_replacement_recovers_at_each_rename_boundary() {
        for phase in 0..4 {
            let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let profile = dir.path().join("profile");
            let stage = dir.path().join(".nodecloak-sync-test");
            std::fs::create_dir(&stage).unwrap();
            let target = match phase {
                0 => {
                    std::fs::create_dir(&profile).unwrap();
                    profile.clone()
                }
                1 => {
                    std::fs::create_dir(stage.join("previous")).unwrap();
                    stage.join("previous")
                }
                2 => {
                    std::fs::create_dir(&profile).unwrap();
                    std::fs::create_dir(stage.join("previous")).unwrap();
                    profile.clone()
                }
                _ => {
                    std::fs::create_dir(stage.join("profile")).unwrap();
                    stage.join("profile")
                }
            };
            std::fs::write(target.join("sentinel"), b"preserved").unwrap();
            engine::atomic_write(
                &journal_path(&profile).unwrap(),
                &serde_json::to_vec(
                    &json!({"stage":".nodecloak-sync-test","hadOriginal":phase!=3}),
                )
                .unwrap(),
            )
            .unwrap();
            recover(&profile).unwrap();
            assert_eq!(
                std::fs::read(profile.join("sentinel")).unwrap(),
                b"preserved"
            );
            assert!(!stage.exists());
            assert!(!journal_path(&profile).unwrap().exists());
        }
    }
    #[test]
    fn origin_parser_rejects_network_credentials_paths_and_nonweb_schemes() {
        for v in [
            "file:///C:/secret",
            "https://user:pass@example.com",
            "https://example.com/path",
            "https://example.com/?x=1",
            "javascript:alert(1)",
        ] {
            assert!(origin(v).is_err(), "{v}")
        }
        assert_eq!(
            origin("https://example.com:443/"),
            Ok("https://example.com".into())
        );
        assert_eq!(
            origin("http://localhost:18443"),
            Ok("http://localhost:18443".into())
        );
    }
    #[test]
    fn snapshot_validation_rejects_unknown_fields_and_unsafe_cookie_scope() {
        let mut v = json!({"schema":1,"cookies":[],"origins":[]});
        v["executablePath"] = json!("C:/evil.exe");
        assert!(Snapshot::parse(v).is_err());
        let v = json!({"schema":1,"cookies":[{"name":"x","value":"secret","domain":"../../evil","path":"/","secure":true,"httpOnly":true,"sameSite":"Lax","expires":-1}],"origins":[]});
        assert!(Snapshot::parse(v).is_err());
    }
    #[test]
    fn importing_cannot_touch_an_open_browser() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let profile = dir.path().join("profile");
        std::fs::create_dir_all(&profile).unwrap();
        #[cfg(windows)]
        {
            let _lock = std::fs::OpenOptions::new()
                .create(true)
                .truncate(true)
                .write(true)
                .share_mode(0)
                .open(profile.join("parent.lock"))
                .unwrap();
            assert!(restore_profile("firefox", &profile, &Snapshot::empty()).is_err());
        }
    }
    #[cfg(windows)]
    use std::os::windows::fs::OpenOptionsExt;
    #[test]
    #[ignore = "launches installed browsers against synthetic, temporary profiles only"]
    fn installed_edge_sync_ntp_origin_roundtrip() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let profile = dir.path().join("edge-ntp-fixture");
        let input = Snapshot::parse(json!({"schema":1,"cookies":[],"origins":[{
            "origin":"https://ntp.msn.com",
            "state":{"local":[["sync-probe","synthetic"]],"databases":[]}
        }]})).unwrap();
        restore_profile("edge", &profile, &input).unwrap();
        let exported = export_profile("edge", &profile).unwrap();
        assert_eq!(exported.origins.len(), 1);
        assert_eq!(exported.origins[0].state, input.origins[0].state);
    }
    #[test]
    #[ignore = "launches installed browsers against synthetic, temporary profiles only"]
    fn installed_chromium_reads_native_localstorage_writes() {
        for engine in ["chrome", "edge"] {
            let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let profile = dir.path().join("native-local-fixture");
            std::fs::create_dir(&profile).unwrap();
            let key = SiteKey { origin: "https://localhost:18443".into(), partition_key: None };
            let expected = json!({"local":[["", ""],["latin", "café"],["汉字😀", "a\u{0000}b😀"]],"databases":[]});
            super::super::session_local::write(&profile, &key, &expected).unwrap();
            let mut browser = Browser::launch(engine, &profile).unwrap();
            browser.site(&key).unwrap();
            let mut actual = browser.eval("__ncSessionCodec.exportOrigin()").unwrap();
            canonical_storage(&mut actual).unwrap();
            assert_eq!(actual, expected, "{engine}: native storage differs from browser API");
            browser.eval("localStorage.setItem('browser-written','真实值😀'); true").unwrap();
            browser.close().unwrap();
            let actual = super::super::session_local::read(&profile, &key).unwrap();
            assert!(actual["local"].as_array().unwrap().contains(&json!(["browser-written", "真实值😀"])), "{engine}: browser storage differs from native reader");
        }
    }
    #[test]
    #[ignore = "launches installed browsers against synthetic, temporary profiles only"]
    fn installed_edge_internal_partitions_preserve_website_and_internal_data() {
        use rusty_leveldb::{Options, DB};
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let profile = dir.path().join("synthetic-edge");
        let input = Snapshot::parse(json!({"schema":1,"cookies":[],"origins":[{
            "origin":"https://localhost:18443",
            "state":{"local":[["session","synthetic-login"]],"databases":[]}
        }]})).unwrap();
        restore_profile("edge", &profile, &input).unwrap();
        let db_path = profile.join("Default/Local Storage/leveldb");
        let internal = b"META:https://assistant.example/^0edge://copilot-ntp";
        {
            let mut db = DB::open(&db_path, Options::default()).unwrap();
            db.put(internal, b"synthetic-internal-metadata").unwrap();
            db.flush().unwrap();
        }
        let exported = export_profile("edge", &profile).unwrap();
        assert_eq!(exported.origins.len(), 1);
        assert_eq!(exported.origins[0].state, input.origins[0].state);
        let mut db = DB::open(&db_path, Options::default()).unwrap();
        assert_eq!(db.get(internal).as_deref(), Some(b"synthetic-internal-metadata".as_slice()));
    }
    #[test]
    #[ignore = "launches installed browsers against synthetic, temporary profiles only"]
    fn installed_browsers_roundtrip_cookie_localstorage_and_indexeddb() {
        for browser in ["chrome", "edge", "firefox"] {
            let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
            let profile = dir.path().join("synthetic");
            let token = format!("synthetic-{}", uuid::Uuid::new_v4());
            let packed = |v: Value| json!({"root":v,"nodes":[]});
            let input=Snapshot::parse(json!({"schema":1,"cookies":[{"name":"nc_synthetic","value":token,"domain":"localhost","path":"/","secure":true,"httpOnly":true,"sameSite":"Lax","expires":chrono::Utc::now().timestamp()+604800}],"origins":[{"origin":"https://localhost:18443","state":{"local":[["session",token]],"databases":[{"name":"nc-synthetic","version":1,"stores":[{"name":"kv","keyPath":null,"autoIncrement":false,"indexes":[],"rows":[[packed(json!("session")),packed(json!(token))]]}]}]}}]})).unwrap();
            restore_profile(browser, &profile, &input).unwrap_or_else(|e| panic!("{browser}: {e}"));
            let exported =
                export_profile(browser, &profile).unwrap_or_else(|e| panic!("{browser}: {e}"));
            assert!(
                exported.cookies.iter().any(|c| c.name == "nc_synthetic"
                    && c.value == token
                    && c.http_only
                    && c.secure),
                "{browser}: cookie missing"
            );
            assert_eq!(exported.origins.len(), 1, "{browser}: origin discovery");
            assert_eq!(
                exported.origins[0].state, input.origins[0].state,
                "{browser}: storage"
            );
            let target = dir.path().join("second-profile");
            restore_profile(browser, &target, &exported)
                .unwrap_or_else(|e| panic!("{browser}: {e}"));
            let second = export_profile(browser, &target).unwrap();
            assert_eq!(
                second.bytes().unwrap().as_slice(),
                exported.bytes().unwrap().as_slice(),
                "{browser}: restart persistence"
            );
        }
    }
}
