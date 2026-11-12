//! Native, loopback-only browser emulation. No page-level Date/Intl hooks.
use crate::{
    platform,
    profiles::PermissionMode,
    regional::{self, ResolvedRegion},
};
use serde_json::{json, Value};
use std::{
    collections::{HashMap, HashSet, VecDeque},
    net::TcpStream,
    path::{Path, PathBuf},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    time::{Duration, Instant},
};
use tungstenite::{stream::MaybeTlsStream, Message, WebSocket};
mod automation;
mod downloads;
mod recovery;
mod worker_setup;
#[cfg(test)]
mod recovery_tests;
#[cfg(test)]
mod separation_tests;
#[cfg(test)]
mod startup_tests;

pub(crate) fn log_capture_failure(path: &Path, browser: &str, attempt: usize, error: &str) {
    recovery::log(path, browser, "session_capture_failed", attempt, error);
}
pub(crate) fn log_startup_endpoint(path: &Path, browser: &str, event: &str, detail: &str) {
    recovery::log(path, browser, event, 0, detail);
}

/// The caller opts into automation explicitly; profile settings never grant it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum LaunchPurpose {
    Manual,
    Mcp,
}
#[derive(Clone, Copy, Debug)]
struct ControlPlan {
    profile_settings: bool,
    automation: bool,
}
impl ControlPlan {
    fn new(browser: &str, preferences: &crate::profiles::Preferences, purpose: LaunchPurpose) -> Self {
        Self {
            profile_settings: regional::needs_control_for(browser, preferences),
            automation: purpose == LaunchPurpose::Mcp,
        }
    }
}

#[derive(Default)]
pub struct Runtime(Mutex<HashMap<String, Control>>);
#[derive(Debug, Clone)]
pub enum OpenError {
    Navigation(String),
    Preparation(String),
    Control(String),
}
impl std::fmt::Display for OpenError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Navigation(s) | Self::Preparation(s) | Self::Control(s) => f.write_str(s),
        }
    }
}
impl From<String> for OpenError {
    fn from(s: String) -> Self {
        Self::Control(s)
    }
}
impl From<&str> for OpenError {
    fn from(s: &str) -> Self {
        Self::Control(s.into())
    }
}
impl Runtime {
    pub(crate) fn automation_ready(&self, id: &str) -> bool {
        self.0.lock().ok().and_then(|m| {
            m.get(id).map(|c| c.automation && c.alive.load(Ordering::SeqCst))
        }).unwrap_or(false)
    }
    pub fn ready(&self, id: &str) -> bool {
        self.0
            .lock()
            .ok()
            .and_then(|m| m.get(id).map(|c| c.alive.load(Ordering::SeqCst)))
            .unwrap_or(false)
    }
    pub fn error(&self, id: &str) -> Option<String> {
        self.0.lock().ok().and_then(|m| {
            m.get(id)
                .and_then(|c| c.error.lock().ok().and_then(|e| e.clone()))
        })
    }
    pub fn stop(&self, id: &str) {
        if let Ok(mut m) = self.0.lock() {
            m.remove(id);
        }
    }
    pub fn clear(&self) {
        if let Ok(mut m) = self.0.lock() {
            m.clear();
        }
    }
    pub fn insert(&self, id: String, control: Control) -> Result<(), String> {
        self.0
            .lock()
            .map_err(|_| "区域设置控制器不可用")?
            .insert(id, control);
        Ok(())
    }
    pub fn open(&self, id: &str, url: String, new_tab: bool) -> Result<(), OpenError> {
        let m = self.0.lock().map_err(|_| "区域设置控制器不可用")?;
        m.get(id)
            .ok_or("此副本区域设置尚未连接，请关闭并重新启动")?
            .open(url, new_tab)
    }
}
enum Request {
    Resume(mpsc::Sender<Result<bool,String>>),
    InvalidateDownloads(Option<String>),
    Automation(String,Value,crate::mcp::policy::Grant,Arc<AtomicBool>,mpsc::Sender<Result<Value,String>>),
    Open(String, bool, mpsc::Sender<Result<(), OpenError>>),
    Quit(mpsc::Sender<Result<(), String>>),
    #[cfg(test)]
    Probe(mpsc::Sender<Result<Value, String>>),
    #[cfg(test)]
    Disconnect(mpsc::Sender<()>),
    #[cfg(test)]
    ExitBrowser(mpsc::Sender<()>),
}
pub struct Control {
    automation: bool,
    sender: mpsc::Sender<Request>,
    stop: Arc<AtomicBool>,
    alive: Arc<AtomicBool>,
    error: Arc<Mutex<Option<String>>>,
    worker: Option<std::thread::JoinHandle<()>>,
}
impl Drop for Control {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}
impl Control {
    pub(crate) fn resume_previous(&self) -> Result<bool,String> {
        if self.automation {return Ok(false);}
        let (tx,rx)=mpsc::channel();
        self.sender.send(Request::Resume(tx)).map_err(|_|"区域控制连接已中断")?;
        rx.recv_timeout(Duration::from_secs(20)).map_err(|_|"恢复上次浏览器页面超时".to_string())?
    }
    /// Used only while retrying startup on about:blank; no user pages exist yet.
    pub(crate) fn quit_before_navigation(&self) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.sender
            .send(Request::Quit(tx))
            .map_err(|_| "无法关闭初始化浏览器")?;
        rx.recv_timeout(Duration::from_secs(20))
            .map_err(|_| "初始化浏览器关闭超时")?
    }
    fn open(&self, url: String, new_tab: bool) -> Result<(), OpenError> {
        if !self.alive.load(Ordering::SeqCst) {
            return Err("区域控制连接已中断，请重启副本".into());
        }
        let (tx, rx) = mpsc::channel();
        self.sender
            .send(Request::Open(url, new_tab, tx))
            .map_err(|_| "区域控制连接已中断")?;
        rx.recv_timeout(Duration::from_secs(20))
            .map_err(|_| "浏览器页面打开超时")?
    }
}
#[derive(PartialEq)]
enum StartupBarrier { None, Arming, Removing, Resuming, Disabling }
struct DocumentSetup {
    pending: HashMap<u64, &'static str>,
    resume: Option<Value>,
    started: Instant,
    delayed: bool,
    ready: bool,
    error: Option<String>,
    barrier: StartupBarrier,
    breakpoint: Option<String>,
    deferred: Option<Vec<(&'static str, Value)>>,
}
struct Protocol {
    socket: WebSocket<MaybeTlsStream<TcpStream>>,
    worker_setups: HashMap<String, worker_setup::WorkerSetup>,
    document_setups: HashMap<String, DocumentSetup>,
    barrier_owner: Option<String>,
    next: u64,
    events: VecDeque<Value>,
    last_error: Option<String>,
    navigation: Option<(u64, Option<Value>)>,
    reconnect_endpoint: Option<String>,
}
impl Protocol {
    // Initial empty targets can exist before a renderer is allocated. Queue
    // setup per session and keep pumping other targets instead of waiting here.
    fn configure_document(&mut self, session: &str, commands: Vec<(&'static str, Value)>) -> Result<(), String> {
        if let Some(error) = self.document_setups.get(session).and_then(|s|s.error.clone()) { return Err(error); }
        if let Some(setup) = self.document_setups.get_mut(session) {
            if setup.barrier != StartupBarrier::None {
                // Main-thread overrides cannot acknowledge during an instrumentation
                // pause (including while earlier replies are still unread). Finish
                // releasing this generation before applying the latest
                // desired settings; document readiness also waits for that next batch.
                setup.deferred = Some(commands);
                return Ok(());
            }
        }
        let updating = self.document_pending(session);
        let mut setup = if updating { self.document_setups.remove(session).unwrap() } else { DocumentSetup {
            pending: HashMap::new(), resume: None, started: Instant::now(),
            delayed: false, ready: false, error: None,
            barrier: StartupBarrier::None, breakpoint: None, deferred: None,
        } };
        for (method, params) in commands {
            if method == "Runtime.runIfWaitingForDebugger" {
                if !updating { setup.resume = Some(params); }
                continue;
            }
            // Final resolved settings can arrive while provisional startup
            // settings are pending. Append their acknowledgements to the same
            // gate so the target cannot resume with the provisional settings.
            self.next += 1;
            setup.pending.insert(self.next, method);
            self.socket.send(Message::Text(json!({"id":self.next,"method":method,"params":params,"sessionId":session}).to_string().into()))
                .map_err(|e| format!("无法写入浏览器区域设置 {method}（{}）", recovery::transport_error(&e)))?;
        }
        self.document_setups.insert(session.into(), setup);
        self.advance_document(session)
    }
    fn document_pending(&self, session: &str) -> bool {
        self.document_setups.get(session).is_some_and(|setup| !setup.ready)
    }
    fn setup_command(&mut self, session: &str, method: &'static str, params: Value) -> Result<(), String> {
        self.next += 1;
        self.document_setups.get_mut(session).unwrap().pending.insert(self.next,method);
        self.socket.send(Message::Text(json!({"id":self.next,"method":method,"params":params,"sessionId":session}).to_string().into()))
            .map_err(|e|format!("无法写入浏览器初始化命令 {method}（{}）",recovery::transport_error(&e)))
    }
    fn advance_document(&mut self, session: &str) -> Result<(), String> {
        let setup = &self.document_setups[session];
        if setup.error.is_some() || setup.ready { return Ok(()); }
        let protection_ready = setup.pending.values().all(|method| *method == "Page.enable");
        if setup.resume.is_some() && !protection_ready && setup.started.elapsed() >= Duration::from_millis(150)
            && setup.barrier == StartupBarrier::None && self.barrier_owner.is_none() {
            // New noopener windows cannot allocate their renderer until startup
            // resumes. Arm a native first-script breakpoint in the queued CDP
            // batch before releasing that browser-side navigation throttle.
            // Serialize these barriers: debugger pauses can span page sessions
            // sharing a renderer, so another target must never release this one.
            self.barrier_owner = Some(session.into());
            self.document_setups.get_mut(session).unwrap().barrier = StartupBarrier::Arming;
            self.setup_command(session,"Debugger.enable",json!({}))?;
            self.setup_command(session,"Debugger.setInstrumentationBreakpoint",json!({"instrumentation":"beforeScriptExecution"}))?;
        }
        let setup = &mut self.document_setups.get_mut(session).unwrap();
        if setup.resume.is_some() && (protection_ready || setup.barrier == StartupBarrier::Arming) {
            let params = setup.resume.take().unwrap();
            self.setup_command(session,"Runtime.runIfWaitingForDebugger",params)?;
        }
        let setup = &self.document_setups[session];
        if setup.resume.is_none() && setup.pending.is_empty() {
            match setup.barrier {
                StartupBarrier::Arming => {
                    let breakpoint = setup.breakpoint.clone().ok_or("页面初始化保护断点无效")?;
                    self.document_setups.get_mut(session).unwrap().barrier = StartupBarrier::Removing;
                    self.setup_command(session,"Debugger.removeBreakpoint",json!({"breakpointId":breakpoint}))?;
                }
                StartupBarrier::Removing => {
                    self.document_setups.get_mut(session).unwrap().barrier = StartupBarrier::Resuming;
                    self.setup_command(session,"Debugger.resume",json!({}))?;
                }
                StartupBarrier::Resuming => {
                    // A page instrumentation pause only processes IO commands.
                    // Resume it before sending Debugger.disable on the main lane.
                    self.document_setups.get_mut(session).unwrap().barrier = StartupBarrier::Disabling;
                    self.setup_command(session,"Debugger.disable",json!({}))?;
                }
                StartupBarrier::Disabling => {
                    self.document_setups.get_mut(session).unwrap().barrier = StartupBarrier::None;
                    self.barrier_owner = None;
                }
                StartupBarrier::None => {},
            }
            let setup = self.document_setups.get_mut(session).unwrap();
            if setup.pending.is_empty() && setup.barrier == StartupBarrier::None {
                setup.ready = true;
                if let Some(commands) = setup.deferred.take() {
                    let delayed = setup.delayed;
                    self.configure_document(session, commands)?;
                    self.document_setups.get_mut(session).unwrap().delayed |= delayed;
                } else if setup.delayed {
                    self.events.push_back(json!({"method":"NodeCloak.documentSetupReady","sessionId":session}));
                }
            }
        }
        Ok(())
    }
    fn route_document_reply(&mut self, value: &Value) -> Result<bool, String> {
        let Some(id) = value["id"].as_u64() else { return Ok(false) };
        let Some(session) = self.document_setups.iter().find_map(|(session, setup)| setup.pending.contains_key(&id).then(||session.clone())) else { return Ok(false) };
        let setup = self.document_setups.get_mut(&session).unwrap();
        let method = setup.pending.remove(&id).unwrap();
        if method == "Debugger.setInstrumentationBreakpoint" && value.get("error").is_none() {
            setup.breakpoint = value["result"]["breakpointId"].as_str().map(str::to_owned);
            if setup.breakpoint.is_none() {
                setup.error = Some("页面初始化保护断点无效".into());
                self.events.push_back(json!({"method":"NodeCloak.documentSetupFailed","sessionId":session,"params":{"error":"页面初始化保护断点无效"}}));
            }
        }
        let error = if let Some(error) = value.get("error") {
            let message = error["message"].as_str().unwrap_or("");
            let inherited = (method == "Debugger.resume" && message.trim_end_matches('.') == "Can only perform operation while paused")
                || (method == "Emulation.setLocaleOverride" && message == "Another locale override is already in effect")
                || (method == "Emulation.setTimezoneOverride" && message.trim_end_matches('.') == "Timezone override is already in effect");
            (!inherited).then(|| format!("浏览器拒绝控制命令 {method}（code={}）", error["code"].as_i64().map(|v|v.to_string()).unwrap_or_else(||"unknown".into())))
        } else if value["result"].get("exceptionDetails").is_some() {
            Some(format!("浏览器区域设置执行失败：{method}"))
        } else { None };
        if let Some(error) = error {
            if setup.error.is_none() {
                setup.error = Some(error.clone());
                self.events.push_back(json!({"method":"NodeCloak.documentSetupFailed","sessionId":session,"params":{"error":error}}));
            }
        }
        self.advance_document(&session)?;
        Ok(true)
    }
    fn check_document_delays(&mut self) {
        self.check_worker_delays();
        for (session, setup) in &mut self.document_setups {
            if !setup.ready && setup.error.is_none() && !setup.delayed && setup.started.elapsed() >= Duration::from_secs(15) {
                setup.delayed = true;
                let mut pending: Vec<_> = setup.pending.values().copied().collect();
                pending.sort_unstable();
                self.events.push_back(json!({"method":"NodeCloak.documentSetupDelayed","sessionId":session,"params":{"pending":pending}}));
            }
        }
    }
    fn read(&mut self) -> Result<Option<Value>, String> {
        let result = match self.socket.read() {
            Ok(Message::Text(text)) => {
                let value: Value = serde_json::from_str(&text).map_err(|_| "浏览器控制响应格式无效")?;
                if self.route_worker_reply(&value)? || self.route_document_reply(&value)? { Ok(None) } else { Ok(Some(value)) }
            },
            Ok(Message::Close(frame)) => Err(format!("浏览器控制连接关闭（WebSocket Close，code={}）", frame.map(|f| u16::from(f.code)).unwrap_or(1005))),
            Ok(_) => Ok(None),
            Err(tungstenite::Error::Io(e))
                if matches!(
                    e.kind(),
                    std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
                ) =>
            {
                Ok(None)
            }
            Err(e) => Err(format!("浏览器控制连接中断（{}）", recovery::transport_error(&e))),
        };
        if result.is_ok() {
            for session in self.document_setups.keys().cloned().collect::<Vec<_>>() {
                self.advance_document(&session)?;
            }
        }
        result
    }
    // UI activation can wait indefinitely for OS visibility in Firefox background windows.
    // It is independent of the acknowledged region settings and navigation commands.
    fn activate(&mut self, context: &str) -> Result<(), String> {
        self.next += 1;
        self.socket.send(Message::Text(json!({"id":self.next,"method":"browsingContext.activate","params":{"context":context}}).to_string().into())).map_err(|_|"无法切换浏览器标签页".into())
    }
    fn call(
        &mut self,
        method: &str,
        params: Value,
        session: Option<&str>,
    ) -> Result<Value, String> {
        self.last_error = None;
        self.next += 1;
        let id = self.next;
        let mut message = json!({"id":id,"method":method,"params":params});
        if let Some(s) = session {
            message["sessionId"] = json!(s)
        }
        self.socket
            .send(Message::Text(message.to_string().into()))
            .map_err(|e| format!("无法写入浏览器区域设置 {method}（{}）", recovery::transport_error(&e)))?;
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(if method=="Page.enable"{5}else{15}) {
            if let Some(value) = self.read()? {
                if value["method"]=="Target.detachedFromTarget"&&session.is_some_and(|s|value["params"]["sessionId"]==s){self.events.push_back(value);self.last_error=Some("TARGET_GONE".into());return Err("TARGET_GONE".into());}
                if value["id"] == id {
                    if value.get("error").is_some() {
                        self.last_error = value["error"]["message"]
                            .as_str()
                            .or_else(|| value["message"].as_str())
                            .or_else(|| value["error"].as_str())
                            .map(str::to_owned);
                        let code = value["error"]["code"].as_i64().map(|n|n.to_string())
                            .or_else(|| value["error"].as_str().map(str::to_owned)).unwrap_or_else(|| "unknown".into());
                        let reason = if self.last_error.as_deref().is_some_and(|m|m.contains("Maximum number of active sessions")) {
                            "；现有 Firefox BiDi 会话仍在运行，当前浏览器未开放会话重连"
                        } else { "" };
                        return Err(format!("浏览器拒绝控制命令 {method}（code={code}）{reason}"));
                    }
                    if value["type"] == "exception"
                        || value["result"]["type"] == "exception"
                        || value["result"].get("exceptionDetails").is_some()
                    {
                        let details = &value["result"]["exceptionDetails"];
                        let detail = details["exception"]["description"]
                            .as_str()
                            .or_else(|| details["exception"]["value"].as_str())
                            .or_else(|| details["text"].as_str())
                            .or_else(|| value["result"]["exception"]["description"].as_str())
                            .or_else(|| value["result"]["message"].as_str())
                            .unwrap_or("");
                        let detail = detail
                            .lines()
                            .next()
                            .unwrap_or("")
                            .trim()
                            .chars()
                            .take(240)
                            .collect::<String>();
                        return Err(if detail.is_empty() {
                            "浏览器区域匹配执行失败".into()
                        } else {
                            format!("浏览器区域匹配执行失败（{detail}）")
                        });
                    }
                    return Ok(value["result"].clone());
                }
                if value.get("id").is_none() {
                    self.events.push_back(value);
                } else if let Some((waiting, reply)) = &mut self.navigation {
                    if value["id"] == *waiting {
                        *reply = Some(value);
                    }
                }
                if self.events.len() > 4096 {
                    return Err("浏览器控制事件过多".into());
                }
            }
        }
        Err(format!("浏览器区域设置 {method} 超时"))
    }
}
pub(crate) fn endpoint(browser: &str, path: &Path) -> Result<String, String> {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(15) {
        let endpoint = if browser == "firefox" {
            std::fs::read(path.join("WebDriverBiDiServer.json"))
                .ok()
                .and_then(|b| serde_json::from_slice::<Value>(&b).ok())
                .and_then(|v| {
                    let host = v["ws_host"].as_str()?;
                    if !["127.0.0.1", "localhost", "::1"].contains(&host) {
                        return None;
                    }
                    let port = v["ws_port"].as_u64().filter(|p| *p > 0 && *p <= 65535)?;
                    Some(format!("ws://127.0.0.1:{port}/session"))
                })
        } else {
            std::fs::read_to_string(path.join("DevToolsActivePort"))
                .ok()
                .and_then(|text| {
                    let mut lines = text.lines();
                    let port = lines.next()?.parse::<u16>().ok().filter(|p| *p > 0)?;
                    let route = lines.next()?.strip_prefix("/devtools/browser/")?;
                    if route.is_empty()
                        || !route
                            .bytes()
                            .all(|b| b.is_ascii_alphanumeric() || b == b'-')
                    {
                        return None;
                    }
                    Some(format!("ws://127.0.0.1:{port}/devtools/browser/{route}"))
                })
        };
        if let Some(url) = endpoint {
            return Ok(url);
        }
        std::thread::sleep(Duration::from_millis(100));
    }
    Err("等待此副本的本地配置接口超时；请重新启动，诊断信息已记录到日志".into())
}
struct BrowserControl {
    diagnostic: Option<(PathBuf, String)>,
    plan: ControlPlan,
    protocol: Protocol,
    firefox: bool,
    region: ResolvedRegion,
    permission: PermissionMode,
    primary: String,
    sessions: HashSet<String>,
    targets: HashMap<String, String>,
    workers: HashSet<String>,
    pages: HashSet<String>,
    advanced: crate::profiles::AdvancedSettings,
    fingerprint: Option<crate::fingerprint::Settings>,
    fingerprint_sessions: HashSet<String>,
    internal_workers: HashSet<String>,
    session_checked: Instant,
    session_capture_ready: bool,
    restore_queue: VecDeque<(String, bool)>,
    normal_context: Option<String>,
    automation_fatal:Option<String>,
    automation_dialogs:HashMap<String,Value>,
    automation_warnings:Vec<String>,
    automation_cleanup:Vec<automation::Cleanup>,
    download_lease:Option<downloads::Lease>,
    download_subscribed:bool,
    download_checked:Instant,
}
impl BrowserControl {
    fn resume_previous(&mut self) -> Result<bool,String> {
        if self.firefox || self.plan.automation {return Ok(false);}
        if self.diagnostic.as_ref().is_some_and(|(_,browser)|crate::session_restore::uses_tab_urls(browser)) {
            let path=&self.diagnostic.as_ref().unwrap().0;
            let saved=crate::session_restore::read_tabs(path)?;
            // Acknowledge a bounded, local queue before any network wait. A
            // slow saved page must not exhaust the startup caller's deadline.
            self.restore_queue = saved.into_iter().enumerate().map(|(index,url)|(url,index>0)).collect();
            self.session_capture_ready = false;
            return Ok(!self.restore_queue.is_empty());
        }
        let bootstrap=self.primary.clone();
        let existing=self.pages.clone();
        // Native restore may replace an initial empty tab's WebContents before
        // the new-target pause is installed (notably on Edge). Keep our helper
        // non-empty so the browser restores into separate, fenced targets.
        const HELPER:&str="data:text/html;charset=utf-8,%3Ctitle%3ENodeCloak%3C/title%3E";
        self.open(HELPER,false).map_err(|e|e.to_string())?;
        // Deliver the browser's own Reopen Closed Window command only after
        // auto-attach and profile settings are ready. Cold startup restore can
        // execute cached documents before we have attached to the browser.
        let (modifiers,native)=if cfg!(target_os="macos") {(12,17)} else if cfg!(windows) {(10,84)} else {(10,28)};
        let mut sent=HashSet::new();
        for kind in ["rawKeyDown","keyUp"] {
            self.protocol.next+=1;let id=self.protocol.next;sent.insert(id);
            self.protocol.socket.send(Message::Text(json!({"id":id,"method":"Input.dispatchKeyEvent",
                "sessionId":bootstrap,"params":{"type":kind,"key":"T","code":"KeyT","modifiers":modifiers,"windowsVirtualKeyCode":84,"nativeVirtualKeyCode":native}}).to_string().into()))
                .map_err(|_|"无法请求恢复上次浏览器页面".to_string())?;
        }
        let deadline=Instant::now()+Duration::from_secs(8);
        let mut saw_restored=false;
        while Instant::now()<deadline {
            self.drain()?;
            let restored_pages:Vec<_>=self.pages.difference(&existing).cloned().collect();
            saw_restored|=!restored_pages.is_empty();
            if let Some(restored)=restored_pages.first().filter(|_|restored_pages.iter().all(|s|!self.protocol.document_pending(s))).cloned() {
                self.primary=restored;
                // Never close a real user page or a first-run prompt. The
                // normal startup launcher creates exactly this blank helper.
                if let Some(target)=self.targets.iter().find_map(|(id,session)|(session==&bootstrap).then(||id.clone())) {
                    let info=self.protocol.call("Target.getTargetInfo",json!({"targetId":target}),None)?;
                    if info["targetInfo"]["url"]==HELPER {
                        self.protocol.call("Target.closeTarget",json!({"targetId":target}),None)?;
                    }
                }
                if let Some((path,browser))=&self.diagnostic {recovery::log(path,browser,"session_restored",0,"native saved browser session reopened after profile settings");}
                return Ok(true);
            }
            if let Some(event)=self.protocol.read()? {
                if event["id"].as_u64().is_some_and(|id|sent.contains(&id)) {
                    if event.get("error").is_some() {return Ok(false);}
                } else {self.protocol.events.push_back(event);}
            }
        }
        Ok(saw_restored)
    }
    fn restore_next_tab(&mut self) -> Result<(),String> {
        let Some((url,new_tab))=self.restore_queue.pop_front() else {return Ok(())};
        match self.open(&url,new_tab) {
            Ok(())=>{},
            Err(OpenError::Preparation(message))=>{
                // No destination was loaded. Retain the original saved URLs
                // instead of recording the still-blank initialization tab.
                self.restore_queue.clear();
                self.session_capture_ready=false;
                if let Some((path,browser))=&self.diagnostic {recovery::log(path,browser,"session_restore_pending",0,&message);}
                return Ok(());
            },
            Err(OpenError::Navigation(message))=>{
                if let Some((path,browser))=&self.diagnostic {
                    // Persist only a network error code, never a saved URL.
                    let code=message.split(|c:char| !(c.is_ascii_alphanumeric() || matches!(c,':'|'_'))).find(|value| value.starts_with("net::ERR_") && value.bytes().all(|c|c.is_ascii_alphanumeric() || matches!(c,b':'|b'_'))).unwrap_or("saved page navigation unavailable");
                    recovery::log(path,browser,"session_page_unavailable",0,code);
                }
            },
            Err(OpenError::Control(message))=>{
                // Do not replay an operation whose effect is uncertain. Keep
                // the original full snapshot for a later explicit restart.
                self.restore_queue.clear();
                self.session_capture_ready=false;
                return Err(message);
            }
        }
        if self.restore_queue.is_empty(){self.session_capture_ready=true;}
        Ok(())
    }
    fn save_open_tabs(&mut self) -> Result<(),String> {
        self.session_checked=Instant::now();
        let Some((path,browser))=&self.diagnostic else{return Ok(())};
        if !self.restore_queue.is_empty() || !self.session_capture_ready || self.plan.automation || !crate::session_restore::uses_tab_urls(browser) {return Ok(())}
        let Some(normal)=&self.normal_context else{return Ok(())};
        let targets=self.protocol.call("Target.getTargets",json!({}),None)?;
        let Some(urls)=crate::session_restore::normal_tabs(&targets,normal) else{return Ok(())};
        crate::session_restore::write_tabs(path,&urls)
    }
    fn initialize(
        browser: &str,
        path: &Path,
        region: ResolvedRegion,
        permission: PermissionMode,
        match_ip: bool,
        lookup_url: &str,
        advanced: crate::profiles::AdvancedSettings,
        fingerprint: Option<crate::fingerprint::Settings>,
        deny_downloads: bool,
        plan: ControlPlan,
    ) -> Result<Self, String> {
        let endpoint = endpoint(browser, path)?;
        let socket = recovery::connect_socket(&endpoint)?;
        let mut this = Self {
            diagnostic: Some((path.to_path_buf(), browser.to_owned())),
            plan,
            protocol: Protocol {
                worker_setups:HashMap::new(),document_setups:HashMap::new(),barrier_owner:None,socket,
                next: 0,
                events: VecDeque::new(),
                last_error: None,
                navigation: None,
                reconnect_endpoint: None,
            },
            firefox: browser == "firefox",
            region,
            permission,
            primary: String::new(),
            sessions: HashSet::new(),
            targets: HashMap::new(),
            workers: HashSet::new(),
            pages: HashSet::new(),
            advanced,
            fingerprint,
            fingerprint_sessions: HashSet::new(),
            internal_workers: HashSet::new(),
            session_checked: Instant::now(), session_capture_ready: false, restore_queue: VecDeque::new(),
            normal_context: None,
            automation_fatal:None,
            automation_dialogs:HashMap::new(),
            automation_warnings:Vec::new(),
            automation_cleanup:Vec::new(),
            download_lease:None,
            download_subscribed:false,
            download_checked:Instant::now(),
        };
        if this.firefox {
            let session = this.protocol.call(
                "session.new",
                json!({"capabilities":{"alwaysMatch":{"acceptInsecureCerts":false,"unhandledPromptBehavior":"ignore","webSocketUrl":true}}}),
                None,
            )?;
            // A session id is an authentication secret: keep it in memory, never log it.
            if let Some(id) = session["sessionId"].as_str().filter(|id| id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')) {
                this.protocol.reconnect_endpoint = Some(format!("{endpoint}/{id}"));
            }
            if deny_downloads { this.download_policy_after_reconnect()?; }
            if this.plan.automation {
                this.protocol.call("session.subscribe",json!({"events":["browsingContext.userPromptOpened","browsingContext.userPromptClosed","browsingContext.contextCreated"]}),None)?;
            }
            if let Some(settings) = this.fingerprint.as_ref().filter(|f| f.enabled) {
                this.protocol.call("script.addPreloadScript",json!({"functionDeclaration":crate::fingerprint::preload(settings)?,"userContexts":["default"]}),None)?;
            }
            let tree =
                this.protocol
                    .call("browsingContext.getTree", json!({"maxDepth":0}), None)?;
            // First-run, recovery and restored tabs can precede our startup tab.
            // Firefox's privileged internal pages reject script.evaluate. Keep
            // those tabs intact and use only an explicitly loaded blank document.
            this.primary = match tree["contexts"].as_array().and_then(|contexts| {
                contexts.iter().find(|context| context["url"] == "about:blank" && context["userContext"] == "default")
                    .and_then(|context| context["context"].as_str())
            }) {
                Some(context) => context.to_owned(),
                None => this.protocol.call("browsingContext.create", json!({"type":"tab","userContext":"default"}), None)?
                    ["context"].as_str().ok_or("无法创建 Firefox 启动标签页")?.to_owned(),
            };
            this.protocol.call("browsingContext.navigate",
                json!({"context":this.primary,"url":"about:blank","wait":"complete"}), None)?;
        } else {
            if deny_downloads { this.download_policy_after_reconnect()?; }
            this.protocol.call("Target.setAutoAttach",this.auto_attach(true),None)?;
            this.drain()?;
            let started = Instant::now();
            while this.primary.is_empty() {
                if started.elapsed() >= Duration::from_secs(15) { return Err("Chrome / Edge 初始页面尚未就绪".into()); }
                if let Some(event) = this.protocol.read()? { this.protocol.events.push_back(event); }
                this.drain()?;
            }
        }
        if match_ip {
            // Query inside the actual browser so PAC/TUN/extension routing is respected.
            // Try the configured endpoint first, then a fallback with the same fields.
            let urls = json!(regional::lookup_urls(lookup_url)).to_string();
            let expression = r#"(async()=>{const urls=__URLS__;let last='';for(const url of urls){try{const r=await fetch(url,{credentials:'omit',cache:'no-store',signal:AbortSignal.timeout(6000)});if(!r.ok)throw Error('status '+r.status);const t=await r.text();if(t.length>65536)throw Error('region too large');const v=JSON.parse(t);if(v.success!==true)throw Error('region rejected');const tz=v.timezone&&typeof v.timezone==='object'?v.timezone.id:v.timezone;return JSON.stringify({success:true,ip:v.ip,country_code:v.country_code,city:v.city,latitude:v.latitude,longitude:v.longitude,timezone:{id:tz}});}catch(e){last=String(e);}}throw Error('region lookup failed: '+last);})()"#
                .replace("__URLS__", &urls);
            let call = if this.firefox {
                this.protocol.call("script.evaluate",json!({"expression":expression,"target":{"context":this.primary},"awaitPromise":true}),None)
            } else {
                this.protocol.call(
                    "Runtime.evaluate",
                    json!({"expression":expression,"awaitPromise":true,"returnByValue":true}),
                    Some(&this.primary),
                )
            };
            let result = call
                .map_err(|error| format!("{error}；请检查代理是否允许访问区域查询服务，或改用自定义区域设置"))?
                ["result"]["value"]
                .clone();
            let data: Value = serde_json::from_str(
                result
                    .as_str()
                    .ok_or("无法在副本浏览器内匹配 IP 区域，请检查代理或使用自定义设置")?,
            )
            .map_err(|_| "浏览器 IP 区域响应无效")?;
            this.region.ip_region = Some(regional::parse_ip_region(&data)?);
        }
        Ok(this)
    }
    fn apply(&mut self, session: Option<&str>) -> Result<(), String> {
        if !self.plan.profile_settings {
            return Ok(());
        }
        if !self.firefox {
            if let Some(session) = session {
                return if self.workers.contains(session) { self.apply_worker(session, false) } else { self.apply_document(session) };
            }
        }
        let locale = self.region.language.split(',').next().unwrap_or("en-US");
        if self.firefox {
            self.protocol.call(
                "emulation.setLocaleOverride",
                json!({"locale":locale,"userContexts":["default"]}),
                None,
            )?;
            if self.advanced.viewport_width > 0 {
                self.protocol.call("browsingContext.setViewport",json!({"userContexts":["default"],"viewport":{"width":self.advanced.viewport_width,"height":self.advanced.viewport_height},"devicePixelRatio":1}),None)?;
            }
            if let Some(timezone) = &self.region.timezone {
                self.protocol.call(
                    "emulation.setTimezoneOverride",
                    json!({"timezone":timezone,"userContexts":["default"]}),
                    None,
                )?;
            }
            if let (Some(latitude), Some(longitude)) = (self.region.latitude, self.region.longitude)
            {
                self.protocol.call("emulation.setGeolocationOverride",json!({"coordinates":{"latitude":latitude,"longitude":longitude,"accuracy":self.region.accuracy},"userContexts":["default"]}),None)?;
            }
        } else {
            if self.advanced.hardware_concurrency > 0 {
                self.protocol.call(
                    "Emulation.setHardwareConcurrencyOverride",
                    json!({"hardwareConcurrency":self.advanced.hardware_concurrency}),
                    session,
                )?;
            }
            if self.advanced.viewport_width > 0 && session.is_some_and(|s| self.pages.contains(s)) {
                self.protocol.call("Emulation.setDeviceMetricsOverride",json!({"width":self.advanced.viewport_width,"height":self.advanced.viewport_height,"deviceScaleFactor":1,"mobile":false}),session)?;
            }
            // A popup's initial about:blank inherits the opener's Intl values,
            // but those are not persistent overrides for this target. Configure
            // every target directly without executing JS in a paused renderer.
            {
                if let Err(error) = self.protocol.call(
                    "Emulation.setLocaleOverride",
                    json!({"locale":locale}),
                    session,
                ) {
                    // Chromium shares Intl overrides with same-process frames. A newly paused
                    // iframe may not have an execution context yet; its parent owns this override.
                    if self.protocol.last_error.as_deref()
                        != Some("Another locale override is already in effect")
                    {
                        return Err(error);
                    }
                }
            }
            if let Some(timezone) = &self.region.timezone {
                {
                    if let Err(error) = self.protocol.call(
                        "Emulation.setTimezoneOverride",
                        json!({"timezoneId":timezone}),
                        session,
                    ) {
                        if self
                            .protocol
                            .last_error
                            .as_deref()
                            .map(|e| e.trim_end_matches('.'))
                            != Some("Timezone override is already in effect")
                        {
                            return Err(error);
                        }
                    }
                }
            }
            if let (Some(latitude), Some(longitude)) = (
                self.region
                    .latitude
                    .filter(|_| session.is_some_and(|s| self.pages.contains(s))),
                self.region.longitude,
            ) {
                self.protocol.call("Emulation.setGeolocationOverride",json!({"latitude":latitude,"longitude":longitude,"accuracy":self.region.accuracy}),session)?;
            }
        }
        Ok(())
    }
    fn apply_worker(&mut self, session: &str, waiting: bool) -> Result<(), String> {
        let mut commands = vec![("Emulation.setLocaleOverride", json!({"locale":self.region.language.split(',').next().unwrap_or("en-US")}))];
        if self.advanced.hardware_concurrency > 0 {
            commands.push(("Emulation.setHardwareConcurrencyOverride", json!({"hardwareConcurrency":self.advanced.hardware_concurrency})));
        }
        if let Some(timezone) = &self.region.timezone {
            commands.push(("Emulation.setTimezoneOverride", json!({"timezoneId":timezone})));
        }
        commands.push(("Target.setAutoAttach", self.auto_attach(false)));
        let script = self.fingerprint.as_ref().filter(|f|f.enabled)
            .filter(|_|!self.internal_workers.contains(session) && !self.fingerprint_sessions.contains(session))
            .map(crate::fingerprint::script).transpose()?;
        self.protocol.configure_worker(session, commands, script, waiting)?;
        self.fingerprint_sessions.insert(session.into());
        Ok(())
    }
    fn apply_document(&mut self, session: &str) -> Result<(), String> {
        let mut commands = Vec::new();
        let preload = self.fingerprint.as_ref().filter(|f| f.enabled)
            .filter(|_| !self.fingerprint_sessions.contains(session));
        if let Some(settings) = preload {
            commands.push(("Page.enable", json!({})));
            commands.push(("Page.addScriptToEvaluateOnNewDocument", json!({"source":crate::fingerprint::script(settings)?})));
        }
        if self.advanced.hardware_concurrency > 0 {
            commands.push(("Emulation.setHardwareConcurrencyOverride", json!({"hardwareConcurrency":self.advanced.hardware_concurrency})));
        }
        if self.advanced.viewport_width > 0 && self.pages.contains(session) {
            commands.push(("Emulation.setDeviceMetricsOverride",json!({"width":self.advanced.viewport_width,"height":self.advanced.viewport_height,"deviceScaleFactor":1,"mobile":false})));
        }
        let locale = self.region.language.split(',').next().unwrap_or("en-US");
        commands.push(("Emulation.setLocaleOverride", json!({"locale":locale})));
        if let Some(timezone) = &self.region.timezone {
            commands.push(("Emulation.setTimezoneOverride", json!({"timezoneId":timezone})));
        }
        if let (Some(latitude), Some(longitude)) = (self.region.latitude.filter(|_| self.pages.contains(session)), self.region.longitude) {
            commands.push(("Emulation.setGeolocationOverride",json!({"latitude":latitude,"longitude":longitude,"accuracy":self.region.accuracy})));
        }
        commands.push(("Target.setAutoAttach", self.auto_attach(false)));
        commands.push(("Runtime.runIfWaitingForDebugger", json!({})));
        self.protocol.configure_document(session, commands)?;
        if preload.is_some() { self.fingerprint_sessions.insert(session.into()); }
        Ok(())
    }
    fn configure(&mut self) -> Result<(), String> {
        if !self.plan.profile_settings {
            return Ok(());
        }
        if self.firefox {
            self.apply(None)
        } else {
            self.protocol.call("Browser.setPermission",json!({"permission":{"name":"geolocation"},"setting":match self.permission{PermissionMode::Allow=>"granted",PermissionMode::Block=>"denied",_=>"prompt"}}),None)?;
            for session in self.sessions.clone() {
                self.apply(Some(&session))?;
            }
            let started = Instant::now();
            loop {
                if self.primary.is_empty() || !self.pages.contains(&self.primary) { return Err("浏览器初始页面已关闭，请重新启动副本".into()); }
                if !self.protocol.document_pending(&self.primary) { break; }
                if let Some(error) = self.protocol.document_setups.get(&self.primary).and_then(|s|s.error.clone()) { return Err(error); }
                if started.elapsed() >= Duration::from_secs(15) { return Err("浏览器初始页面设置尚未就绪".into()); }
                if let Some(event) = self.protocol.read()? { self.protocol.events.push_back(event); }
                self.drain()?;
            }
            Ok(())
        }
    }
    fn drain(&mut self) -> Result<(), String> {
        self.protocol.check_document_delays();
        while let Some(event) = self.protocol.events.pop_front() {
            if self.plan.automation {
                self.download_event(&event);
                if event["method"]=="Page.javascriptDialogClosed"||event["method"]=="browsingContext.userPromptClosed"{self.forget_dialog(&event);continue;}
                if event["method"]=="Page.javascriptDialogOpening"||event["method"]=="browsingContext.userPromptOpened"{self.remember_dialog(&event);continue;}
            }
            if event["method"] == "Target.attachedToTarget" {
                let params = &event["params"];

                let session = params["sessionId"]
                    .as_str()
                    .ok_or("浏览器附加页面无效")?
                    .to_owned();
                if let Some(target) = params["targetInfo"]["targetId"].as_str() {
                    self.targets.insert(target.into(), session.clone());
                }
                if !["page", "iframe"]
                    .contains(&params["targetInfo"]["type"].as_str().unwrap_or(""))
                {
                    self.workers.insert(session.clone());
                    if params["targetInfo"]["url"].as_str().is_some_and(|url| {
                        url.starts_with("chrome-extension:")
                            || url.starts_with("chrome:")
                            || url.starts_with("devtools:")
                    }) {
                        self.internal_workers.insert(session.clone());
                    }
                }
                if params["targetInfo"]["type"] == "page" {
                    self.pages.insert(session.clone());
                }
                if self.sessions.insert(session.clone()) {
                    if self.primary.is_empty() && params["targetInfo"]["type"] == "page" {
                        self.primary = session.clone();
                        if self.normal_context.is_none(){self.normal_context=params["targetInfo"]["browserContextId"].as_str().map(str::to_owned);}
                    }
                    // Documents initialize asynchronously; Worker commands can
                    // observe a normal detach at any point in this sequence.
                    let setup: Result<(), String> = (|| {
                        if self.plan.profile_settings && self.workers.contains(&session) {
                            self.apply_worker(&session, params["waitingForDebugger"].as_bool().unwrap_or(false))?;
                        } else { self.apply(Some(&session))?; }
                        if !self.plan.profile_settings {
                            self.protocol.call("Target.setAutoAttach",self.auto_attach(false),Some(&session))?;
                        }
                        Ok(())
                    })();
                    if let Err(error) = setup {
                        // Protocol::call queues the confirmed detach event. Let
                        // the common handler clean up this target, preserving
                        // the other pages and their protection state.
                        if error == "TARGET_GONE" { continue; }
                        return Err(error);
                    }
                }
            } else if event["method"].as_str().is_some_and(|method| method.starts_with("NodeCloak.documentSetup") || method.starts_with("NodeCloak.workerSetup")) {
                let method = event["method"].as_str().unwrap();
                let worker = method.starts_with("NodeCloak.workerSetup");
                let detail = if method.ends_with("Delayed") {
                    format!("{}; phase={}; commands={}", if worker { "background task setup pending; other targets remain available" } else { "renderer setup pending; tab retained and protection pause preserved" }, if worker { event["params"]["phase"].as_str().unwrap_or("unknown") } else { "document" }, event["params"]["pending"])
                } else if method.ends_with("Failed") {
                    format!("{}; {}", event["params"]["error"].as_str().unwrap_or("setup rejected"), if worker { "background task settings not confirmed; other targets retained" } else { "tab retained without releasing protection pause" })
                } else { "delayed renderer initialized; original settings applied".into() };
                if let Some((path, browser)) = &self.diagnostic {
                    recovery::log(path, browser, match (worker, method.rsplit("Setup").next().unwrap_or("")) {
                        (true,"Delayed")=>"worker_setup_delayed", (true,"Failed")=>"worker_setup_failed", (true,_)=>"worker_setup_ready",
                        (false,"Delayed")=>"target_setup_delayed", (false,"Failed")=>"target_setup_failed", _=>"target_setup_ready",
                    }, 0, &detail);
                }
                if method.ends_with("Failed") && self.automation_warnings.len() < 16 { self.automation_warnings.push(detail); }
            } else if event["method"] == "Target.detachedFromTarget" {
                if let Some(session) = event["params"]["sessionId"].as_str() {
                    self.protocol.document_setups.remove(session);
                    self.protocol.worker_setups.remove(session);
                    if self.protocol.barrier_owner.as_deref() == Some(session) { self.protocol.barrier_owner = None; }
                    self.sessions.remove(session);
                    self.workers.remove(session);
                    self.pages.remove(session);
                    self.fingerprint_sessions.remove(session);
                    self.internal_workers.remove(session);
                    self.targets.retain(|_, attached| attached != session);
                    self.automation_dialogs.remove(session);
                    if self.primary == session {
                        self.primary = self.pages.iter().filter(|s| !self.protocol.document_pending(s)).next()
                            .or_else(||self.pages.iter().next()).cloned().unwrap_or_default();
                    }
                }
            }
        }
        Ok(())
    }
    fn auto_attach(&self, include_pages: bool) -> Value {
        let mut filter = vec![];
        if include_pages {
            filter.push(json!({"type":"page","exclude":false}));
        }
        filter.push(json!({"type":"iframe","exclude":false}));
        if self.plan.profile_settings {
            for kind in ["worker", "shared_worker", "service_worker"] {
                filter.push(json!({"type":kind,"exclude":false}));
            }
        }
        filter.push(json!({"exclude":true}));
        json!({"autoAttach":true,"waitForDebuggerOnStart":self.plan.profile_settings,"flatten":true,"filter":filter})
    }
    fn handle_open(
        &mut self,
        url: &str,
        new_tab: bool,
        response: mpsc::Sender<Result<(), OpenError>>,
    ) -> Result<(), String> {
        let result = self.open(url, new_tab);
        // The startup helper is not the user session. Capture starts only after
        // restoration or the first requested navigation has completed.
        match &result {
            Err(OpenError::Preparation(_) | OpenError::Control(_))=>{},
            _=>self.session_capture_ready=true,
        }
        let fatal = match &result {
            Err(OpenError::Control(message)) => Some(message.clone()),
            _ => None,
        };
        let _ = response.send(result);
        if let Some(message) = fatal {
            Err(message)
        } else {
            Ok(())
        }
    }
    fn open(&mut self, url: &str, new_tab: bool) -> Result<(), OpenError> {
        if self.firefox {
            let context = if new_tab {
                self.protocol.call(
                    "browsingContext.create",
                    json!({"type":"tab","referenceContext":self.primary,"background":true}),
                    None,
                )?["context"]
                    .as_str()
                    .ok_or("无法创建 Firefox 页面")?
                    .to_owned()
            } else {
                self.primary.clone()
            };
            self.protocol.activate(&context)?;
            if let Ok(parsed) = reqwest::Url::parse(url) {
                if ["http", "https"].contains(&parsed.scheme()) {
                    self.protocol.call("permissions.setPermission",json!({"descriptor":{"name":"geolocation"},"state":match self.permission{PermissionMode::Allow=>"granted",PermissionMode::Block=>"denied",_=>"prompt"},"origin":parsed.origin().ascii_serialization(),"userContext":"default"}),None)?;
                }
            }
            match self.protocol.call(
                "browsingContext.navigate",
                json!({"context":context,"url":url,"wait":"none"}),
                None,
            ) {
                Err(_) if self.protocol.last_error.is_some() => {
                    return Err(OpenError::Navigation(
                        "浏览器页面打开失败，请检查代理连接和网址".into(),
                    ))
                }
                Err(error) => return Err(error.into()),
                Ok(_) => {}
            }
        } else {
            let session = if new_tab {
                let target = self.protocol.call(
                    "Target.createTarget",
                    json!({"url":"about:blank"}),
                    None,
                )?["targetId"]
                    .as_str()
                    .ok_or("无法创建浏览器页面")?
                    .to_owned();
                let started = Instant::now();
                loop {
                    self.drain()?;
                    if let Some(session) = self.targets.get(&target) {
                        break session.clone();
                    }
                    if started.elapsed() > Duration::from_secs(5) {
                        return Err("新标签页的区域设置未就绪".into());
                    }
                    if let Some(event) = self.protocol.read()? {
                        self.protocol.events.push_back(event)
                    }
                }
            } else {
                self.primary.clone()
            };
            if let Some(target) = self
                .targets
                .iter()
                .find_map(|(target, attached)| (attached == &session).then(|| target.clone()))
            {
                self.protocol
                    .call("Target.activateTarget", json!({"targetId":target}), None)?;
            }
            // Attachment only identifies the target. Its renderer overrides
            // and first-script barrier can still be pending, especially when
            // Edge allocates a new renderer for a restored page. Activation
            // must precede this wait so background targets can initialize.
            let preparation_started=Instant::now();
            while self.protocol.document_pending(&session) {
                self.drain()?;
                if let Some(error)=self.protocol.document_setups.get(&session).and_then(|setup|setup.error.clone()) {
                    return Err(OpenError::Preparation(error));
                }
                if !self.protocol.document_pending(&session) {break;}
                if preparation_started.elapsed()>=Duration::from_secs(15) {
                    return Err(OpenError::Preparation("新标签页的区域设置未就绪".into()));
                }
                if let Some(event)=self.protocol.read()? {self.protocol.events.push_back(event);}
            }
            // Page.navigate can wait for a newly auto-attached frame or worker.
            // Pump those targets while waiting; a plain synchronous call deadlocks.
            self.protocol.next += 1;
            let id = self.protocol.next;
            self.protocol.navigation = Some((id, None));
            self.protocol.socket.send(Message::Text(json!({"id":id,"method":"Page.navigate","params":{"url":url},"sessionId":session}).to_string().into()))
                .map_err(|_| "无法发送浏览器页面打开请求")?;
            let result = (|| {
                let started = Instant::now();
                loop {
                    self.drain()?;
                    if let Some((_, Some(reply))) = self.protocol.navigation.as_mut() {
                        let reply = reply.take();
                        if reply.get("error").is_some() {
                            return Err(OpenError::Navigation(
                                "浏览器拒绝打开页面，请检查网址并重试".into(),
                            ));
                        }
                        if let Some(error) = reply["result"]["errorText"]
                            .as_str()
                            .filter(|s| !s.is_empty())
                        {
                            return Err(OpenError::Navigation(format!(
                                "浏览器页面打开失败：{error}；请检查代理连接和网址"
                            )));
                        }
                        return Ok(());
                    }
                    if started.elapsed() > Duration::from_secs(15) {
                        return Err("浏览器页面打开超时，请检查代理连接并重试".into());
                    }
                    if let Some(event) = self.protocol.read()? {
                        if event["id"] == id {
                            self.protocol.navigation.as_mut().unwrap().1 = Some(event);
                        } else if event.get("id").is_none() {
                            self.protocol.events.push_back(event);
                        }
                    }
                }
            })();
            self.protocol.navigation = None;
            result?;
        }
        Ok(())
    }
}
pub(crate) fn start_with_lookup(
    browser: String,
    path: PathBuf,
    preferences: crate::profiles::Preferences,
    initial: ResolvedRegion,
    lookup_url: String,
    purpose: LaunchPurpose,
) -> Result<(Control, ResolvedRegion), String> {
    let plan = ControlPlan::new(&browser, &preferences, purpose);
    let (sender, requests) = mpsc::channel();
    let (ready_tx, ready_rx) = mpsc::channel();
    let stop = Arc::new(AtomicBool::new(false));
    let alive = Arc::new(AtomicBool::new(false));
    let error = Arc::new(Mutex::new(None));
    let mut control = Control {
        automation: plan.automation,
        sender,
        stop: stop.clone(),
        alive: alive.clone(),
        error: error.clone(),
        worker: None,
    };
    let worker = std::thread::spawn(move || {
        let boot = BrowserControl::initialize(
            &browser,
            &path,
            initial,
            preferences.advanced.location.clone(),
            regional::uses_ip(&preferences),
            &lookup_url,
            preferences.advanced.clone(),
            preferences.fingerprint.clone(),
            false,
            plan,
        )
        .and_then(|mut state| {
            state.region = regional::resolve(&preferences, state.region.ip_region.clone())?;
            state.configure()?;
            state.drain()?;
            Ok(state)
        });
        let mut state = match boot {
            Ok(state) => state,
            Err(e) => {
                recovery::log(&path,&browser,"startup_failed",0,&e);
                let _ = ready_tx.send(Err(e));
                return;
            }
        };
        alive.store(true, Ordering::SeqCst);
        if ready_tx.send(Ok(state.region.clone())).is_err() {
            return;
        }
        recovery::log(&path, &browser, "connected", 0, "browser control ready");
        let mut recent_recoveries = VecDeque::new();
        let run = loop {
          let result = (|| -> Result<(), String> {
            while !stop.load(Ordering::SeqCst) {
                if state.session_checked.elapsed()>=Duration::from_secs(2) {
                    if let Err(error)=state.save_open_tabs(){recovery::log(&path,&browser,"session_capture_failed",0,&error);}
                }
                if let Ok(request) = requests.try_recv() {
                    match request {
                        Request::Resume(response)=>{
                            let result=state.resume_previous();
                            if matches!(result, Ok(true)) && state.restore_queue.is_empty() {state.session_capture_ready=true;}
                            let _=response.send(result);
                        }
                        Request::InvalidateDownloads(client)=>state.invalidate_downloads(client.as_deref())?,
                        Request::Automation(tool,args,grant,cancel,response)=>{
                            let mut result=state.automate(&tool,args,&grant,&cancel);
                            if let Ok(value)=&mut result{if !state.automation_warnings.is_empty(){value["warnings"]=json!(std::mem::take(&mut state.automation_warnings));}}
                            let _=response.send(result);
                            if let Some(error)=state.automation_fatal.take(){return Err(error);}
                        }
                        Request::Quit(response) => {
                            let result = state
                                .protocol
                                .call(
                                    if state.firefox {
                                        "browser.close"
                                    } else {
                                        "Browser.close"
                                    },
                                    json!({}),
                                    None,
                                )
                                .map(|_| ());
                            let _ = response.send(result);
                            return Ok(());
                        }
                        Request::Open(url, new_tab, response) => {
                            state.handle_open(&url, new_tab, response)?;
                        }
                        #[cfg(test)]
                        Request::ExitBrowser(response) => {
                            let _ = state.protocol.call(if state.firefox { "browser.close" } else { "Browser.close" },json!({}),None);
                            let _ = response.send(());
                        }
                        #[cfg(test)]
                        Request::Disconnect(response) => {
                            if let MaybeTlsStream::Plain(stream) = state.protocol.socket.get_mut() { let _ = stream.shutdown(std::net::Shutdown::Both); }
                            let _ = response.send(());
                        }
                        #[cfg(test)]
                        Request::Probe(response) => {
                            let _=response.send(state.protocol.call("Runtime.evaluate",json!({"expression":"({url:location.href,ready:document.readyState,body:document.body.innerText,first:typeof first!=='undefined'?first:null,progress:window.progress})","returnByValue":true}),Some(&state.primary)));
                        }
                    }
                }
                state.restore_next_tab()?;
                state.drain()?;
                if state.plan.automation { state.cleanup_closed_dialogs(); }
                if state.download_lease.is_some() && state.download_checked.elapsed() >= Duration::from_secs(1) {
                    state.download_checked = Instant::now();
                    let _ = state.maintain_downloads(&AtomicBool::new(false));
                }
                if let Some(event) = state.protocol.read()? {
                    state.protocol.events.push_back(event)
                }
            }
            Ok(())
          })();
          let Err(message) = result else { break Ok(()); };
          alive.store(false, Ordering::SeqCst);
          match recovery::recover(&mut state, &browser, &path, &stop, &message, &mut recent_recoveries) {
              Ok(true) => { alive.store(true, Ordering::SeqCst); }
              Ok(false) => break Ok(()),
              Err(error) => break Err(error),
          }
        };
        alive.store(false, Ordering::SeqCst);
        if let Err(message) = run {
            if !stop.load(Ordering::SeqCst)
                && platform::profile_process_ids(&path)
                    .map(|pids| !pids.is_empty())
                    .unwrap_or(true)
            {
                if let Ok(mut e) = error.lock() {
                    *e = Some(format!("{message}；已关闭此副本，请重新启动"));
                }
                recovery::log(&path, &browser, "recovery_exhausted", 0, &message);
                if let Err(close_error) = platform::close_profile(&path) {
                    recovery::log(&path, &browser, "close_failed", 0, &close_error);
                }
            }
        }
    });
    control.worker = Some(worker);
    let region = ready_rx
        .recv_timeout(Duration::from_secs(60))
        .map_err(|_| "区域设置准备超时")??;
    Ok((control, region))
}

#[cfg(test)]
mod tests {
    include!("browser_control/resume_checks.rs");
    use super::*;
    use crate::{
        profiles::{self, Draft, Preferences},
        proxy::ProxyConfig,
    };
    use std::{
        io::{Read, Write},
        net::TcpListener,
    };
    #[cfg(windows)]
    unsafe extern "system" fn show_test_window(
        hwnd: windows_sys::Win32::Foundation::HWND,
        state: isize,
    ) -> i32 {
        use windows_sys::Win32::UI::WindowsAndMessaging::*;
        let ids = &*(state as *const Vec<u32>);
        let mut pid = 0;
        GetWindowThreadProcessId(hwnd, &mut pid);
        if ids.contains(&pid) && IsWindowVisible(hwnd) != 0 {
            // Only the synthetic fixture's native window. No browser API overrides.
            SetWindowPos(
                hwnd,
                HWND_TOPMOST,
                40,
                40,
                0,
                0,
                SWP_SHOWWINDOW | SWP_NOSIZE,
            );
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
        1
    }
    fn navigation_fixture(fail: bool) -> (BrowserControl, std::thread::JoinHandle<()>) {
        navigation_fixture_with_worker(fail, None)
    }
    fn navigation_fixture_with_worker(fail: bool, detach_at: Option<&str>) -> (BrowserControl, std::thread::JoinHandle<()>) {
        let detach_at = detach_at.map(str::to_owned);
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(18)))
                .unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            let navigation: Value =
                serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
            assert_eq!(navigation["method"], "Page.navigate");
            if fail {
                ws.send(Message::Text(json!({"id":navigation["id"],"result":{"errorText":"net::ERR_PROXY_CONNECTION_FAILED"}}).to_string().into())).unwrap();
                if let Ok(Message::Text(raw)) = ws.read() {
                    let retry: Value = serde_json::from_str(&raw).unwrap();
                    assert_eq!(retry["method"], "Page.navigate");
                    ws.send(Message::Text(
                        json!({"id":retry["id"],"result":{"frameId":"main"}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                }
                return;
            }
            if let Some(detach_at) = detach_at {
                ws.send(Message::Text(json!({"method":"Target.attachedToTarget","params":{"sessionId":"short-worker","targetInfo":{"targetId":"worker","type":"worker"},"waitingForDebugger":true}}).to_string().into())).unwrap();
                let mut detached = false;
                while let Ok(Message::Text(raw)) = ws.read() {
                    let command: Value = serde_json::from_str(&raw).unwrap();
                    if detach_at == "delayed-worker" && command["method"] == "Debugger.enable" {
                        ws.send(Message::Text(json!({"id":navigation["id"],"result":{"frameId":"main"}}).to_string().into())).unwrap();
                        detached = true;
                    } else if !detached && command["method"] == detach_at {
                        ws.send(Message::Text(json!({"method":"Target.detachedFromTarget","params":{"sessionId":"short-worker"}}).to_string().into())).unwrap();
                        ws.send(Message::Text(json!({"id":navigation["id"],"result":{"frameId":"main"}}).to_string().into())).unwrap();
                        detached = true;
                    } else {
                        ws.send(Message::Text(json!({"id":command["id"],"result":{"breakpointId":"worker-first-script"}}).to_string().into())).unwrap();
                    }
                }
                assert!(detached, "worker stage not reached: {detach_at}");
                return;
            }
            ws.send(Message::Text(json!({"method":"Target.attachedToTarget","params":{"sessionId":"child","targetInfo":{"targetId":"frame","type":"iframe"},"waitingForDebugger":true}}).to_string().into())).unwrap();
            loop {
                let command = match ws.read() {
                    Ok(Message::Text(raw)) => serde_json::from_str::<Value>(&raw).unwrap(),
                    _ => break,
                };
                let method = command["method"].as_str().unwrap();
                let result = if method == "Runtime.evaluate" {
                    json!({"result":{"value":{"locale":"en-US"}}})
                } else {
                    json!({})
                };
                ws.send(Message::Text(
                    json!({"id":command["id"],"result":result})
                        .to_string()
                        .into(),
                ))
                .unwrap();
                if method == "Runtime.runIfWaitingForDebugger" {
                    ws.send(Message::Text(
                        json!({"id":navigation["id"],"result":{"frameId":"main"}})
                            .to_string()
                            .into(),
                    ))
                    .unwrap();
                    break;
                }
            }
        });
        let (mut socket, _) = tungstenite::connect(format!("ws://{address}")).unwrap();
        if let MaybeTlsStream::Plain(stream) = socket.get_mut() {
            stream
                .set_read_timeout(Some(Duration::from_millis(100)))
                .unwrap();
        }
        (
            BrowserControl {
                diagnostic:None,plan: ControlPlan { profile_settings: true, automation: false },
                protocol: Protocol {
                    worker_setups:HashMap::new(),document_setups:HashMap::new(),barrier_owner:None,socket,
                    next: 0,
                    events: VecDeque::new(),
                    last_error: None,
                    navigation: None,
                reconnect_endpoint: None,
                },
                firefox: false,
                region: regional::initial(&Preferences::default()).unwrap(),
                permission: PermissionMode::Ask,
                primary: "main".into(),
                sessions: HashSet::new(),
                targets: HashMap::new(),
                workers: HashSet::new(),
                pages: HashSet::new(),
                advanced: Default::default(),
                fingerprint: None,
                fingerprint_sessions: HashSet::new(),
                    internal_workers: HashSet::new(),
                                    session_checked: Instant::now(), session_capture_ready: false, restore_queue: VecDeque::new(),
            normal_context: None,
            automation_fatal:None,
            automation_dialogs:HashMap::new(),
            automation_warnings:Vec::new(),
            automation_cleanup:Vec::new(),
            download_lease:None,
            download_subscribed:false,
            download_checked:Instant::now(),
            },
            server,
        )
    }
    #[test]
    fn delayed_worker_initialization_keeps_other_pages_responsive() {
        let (mut state, server) = navigation_fixture_with_worker(false, Some("delayed-worker"));
        state.fingerprint = Some(crate::fingerprint::fresh().unwrap());
        let started = Instant::now();
        let navigation = state.open("https://example.invalid/", false);
        let responsive = state.protocol.call("Runtime.evaluate", json!({"expression":"1"}), Some("main"));
        drop(state);
        server.join().unwrap();
        assert!(navigation.is_ok(), "one delayed Worker interrupted navigation: {navigation:?}");
        assert!(responsive.is_ok(), "one delayed Worker interrupted another page");
        assert!(started.elapsed() < Duration::from_secs(3), "Worker setup blocked the protocol pump");
    }
    #[test]
    fn terminated_worker_during_setup_does_not_interrupt_browser_control() {
        for method in ["Debugger.enable", "Emulation.setLocaleOverride", "Target.setAutoAttach", "Runtime.runIfWaitingForDebugger"] {
            let (mut state, server) = navigation_fixture_with_worker(false, Some(method));
            state.fingerprint = Some(crate::fingerprint::fresh().unwrap());
            let result = state.open("https://example.invalid/", false);
            let drained = state.drain();
            let alive = state.protocol.call("Runtime.evaluate", json!({"expression":"1"}), Some("main"));
            let worker_removed = !state.sessions.contains("short-worker") && !state.workers.contains("short-worker") && !state.targets.contains_key("worker");
            drop(state);
            server.join().unwrap();
            assert!(result.is_ok(), "{method}: {result:?}");
            assert!(drained.is_ok() && alive.is_ok() && worker_removed, "{method}: worker cleanup or surviving page failed");
        }
    }
    #[test]
    fn navigation_resumes_new_frames_before_waiting_for_navigation_acknowledgement() {
        let (mut state, server) = navigation_fixture(false);
        let result = state.open("https://example.invalid/", false);
        drop(state);
        server.join().unwrap();
        assert!(result.is_ok(), "{result:?}");
    }
    #[test]
    fn detached_primary_reselects_a_surviving_page() {
        let (mut state, server) = navigation_fixture(false);
        state.open("https://example.invalid/", false).unwrap();
        state.sessions.extend(["main".into(),"replacement".into()]);
        state.pages.extend(["main".into(),"replacement".into()]);
        state.protocol.events.push_back(json!({"method":"Target.detachedFromTarget","params":{"sessionId":"main"}}));
        state.drain().unwrap();
        assert_eq!(state.primary, "replacement");
        state.protocol.events.push_back(json!({"method":"Target.detachedFromTarget","params":{"sessionId":"replacement"}}));
        state.drain().unwrap();
        assert!(state.primary.is_empty());
        drop(state);
        server.join().unwrap();
    }
    #[test]
    fn navigation_reports_browser_network_errors_instead_of_claiming_success() {
        let (mut state, server) = navigation_fixture(true);
        let result = state.open("https://example.invalid/", false);
        drop(state);
        server.join().unwrap();
        assert!(result
            .unwrap_err()
            .to_string()
            .contains("ERR_PROXY_CONNECTION_FAILED"));
    }
    #[test]
    fn page_network_failure_keeps_controller_usable_for_a_second_navigation() {
        let (mut state, server) = navigation_fixture(true);
        let (tx, rx) = mpsc::channel();
        assert!(state
            .handle_open("https://example.invalid/", false, tx)
            .is_ok());
        assert!(rx.recv().unwrap().is_err());
        assert!(state.session_capture_ready);
        let (tx, rx) = mpsc::channel();
        assert!(state
            .handle_open("https://example.invalid/retry", false, tx)
            .is_ok());
        assert!(rx.recv().unwrap().is_ok());
        drop(state);
        server.join().unwrap();
    }
    #[test]
    fn saved_page_network_failure_keeps_remaining_recovery_and_capture_fenced() {
        let (mut state, server) = navigation_fixture(true);
        state.restore_queue = VecDeque::from([
            ("https://example.invalid/".into(), false),
            ("https://example.invalid/retry".into(), false),
        ]);
        assert!(state.restore_next_tab().is_ok());
        assert_eq!(state.restore_queue.len(), 1);
        assert!(!state.session_capture_ready);
        assert!(state.restore_next_tab().is_ok());
        assert!(state.restore_queue.is_empty());
        assert!(state.session_capture_ready);
        drop(state);
        server.join().unwrap();
    }
    #[test]
    fn firefox_page_network_errors_are_recoverable_without_losing_the_controller() {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let address = listener.local_addr().unwrap();
        let server = std::thread::spawn(move || {
            let (stream, _) = listener.accept().unwrap();
            let mut ws = tungstenite::accept(stream).unwrap();
            for i in 0..6 {
                let command: Value =
                    serde_json::from_str(ws.read().unwrap().to_text().unwrap()).unwrap();
                let reply = if i == 2 {
                    json!({"id":command["id"],"type":"error","error":"unknown error","message":"NS_ERROR_NET_TIMEOUT"})
                } else {
                    json!({"id":command["id"],"type":"success","result":{}})
                };
                ws.send(Message::Text(reply.to_string().into())).unwrap();
            }
        });
        let (socket, _) = tungstenite::connect(format!("ws://{address}")).unwrap();
        let mut state = BrowserControl {
            diagnostic:None,plan: ControlPlan { profile_settings: true, automation: false },
            protocol: Protocol {
                worker_setups:HashMap::new(),document_setups:HashMap::new(),barrier_owner:None,socket,
                next: 0,
                events: VecDeque::new(),
                last_error: None,
                navigation: None,
                reconnect_endpoint: None,
            },
            firefox: true,
            region: regional::initial(&Preferences::default()).unwrap(),
            permission: PermissionMode::Ask,
            primary: "main".into(),
            sessions: HashSet::new(),
            targets: HashMap::new(),
            workers: HashSet::new(),
            pages: HashSet::new(),
            advanced: Default::default(),
            fingerprint: None,
            fingerprint_sessions: HashSet::new(),
            internal_workers: HashSet::new(),
            session_checked: Instant::now(), session_capture_ready: false, restore_queue: VecDeque::new(),
            normal_context: None,
            automation_fatal:None,
            automation_dialogs:HashMap::new(),
            automation_warnings:Vec::new(),
            automation_cleanup:Vec::new(),
            download_lease:None,
            download_subscribed:false,
            download_checked:Instant::now(),
        };
        let (tx, rx) = mpsc::channel();
        assert!(state
            .handle_open("https://example.invalid/", false, tx)
            .is_ok());
        assert!(matches!(rx.recv().unwrap(), Err(OpenError::Navigation(_))));
        let (tx, rx) = mpsc::channel();
        assert!(state
            .handle_open("https://example.invalid/retry", false, tx)
            .is_ok());
        assert!(rx.recv().unwrap().is_ok());
        drop(state);
        server.join().unwrap();
    }
    #[test]
    #[ignore = "Starts isolated installed browsers; verifies first document, iframe, worker, viewport, DNT and graphics/voice API restrictions"]
    fn native_privacy_settings_are_observed_in_all_browsers() {
        let fixture = Fixture::new();
        for engine in ["chrome", "edge", "firefox"].into_iter().filter(|name| {
            std::env::var("CLAUDE_DONE_TEST_BROWSER")
                .map(|s| s == *name)
                .unwrap_or(true)
        }) {
            let mut prefs:Preferences=serde_json::from_value(json!({"language":"en-US,en","privacy":true,"fontRestriction":false,"startupUrl":"about:blank","advanced":{"location":"allow","doNotTrack":"on","webRtc":"public","blockWebgl":true,"blockWebgpu":true,"blockSpeech":true,"hardwareConcurrency":4,"viewportWidth":1280,"viewportHeight":800},"regional":{"enabled":true,"locationMode":"custom","latitude":1.3521,"longitude":103.8198}})).unwrap();
            let first = browser(engine, prefs.clone(), &fixture.url("/region"));
            first.control.open(fixture.url("/privacy"), false).unwrap();
            let report = fixture.report(&first);
            assert_eq!(report["hardware"], 4, "{engine}: {report}");
            assert_eq!(report["worker"]["hardware"], 4, "{engine}: worker {report}");
            assert_eq!(report["frame"]["hardware"], 4, "{engine}: frame {report}");
            assert_eq!(report["dnt"], "1", "{engine}: {report}");
            assert_eq!(report["width"], 1280, "{engine}: {report}");
            assert_eq!(report["height"], 800, "{engine}: {report}");
            assert_eq!(report["webgl"], false, "{engine}: {report}");
            assert_eq!(report["webgpu"], false, "{engine}: {report}");
            assert_eq!(report["voices"], 0, "{engine}: {report}");
            prefs.advanced = Default::default();
            prefs.advanced.location = PermissionMode::Allow;
            let second = browser(engine, prefs, &fixture.url("/region"));
            second.control.open(fixture.url("/normal"), false).unwrap();
            let natural = fixture.report(&second);
            assert_ne!(
                natural["hardware"], 4,
                "{engine}: native baseline {natural}"
            );
        }
    }
    #[test]
    fn fixture_does_not_send_a_page_before_receiving_a_complete_request() {
        let fixture = Fixture::new();
        for request in [b"".as_slice(), b"GET /separation HTTP/1.1\r\nHost: localhost\r\n".as_slice()] {
            let mut stream = TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
            stream.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
            stream.write_all(request).unwrap();
            let mut response = [0; 256];
            assert_eq!(stream.read(&mut response).unwrap(), 0,
                "speculative or incomplete connections must not receive executable HTML");
        }
        let mut stream = TcpStream::connect(("127.0.0.1", fixture.port)).unwrap();
        stream.set_read_timeout(Some(Duration::from_secs(6))).unwrap();
        stream.write_all(b"GET /favicon.ico HTTP/1.1\r\nHost: localhost\r\n\r\n").unwrap();
        let mut response = String::new();
        stream.read_to_string(&mut response).unwrap();
        assert!(response.starts_with("HTTP/1.1 404"));
        assert!(!response.contains("<script>"));
    }
    struct Fixture {
        port: u16,
        reports: mpsc::Receiver<Value>,
        stop: Arc<AtomicBool>,
    }
    const FINGERPRINT_PROBE: &str = r#"async function fingerprintProbe(){
      const c=document.createElement('canvas');c.width=64;c.height=32;const x=c.getContext('2d');x.fillStyle='#735941';x.fillRect(0,0,64,32);x.font='14px serif';x.fillText('NodeCloak',2,19);
      const hash=data=>{let h=2166136261;for(const v of data)h=Math.imul(h^v,16777619);return h>>>0;};
      const raw=()=>hash(x.getImageData(0,0,64,32).data);const first=raw(),again=raw();const metric=x.measureText('NodeCloak');
      const ctx=new OfflineAudioContext(1,512,44100);const osc=ctx.createOscillator();osc.type='triangle';osc.frequency.value=1000;osc.connect(ctx.destination);osc.start();const audio=await ctx.startRendering();
      const gl=document.createElement('canvas').getContext('webgl');let webgl=null;if(gl){gl.clearColor(.3,.4,.5,1);gl.clear(gl.COLOR_BUFFER_BIT);const out=new Uint8Array(4);gl.readPixels(0,0,1,1,gl.RGBA,gl.UNSIGNED_BYTE,out);webgl=[...out];}
      const exported=await new Promise(resolve=>c.toBlob(async blob=>{const image=await createImageBitmap(blob);const copy=document.createElement('canvas');copy.width=64;copy.height=32;copy.getContext('2d').drawImage(image,0,0);resolve(hash(copy.getContext('2d').getImageData(0,0,64,32).data));}));
      // Decode export in an unhooked bitmap path to avoid applying readback noise twice.
      const blank=document.createElement('canvas').getContext('2d').getImageData(0,0,1,1).data;
      let invalid=false;try{x.getImageData(Infinity,0,1,1);}catch(e){invalid=e instanceof TypeError;}
      const numeric=hash(x.getImageData('0','0','64','32').data)===first;
      if(gl){gl.pixelStorei(gl.PACK_ALIGNMENT,8);const padded=new Uint8Array(8).fill(77);gl.readPixels(0,0,1,1,gl.RGBA,gl.UNSIGNED_BYTE,padded);gl.pixelStorei(gl.PACK_ALIGNMENT,4);gl.readPixels(0,0,1,1,gl.RGBA,gl.UNSIGNED_BYTE,new Uint8Array(4));if(padded.slice(4).some(v=>v!==77))throw Error('pixel padding changed');}
      return {marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')]),first,again,uri:hash(new TextEncoder().encode(c.toDataURL())),font:metric.width,branded:Object.getOwnPropertyDescriptor(TextMetrics.prototype,'width').get.call(metric)===metric.width,audio:hash(new Uint8Array(audio.getChannelData(0).buffer)),webgl,exported,after:raw(),blank:[...blank],invalid,numeric};
    }"#;
    #[test]
    #[ignore = "Synthetic installed-browser fingerprint first-document/iframe/restart/regeneration verification"]
    fn native_fingerprints_are_stable_across_restart_and_distinct_between_profiles() {
        let fixture = Fixture::new();
        for engine in ["chrome", "edge", "firefox"].into_iter().filter(|name| {
            std::env::var("CLAUDE_DONE_TEST_BROWSER")
                .map(|v| v == *name)
                .unwrap_or(true)
        }) {
            let mut prefs = Preferences::default();
            let mut settings = crate::fingerprint::fresh().unwrap();
            settings.seed = "11".repeat(32);
            prefs.fingerprint = Some(settings.clone());
            let first = browser(engine, prefs.clone(), &fixture.url("/region"));
            let route = if engine == "firefox" {
                "/fingerprint"
            } else {
                "/fingerprint?worker=1"
            };
            first.control.open(fixture.url(route), false).unwrap();
            let report = fixture
                .reports
                .recv_timeout(Duration::from_secs(25))
                .unwrap();
            assert_eq!(
                report["main"]["marker"], true,
                "{engine} preload was not executed"
            );
            if engine != "firefox" {
                assert_eq!(report["worker"]["marker"], true, "{engine} Worker preload");
                assert_eq!(
                    report["worker"]["first"], report["main"]["first"],
                    "{engine} Worker canvas consistency"
                );
                assert_eq!(
                    report["worker"]["font"], report["main"]["font"],
                    "{engine} Worker font consistency"
                );
            }
            assert_eq!(
                report["main"]["first"], report["main"]["again"],
                "{engine} repeat read"
            );
            assert_eq!(
                report["main"], report["frame"],
                "{engine} same-origin iframe preload"
            );
            assert_eq!(
                report["cross"]["marker"], true,
                "{engine} cross-origin iframe preload"
            );
            assert_ne!(
                report["cross"]["first"], report["main"]["first"],
                "{engine} origins have separate noise"
            );
            assert_eq!(
                report["main"]["blank"],
                json!([0, 0, 0, 0]),
                "{engine} transparent pixels stay native"
            );
            assert_eq!(
                report["main"]["invalid"], true,
                "{engine} canvas native range validation"
            );
            assert_eq!(
                report["main"]["numeric"], true,
                "{engine} numeric coordinate coercion"
            );
            assert_eq!(
                report["main"]["after"], report["main"]["first"],
                "{engine} export mutated source"
            );
            assert_eq!(
                report["main"]["branded"], true,
                "{engine} TextMetrics identity"
            );
            first.control.open(fixture.url(route), true).unwrap();
            assert_eq!(
                fixture
                    .reports
                    .recv_timeout(Duration::from_secs(25))
                    .unwrap()["main"],
                report["main"],
                "{engine} new tab"
            );
            let path = first.path.clone();
            first.control.quit_before_navigation().unwrap();
            platform::close_profile(&path).unwrap();
            platform::spawn_controlled_browser(engine, &path, "direct", None).unwrap();
            let (control, _) = start_with_lookup(
                engine.into(),
                path.clone(),
                prefs.clone(),
                regional::initial(&prefs).unwrap(),
                fixture.url("/region"),
                LaunchPurpose::Manual,
            )
            .unwrap();
            control.open(fixture.url(route), false).unwrap();
            assert_eq!(
                fixture
                    .reports
                    .recv_timeout(Duration::from_secs(25))
                    .unwrap()["main"],
                report["main"],
                "{engine} restart"
            );
            control.quit_before_navigation().unwrap();
            platform::close_profile(&path).unwrap();
            drop(control);
            prefs.fingerprint.as_mut().unwrap().seed = "aa".repeat(32);
            let second = browser(engine, prefs, &fixture.url("/region"));
            second.control.open(fixture.url(route), false).unwrap();
            let regenerated = fixture
                .reports
                .recv_timeout(Duration::from_secs(25))
                .unwrap();
            assert_ne!(
                regenerated["main"]["first"], report["main"]["first"],
                "{engine} independent canvas"
            );
            assert_ne!(
                regenerated["main"]["font"], report["main"]["font"],
                "{engine} independent font"
            );
            assert_ne!(
                regenerated["main"]["audio"], report["main"]["audio"],
                "{engine} independent audio"
            );
            println!("{engine}: fingerprint repeat/frame/tab/restart/independence verified");
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            self.stop.store(true, Ordering::SeqCst);
        }
    }
    impl Fixture {
        fn new() -> Self {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let port = listener.local_addr().unwrap().port();
            listener.set_nonblocking(true).unwrap();
            let stop = Arc::new(AtomicBool::new(false));
            let stopping = stop.clone();
            let (tx, reports) = mpsc::channel();
            std::thread::spawn(move || {
                while !stopping.load(Ordering::SeqCst) {
                    let Ok((mut stream, _)) = listener.accept() else {
                        std::thread::sleep(Duration::from_millis(10));
                        continue;
                    };
                    let tx = tx.clone();
                    std::thread::spawn(move || {
                        stream.set_nonblocking(false).unwrap();
                        stream
                            .set_read_timeout(Some(Duration::from_secs(3)))
                            .unwrap();
                        let mut bytes = Vec::new();
                        loop {
                            let mut chunk = [0; 2048];
                            let Ok(n) = stream.read(&mut chunk) else {
                                break;
                            };
                            if n == 0 {
                                break;
                            }
                            bytes.extend_from_slice(&chunk[..n]);
                            if bytes.windows(4).any(|w| w == b"\r\n\r\n") || bytes.len() > 16384 {
                                break;
                            }
                        }
                        // A preconnected socket is not a request. In particular,
                        // never queue a default HTML response after its read
                        // times out: a browser can reuse it for another URL.
                        if bytes.len() > 16384 || !bytes.windows(4).any(|w| w == b"\r\n\r\n") {
                            return;
                        }
                        let request = String::from_utf8_lossy(&bytes);
                        let mut line = request.lines().next().unwrap_or("").split_whitespace();
                        let (Some("GET"), Some(route), Some("HTTP/1.1"), None) =
                            (line.next(), line.next(), line.next(), line.next()) else { return; };

                        let normalized = reqwest::Url::parse(route).ok().map(|url| {
                            format!(
                                "{}{}",
                                url.path(),
                                url.query().map(|q| format!("?{q}")).unwrap_or_default()
                            )
                        });
                        let route = normalized.as_deref().unwrap_or(route);
                        let generic = matches!(route.split('?').next().unwrap_or(route),
                            "/privacy" | "/normal" | "/first" | "/second" | "/new-tab" |
                            "/custom-native" | "/ip-native" | "/launch-ip" | "/live-ip" |
                            "/never-release-on-failed-ip");
                        if !generic && !["/region", "/report?", "/popup-", "/separation", "/fingerprint", "/frame"]
                            .iter().any(|prefix| route.starts_with(prefix)) {
                            let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            return;
                        }
                        if route.starts_with("/popup-redirect") {
                            let response = format!("HTTP/1.1 302 Found\r\nLocation: http://localhost:{port}/popup-final\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                            let _ = stream.write_all(response.as_bytes());
                            return;
                        }
                        let (kind, body) = if route.starts_with("/region") {
                            ("application/json",json!({"success":true,"ip":"203.0.113.12","country_code":"JP","city":"Tokyo","latitude":35.68,"longitude":139.69,"timezone":{"id":"Asia/Tokyo"}}).to_string())
                        } else if route.starts_with("/report?") {
                            let url =
                                reqwest::Url::parse(&format!("http://localhost{route}")).unwrap();
                            let raw = url
                                .query_pairs()
                                .find(|(k, _)| k == "data")
                                .unwrap()
                                .1
                                .into_owned();
                            let mut data: Value = serde_json::from_str(&raw).unwrap();
                            data["header"] = json!(request
                                .lines()
                                .find_map(|l| l
                                    .strip_prefix("Accept-Language: ")
                                    .or_else(|| l.strip_prefix("accept-language: ")))
                                .unwrap_or(""));
                            let _ = tx.send(data);
                            ("text/plain", "ok".into())
                        } else if route.starts_with("/popup-opener") {
                            let action = if route.contains("?write") {
                                r#"const child=window.open('about:blank','_blank');child.document.write('<script>location.replace("/popup-redirect")</'+'script>');child.document.close();"#
                            } else if route.contains("?noopener") { "window.open('/popup-redirect','_blank','noopener')" }
                            else { "window.open('/popup-redirect','_blank')" };
                            ("text/html", format!("<!doctype html><title>Popup opener</title><button id=login onclick=\"{}\">Sign in</button><script>fetch('/report?data='+encodeURIComponent(JSON.stringify({{stage:'opener'}})))</script>", action.replace('"', "&quot;")))
                        } else if route.starts_with("/popup-final") {
                            ("text/html", r#"<!doctype html><title>Login popup fixture</title><script>
                            const first={stage:'popup-first',timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')]),webdriver:navigator.webdriver};
                            fetch('/report?data='+encodeURIComponent(JSON.stringify(first)));
                            setTimeout(()=>fetch('/report?data='+encodeURIComponent(JSON.stringify({stage:'popup-alive',timezone:Intl.DateTimeFormat().resolvedOptions().timeZone}))),8000);
                            </script>"#.into())
                        } else if route.starts_with("/separation-workers") {
                            ("text/html", r#"<!doctype html><title>Concurrent worker setup</title><script>
                            const heldWorkers=[];const jobs=Array.from({length:8},()=>new Promise(resolve=>{
                              const w=new Worker('/separation-worker');heldWorkers.push(w);w.onmessage=e=>resolve(e.data);
                              w.onerror=e=>resolve({error:e.message});setTimeout(()=>resolve({error:'worker timeout'}),25000);
                            }));
                            for(let i=0;i<8;i++)jobs.push(new Promise(resolve=>{
                              const w=new SharedWorker('/separation-shared-worker?i='+i);heldWorkers.push(w);w.port.onmessage=e=>resolve(e.data);
                              w.onerror=e=>resolve({error:e.message});setTimeout(()=>resolve({error:'shared worker timeout'}),25000);
                            }));
                            Promise.all(jobs).then(workers=>fetch('/report?data='+encodeURIComponent(JSON.stringify({workers}))));
                            </script>"#.into())
                        } else if route.starts_with("/separation-shared-worker") {
                            ("application/javascript", "const first={timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')])};onconnect=e=>e.ports[0].postMessage(first);".into())
                        } else if route.starts_with("/separation-worker") {
                            ("application/javascript", "postMessage({timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')])});".into())
                        } else if route.starts_with("/separation-frame") {
                            ("text/html", "<script>parent.postMessage({frame:{timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')])}},'*')</script>".into())
                        } else if route.starts_with("/separation") {
                            ("text/html", r#"<!doctype html><title>Manual settings test</title><script>
                            const main={webdriver:navigator.webdriver,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')]),cookie:document.cookie,route:location.pathname+location.search,documentId:crypto.randomUUID()};
                            if(new URLSearchParams(location.search).has('locale-field'))main.locale=Intl.DateTimeFormat().resolvedOptions().locale;
                            if(new URLSearchParams(location.search).has('store')){document.cookie='nodecloak-fixture=kept; Max-Age=3600; SameSite=Lax';main.cookieAfterStore=document.cookie;}
                            const worker=new Promise(resolve=>{const w=new Worker('/separation-worker');w.onmessage=e=>{w.terminate();resolve(e.data)};w.onerror=e=>resolve({error:e.message});setTimeout(()=>resolve({error:'worker timeout'}),10000)});
                            const frame=new Promise(resolve=>{addEventListener('message',e=>{if(e.data.frame)resolve(e.data.frame)});const f=document.createElement('iframe');f.src=location.origin.replace('127.0.0.1','localhost')+'/separation-frame';document.documentElement.append(f);setTimeout(()=>resolve({error:'frame timeout'}),10000)});
                            const resumeFixture=new URLSearchParams(location.search).has('restore-session');
                            if(resumeFixture){const draft=document.createElement('textarea');draft.id='saved-draft';document.documentElement.append(draft);const spacer=document.createElement('div');spacer.style.height='5000px';document.documentElement.append(spacer);if(location.hash!=='#saved'){document.cookie='nodecloak-session=restored; SameSite=Lax';draft.value='saved draft';draft.dispatchEvent(new Event('input',{bubbles:true}));scrollTo(0,700);history.replaceState({},'','#saved');}}
                            Promise.all([worker,frame]).then(([worker,frame])=>setTimeout(()=>{if(resumeFixture){main.draft=document.getElementById('saved-draft').value;main.scroll=scrollY;}fetch('/report?data='+encodeURIComponent(JSON.stringify({main,worker,frame})));},resumeFixture?1200:0));
                            </script>"#.into())
                        } else if route.starts_with("/fingerprint-worker") {
                            ("application/javascript",r#"const c=new OffscreenCanvas(64,32);const x=c.getContext('2d');x.fillStyle='#735941';x.fillRect(0,0,64,32);x.font='14px serif';x.fillText('NodeCloak',2,19);const bytes=x.getImageData(0,0,64,32).data;let h=2166136261;for(const v of bytes)h=Math.imul(h^v,16777619);postMessage({marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')]),first:h>>>0,font:x.measureText('NodeCloak').width});self.close();"#.into())
                        } else if route.starts_with("/fingerprint-frame") {
                            ("text/html",format!("<script>{FINGERPRINT_PROBE};fingerprintProbe().then(fingerprint=>parent.postMessage({{fingerprint}},'*'));</script>"))
                        } else if route.starts_with("/fingerprint") {
                            ("text/html",format!("<!doctype html><title>Fingerprint fixture</title><script>{FINGERPRINT_PROBE};const frameAt=url=>new Promise(resolve=>{{const f=document.createElement('iframe');addEventListener('message',e=>{{if(e.source===f.contentWindow&&e.data.fingerprint)resolve(e.data.fingerprint)}});f.src=url;document.documentElement.append(f)}});const frame=frameAt('/fingerprint-frame');const cross=frameAt(location.origin.replace('127.0.0.1','localhost')+'/fingerprint-frame');const worker=location.search.includes('worker')?new Promise(resolve=>{{const w=new Worker('/fingerprint-worker');w.onmessage=e=>resolve(e.data);w.onerror=e=>resolve({{error:e.message}})}}):Promise.resolve(null);Promise.all([fingerprintProbe(),frame,cross,worker]).then(([main,frame,cross,worker])=>fetch('/report?data='+encodeURIComponent(JSON.stringify({{main,frame,cross,worker}}))));</script>"))
                        } else if route.starts_with("/frame") {
                            ("text/html",r#"<script>const frame={hardware:navigator.hardwareConcurrency,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,jan:new Date('2026-01-15T12:00:00Z').getTimezoneOffset(),jul:new Date('2026-07-15T12:00:00Z').getTimezoneOffset(),visibility:document.visibilityState};const permission=navigator.permissions.query({name:'geolocation'}).then(p=>frame.permission=p.state);let done=false;const send=async geo=>{if(done)return;done=true;frame.geo=geo;frame.visibilityNow=document.visibilityState;await permission;parent.postMessage({frame},'*')};navigator.geolocation.getCurrentPosition(p=>send({latitude:p.coords.latitude,longitude:p.coords.longitude}),e=>send({error:e.code}),{timeout:8000});setTimeout(()=>send({error:'timeout'}),9000);</script>"#.into())
                        } else {
                            // The frame's geolocation fallback takes nine seconds.
                            // Its outer deadline must also allow process/document
                            // startup, while remaining below the 25s report limit.
                            ("text/html",r#"<!doctype html><title>NodeCloak region verification</title><p>Verifying isolated browser settings</p><script>
                    const snapshot=()=>({hardware:navigator.hardwareConcurrency,webdriver:navigator.webdriver,marker:Boolean(globalThis[Symbol.for('nodecloak.readback.v1')]),language:navigator.language,languages:[...navigator.languages],locale:Intl.DateTimeFormat().resolvedOptions().locale,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,jan:new Date('2026-01-15T12:00:00Z').getTimezoneOffset(),jul:new Date('2026-07-15T12:00:00Z').getTimezoneOffset()});
                    const first=snapshot();first.tag=location.pathname;first.visibility=document.visibilityState;first.focus=document.hasFocus();first.dnt=navigator.doNotTrack;first.width=innerWidth;first.height=innerHeight;first.webgl=!!document.createElement('canvas').getContext('webgl');window.progress={};
                    const privacy=(async()=>{first.webgpu=!!(navigator.gpu&&await navigator.gpu.requestAdapter());if(window.speechSynthesis){await new Promise(r=>setTimeout(r,300));first.voices=speechSynthesis.getVoices().length;}else first.voices=0;})();
                    const worker=new Promise(resolve=>{const w=new Worker(URL.createObjectURL(new Blob(['postMessage(('+snapshot.toString()+')())'],{type:'application/javascript'})));w.onmessage=e=>{w.terminate();window.progress.worker=e.data;resolve(e.data)};w.onerror=e=>resolve({error:e.message});setTimeout(()=>resolve({error:'timeout'}),10000)});
                    const frame=new Promise(resolve=>{addEventListener('message',e=>{if(e.data.frame){window.progress.frame=e.data.frame;resolve(e.data.frame)}});const f=document.createElement('iframe');f.allow='geolocation';f.src=location.origin.replace('127.0.0.1','localhost')+'/frame';document.documentElement.append(f);setTimeout(()=>resolve({error:'timeout'}),18000)});
                    const geo=new Promise(resolve=>{setTimeout(()=>resolve({error:'position timeout'}),6000);navigator.geolocation.getCurrentPosition(p=>{window.progress.geo='success';resolve({latitude:p.coords.latitude,longitude:p.coords.longitude,accuracy:p.coords.accuracy})},e=>{window.progress.geo='error '+e.code;resolve({error:e.code})},{timeout:5000});});const permission=navigator.permissions.query({name:'geolocation'}).then(p=>{window.progress.permission=p.state;return p});setTimeout(()=>window.progress.timer='fired',6000);
                    Promise.all([worker,frame,geo,permission,privacy]).then(([worker,frame,geo,permission])=>fetch('/report?data='+encodeURIComponent(JSON.stringify({...first,worker,frame,geo,permission:permission.state})))).catch(e=>fetch('/report?data='+encodeURIComponent(JSON.stringify({error:String(e)}))));
                    </script>"#.into())
                        };
                        let coop = if route.starts_with("/popup-final") { "Cross-Origin-Opener-Policy: same-origin\r\n" } else { "" };
                        let response=format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nAccess-Control-Allow-Origin: *\r\nAccess-Control-Allow-Methods: GET, OPTIONS\r\nCache-Control: no-store\r\n{coop}\r\n{body}",body.len());
                        let _ = stream.write_all(response.as_bytes());
                    });
                }
            });
            Self {
                port,
                reports,
                stop,
            }
        }
        fn url(&self, route: &str) -> String {
            format!("http://127.0.0.1:{}{route}", self.port)
        }
        fn report(&self, browser: &TempBrowser) -> Value {
            // Chromium suspends geolocation while its native window is occluded.
            // Test actual visible-window behavior rather than overriding visibility.
            platform::focus_profile(&browser.path).unwrap();
            #[cfg(windows)]
            {
                let ids = platform::profile_process_ids(&browser.path).unwrap();
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                        Some(show_test_window),
                        &ids as *const _ as isize,
                    );
                }
            }
            match self.reports.recv_timeout(Duration::from_secs(20)) {
                Ok(data) => data,
                Err(e) => {
                    let (tx, rx) = mpsc::channel();
                    let _ = browser.control.sender.send(Request::Probe(tx));
                    panic!(
                        "browser did not report {e:?}; probe {:?}; controller {:?}",
                        rx.recv_timeout(Duration::from_secs(5)),
                        browser.control.error.lock().unwrap()
                    );
                }
            }
        }
    }
    include!("browser_control/download_checks.rs");
    include!("browser_control/recovery_checks.rs");
    include!("browser_control/worker_checks.rs");

    struct TempBrowser {
        path: PathBuf,
        control: Control,
        _root: tempfile::TempDir,
    }
    #[test]
    #[ignore="Starts isolated installed Chrome/Edge/Firefox for MCP page operations"]
    fn native_mcp_page_operations_all_browsers(){
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();let address=listener.local_addr().unwrap();
        let done=Arc::new(AtomicBool::new(false));let stop=done.clone();listener.set_nonblocking(true).unwrap();
        let server=std::thread::spawn(move||{while !stop.load(Ordering::SeqCst){if let Ok((mut stream,_))=listener.accept(){let _=stream.set_read_timeout(Some(Duration::from_secs(2)));let mut request=[0u8;4096];let n=stream.read(&mut request).unwrap_or(0);let req=String::from_utf8_lossy(&request[..n]);let body=if req.starts_with("GET /frame"){r#"<html><body>Frame ready<button onclick="this.textContent='Frame clicked'">Frame button</button></body></html>"#.to_string()}else{format!(r#"<html><head><title>MCP Fixture</title></head><body><p id="result">Ready</p><input aria-label="Name" oninput="document.getElementById('nameValue').textContent='Name:'+this.value"><span id="nameValue"></span><select aria-label="Color" onchange="document.getElementById('colorValue').textContent='Color:'+this.value"><option value="a">A</option><option value="b">B</option></select><span id="colorValue"></span><input type="file" aria-label="Upload" onchange="document.getElementById('uploadValue').textContent=this.files[0]?.name"><span id="uploadValue"></span><button onclick="document.getElementById('result').textContent='Clicked'">Save</button><button onclick="alert('Hello')">Dialog</button><a href="/file" download="fixture.txt">Download</a><iframe src="/frame?local"></iframe><iframe src="http://localhost:{}/frame"></iframe></body></html>"#,address.port())};let data=if req.starts_with("GET /file"){"HTTP/1.1 200 OK\r\nContent-Disposition: attachment; filename=fixture.txt\r\nContent-Length: 8\r\nConnection: close\r\n\r\nfixture!".to_string()}else{format!("HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",body.len(),body)};let _=stream.write_all(data.as_bytes());}else{std::thread::sleep(Duration::from_millis(15));}}});
        for (engine, stable) in ["chrome","edge","firefox"].into_iter().filter(|e|std::env::var("CLAUDE_DONE_TEST_BROWSER").map(|s|s==*e).unwrap_or(true)).flat_map(|engine| [(engine, false), (engine, true)]){
            println!("{engine}: MCP operations with stable fingerprint={stable}");
            let mut prefs=Preferences::default();prefs.startup_url="about:blank".into();
            if stable { prefs.fingerprint=Some(crate::fingerprint::fresh().unwrap()); }
            let browser=browser(engine,prefs,"");let files=tempfile::tempdir().unwrap();let file=files.path().join("input.txt");std::fs::write(&file,"fixture upload").unwrap();
            let grant=crate::mcp::policy::Grant{id:uuid::Uuid::new_v4().to_string(),name:"Fixture".into(),profile_ids:vec!["test".into()],permissions:vec!["read".into(),"manage".into(),"page_read".into(),"page_write".into(),"files".into()],origins:vec![],expires_at:chrono::Utc::now().timestamp()+600,confirm_interactions:false,file_roots:vec![files.path().to_string_lossy().into()],revoked:false,token_hash:String::new()};
            let call=|tool:&str,mut args:Value|{args["profile_id"]=json!("test");let(tx,rx)=mpsc::channel();browser.control.sender.send(Request::Automation(tool.into(),args,grant.clone(),Arc::new(AtomicBool::new(false)),tx)).unwrap();rx.recv_timeout(Duration::from_secs(45)).unwrap_or_else(|e|panic!("{engine}: {tool}: {e}; {:?}",browser.control.error.lock().unwrap()))};
            let open=call("tabs_open",json!({"url":format!("http://{address}/")})).unwrap_or_else(|e|panic!("{engine}: open: {e}"));let tab=open["tab_id"].as_str().unwrap();let base=json!({"tab_id":tab});
            call("page_wait",json!({"tab_id":tab,"text":"Ready","timeout_ms":15000})).unwrap_or_else(|e|panic!("{engine}: wait: {e}"));
            let snapshot=call("page_snapshot",base.clone()).unwrap();assert_eq!(snapshot["title"],"MCP Fixture","{engine}");
            let element=|label:&str|{let ref_id=snapshot["elements"].as_array().unwrap().iter().find(|e|e["name"]==label).unwrap()["ref"].clone();json!({"tab_id":tab,"snapshot_id":snapshot["snapshot_id"],"element_ref":ref_id})};
            let mut fill=element("Name");fill["value"]=json!("NodeCloak");call("page_fill",fill).unwrap_or_else(|e|panic!("{engine}: fill: {e}"));
            call("page_wait",json!({"tab_id":tab,"text":"Name:NodeCloak"})).unwrap();
            let mut select=element("Color");select["values"]=json!(["b"]);call("page_select",select).unwrap();call("page_wait",json!({"tab_id":tab,"text":"Color:b"})).unwrap();
            let preview=call("__preview",element("Save")).unwrap();assert_eq!(preview["name"],"Save");
            let mut changed=element("Save");changed["_expected_target"]=json!({"url":"https://changed.test/","epoch":0});assert!(call("page_click",changed).unwrap_err().contains("PAGE_CHANGED"));
            let mut renamed=element("Save");let mut wrong=preview.clone();wrong["name"]=json!("Other action");renamed["_expected_target"]=wrong;assert!(call("page_click",renamed).unwrap_err().contains("TARGET_CHANGED"));
            let mut approved=element("Save");approved["_expected_target"]=preview;call("page_click",approved).unwrap();
            let mut upload=element("Upload");upload["paths"]=json!([file]);call("page_upload",upload).unwrap_or_else(|e|panic!("{engine}: upload: {e}"));
            call("page_wait",json!({"tab_id":tab,"text":"input.txt"})).unwrap();
            call("page_click",element("Dialog")).unwrap_or_else(|e|panic!("{engine}: dialog click: {e}"));call("page_dialog",json!({"tab_id":tab,"accept":true})).unwrap_or_else(|e|panic!("{engine}: dialog handle: {e}"));
            call("page_click",element("Download")).unwrap();let limit=Instant::now()+Duration::from_secs(10);loop{let files=call("downloads_list",json!({})).unwrap();if files["files"].as_array().unwrap().iter().any(|f|f["name"]=="fixture.txt"){break;}assert!(Instant::now()<limit,"{engine}: download missing: {files}");std::thread::sleep(Duration::from_millis(200));}
            call("page_click",element("Save")).unwrap();let screen=call("page_screenshot",base.clone()).unwrap();assert!(screen["image_base64"].as_str().unwrap().len()>100,"{engine}");
            call("page_wait",base.clone()).unwrap();let frames=call("frames_list",base.clone()).unwrap();println!("{engine}: frames: {frames}");let children:Vec<_>=frames["items"].as_array().unwrap().iter().filter(|f|f["top_level"]==false).collect();assert!(children.len()>=2,"{engine}: missing frames: {frames}");for child_frame in children{let frame=child_frame["frame_id"].clone();let child=call("page_snapshot",json!({"tab_id":tab,"frame_id":frame})).unwrap_or_else(|e|panic!("{engine}: frame: {e}"));assert!(child["text"].as_str().unwrap().contains("Frame ready"));let button=child["elements"][0]["ref"].clone();call("page_click",json!({"tab_id":tab,"frame_id":frame,"snapshot_id":child["snapshot_id"],"element_ref":button})).unwrap_or_else(|e|panic!("{engine}: frame click: {e}"));call("page_wait",json!({"tab_id":tab,"frame_id":frame,"text":"Frame clicked"})).unwrap_or_else(|e|panic!("{engine}: frame click result: {e}"));call("page_navigate",json!({"tab_id":tab,"frame_id":frame,"url":format!("{}?changed",child_frame["url"].as_str().unwrap())})).unwrap();call("page_wait",json!({"tab_id":tab,"frame_id":frame,"text":"Frame ready"})).unwrap();}
            let root_after=call("page_snapshot",base.clone()).unwrap();assert_eq!(root_after["url"],format!("http://{address}/"),"{engine}: child navigation changed root");
            let next=call("page_snapshot",base.clone()).unwrap();assert!(call("page_click",element("Save")).is_err(),"{engine}: stale ref accepted");assert_ne!(snapshot["snapshot_id"],next["snapshot_id"]);
            call("page_navigate",json!({"tab_id":tab,"url":format!("http://{address}/?again")})).unwrap();call("page_wait",json!({"tab_id":tab,"text":"Ready"})).unwrap();assert!(call("page_fill",element("Name")).is_err());
            call("tabs_close",base).unwrap();assert!(browser.control.alive.load(Ordering::SeqCst));println!("{engine}: MCP page/inputs/upload/screenshot/cross-origin frame/stale-ref passed");
        }
        done.store(true,Ordering::SeqCst);server.join().unwrap();
    }
    impl TempBrowser {
        fn show(&self) {
            if std::env::var_os("NODECLOAK_TEST_HEADLESS_BROWSERS").is_some() { return; }
            platform::focus_profile(&self.path).unwrap();
            #[cfg(windows)]
            {
                let ids = platform::profile_process_ids(&self.path).unwrap();
                unsafe {
                    windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                        Some(show_test_window),
                        &ids as *const _ as isize,
                    );
                }
            }
            std::thread::sleep(Duration::from_millis(250));
        }
    }
    impl Drop for TempBrowser {
        fn drop(&mut self) {
            let _ = self.control.quit_before_navigation();
            self.control.stop.store(true, Ordering::SeqCst);
            let _ = platform::close_profile(&self.path);
            let _ = platform::force_close_profile(&self.path);
        }
    }
    fn browser(browser: &str, prefs: Preferences, lookup: &str) -> TempBrowser {
        browser_with_setup(browser, prefs, lookup, |_| {})
    }
    fn browser_with_setup(browser: &str, prefs: Preferences, lookup: &str, setup: impl FnOnce(&Path)) -> TempBrowser {
        browser_for_purpose(browser, prefs, lookup, LaunchPurpose::Mcp, setup)
    }
    fn browser_for_purpose(browser: &str, mut prefs: Preferences, lookup: &str, purpose: LaunchPurpose, setup: impl FnOnce(&Path)) -> TempBrowser {
        prefs.regional.enabled = true;
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let draft = Draft {
            name: "Region verification".into(),
            browser: browser.into(),
            notes: String::new(),
            tags: vec![],
            proxy: ProxyConfig {
                mode: "direct".into(),
                ..ProxyConfig::default()
            },
            password: None,
            clear_password: false,
            preferences: prefs.clone(),
        };
        let p = profiles::create(&canonical, draft, None).unwrap();
        profiles::apply_preferences_for_control(&canonical,&p,purpose == LaunchPurpose::Mcp).unwrap();
        let path = profiles::path(&canonical, &p).unwrap();
        let ip=regional::uses_ip(&prefs).then(||regional::parse_ip_region(&json!({"success":true,"ip":"203.0.113.12","country_code":"JP","city":"Tokyo","latitude":35.68,"longitude":139.69,"timezone":{"id":"Asia/Tokyo"}})).unwrap());
        if browser == "firefox" {
            crate::firefox::prepare_geolocation(
                &path,
                Some(&regional::resolve(&prefs, ip.clone()).unwrap()),
            )
            .unwrap();
        }
        setup(&path);
        if purpose == LaunchPurpose::Manual {
            platform::spawn_settings_browser(browser, &path, "direct", None).unwrap();
        } else {
            platform::spawn_controlled_browser(browser, &path, "direct", None).unwrap();
        }
        let result = start_with_lookup(
            browser.into(),
            path.clone(),
            prefs.clone(),
            regional::resolve(&prefs, ip).unwrap(),
            lookup.into(),
            purpose,
        );
        if result.is_err() {
            let _ = platform::force_close_profile(&path);
        }
        let (control, actual) = result.unwrap();
        println!("{browser} resolved: {actual:?}");
        let browser = TempBrowser {
            path,
            control,
            _root: root,
        };
        browser.show();
        browser
    }
    #[test]
    #[ignore = "Installs the signed companion in a disposable Firefox, then verifies real app launch and toggling"]
    fn native_firefox_signed_fingerprint_launch_and_disable() {
        assert!(std::env::var_os("NODECLOAK_TEST_FIREFOX_NATIVE_HOST").is_some(), "Set a standalone app native-host executable for this opt-in check");
        struct Guard { path: PathBuf, root: tempfile::TempDir }
        impl Drop for Guard {
            fn drop(&mut self) {
                let _ = platform::close_profile(&self.path);
                let _ = platform::force_close_profile(&self.path);
                #[cfg(windows)]
                {
                    use winreg::{RegKey, enums::*};
                    let key_path = r"Software\Mozilla\NativeMessagingHosts\com.nodecloak.fingerprint";
                    let owner = self.root.path().canonicalize().unwrap_or_else(|_| self.root.path().to_path_buf()).join("firefox-privacy/com.nodecloak.fingerprint.json");
                    let hkcu = RegKey::predef(HKEY_CURRENT_USER);
                    if let Ok(key) = hkcu.open_subkey_with_flags(key_path, KEY_READ | KEY_WOW64_64KEY) {
                        let value: Result<String, _> = key.get_value("");
                        if value.is_ok_and(|v| Path::new(&v) == owner) {
                            drop(key);
                            let _ = hkcu.delete_subkey_with_flags(key_path, KEY_WOW64_64KEY);
                        }
                    }
                }
            }
        }
        let fixture = Fixture::new();
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let prefs = Preferences { startup_url: fixture.url("/separation?store"), fingerprint: Some(crate::fingerprint::fresh().unwrap()), ..Preferences::default() };
        let mut draft = Draft { name: "Signed privacy regression".into(), browser: "firefox".into(), notes: String::new(), tags: vec![], password: None, clear_password: false, proxy: ProxyConfig { mode: "direct".into(), ..ProxyConfig::default() }, preferences: prefs };
        let p = profiles::create(&canonical, draft.clone(), None).unwrap();
        let path = profiles::path(&canonical, &p).unwrap();
        #[cfg(target_os = "macos")]
        if std::env::var_os("NODECLOAK_TEST_HEADLESS_BROWSERS").is_some() {
            // Mozilla's headless geolocation tests disable the unavailable OS
            // settings dialog. This belongs only to this disposable fixture;
            // the product never changes the user's system permission policy.
            let mut file = std::fs::OpenOptions::new().create(true).append(true)
                .open(path.join("user.js")).unwrap();
            writeln!(file, "\nuser_pref(\"geo.prompt.open_system_prefs\", false);").unwrap();
        }
        let _guard = Guard { path: path.clone(), root };
        let runtime = profiles::Runtime::default();
        let original_seed = p.preferences.fingerprint.as_ref().unwrap().seed.clone();
        let switches = if std::env::var_os("NODECLOAK_TEST_REGIONS_ONLY").is_some() { vec![true] } else { vec![true,true,false,true] };
        for (run, enabled) in switches.into_iter().enumerate() {
            let route = format!("/separation?{}&run={}", if run==0 {"store"} else {"read"}, uuid::Uuid::new_v4());
            draft.preferences.startup_url = fixture.url(&route);
            draft.preferences.fingerprint.as_mut().unwrap().enabled = enabled;
            profiles::update(&canonical, &p.id, draft.clone()).unwrap();
            crate::profile_commands::launch(&canonical, &p.id, "__fixture_startup", &runtime).unwrap();
            let report = fixture.reports.recv_timeout(Duration::from_secs(25)).unwrap();
            assert_eq!(report["main"]["route"],route,"stale document report: {report}");
            if run==0 { assert_eq!(report["main"]["cookieAfterStore"],"nodecloak-fixture=kept","synthetic cookie write failed: {report}"); }
            assert_eq!(report["main"]["webdriver"], false, "manual Firefox is automated: {report}");
            assert_eq!(report["main"]["marker"], enabled, "companion did not follow the saved switch: {report}");
            assert_eq!(report["frame"]["marker"], enabled, "frame protection did not follow the saved switch: {report}");
            if run > 0 { assert_eq!(report["main"]["cookie"], "nodecloak-fixture=kept", "installation/toggling lost persisted site data"); }
            assert!(!runtime.1.ready(&p.id));
            assert!(!runtime.1.automation_ready(&p.id));
            assert_eq!(profiles::get(&canonical, &p.id, false).unwrap().preferences.fingerprint.as_ref().unwrap().seed, original_seed);
            println!("Firefox signed app launch enabled={enabled}: first document/frame protected as requested; native webdriver=false");
            #[cfg(target_os="macos")]
            if std::env::var_os("NODECLOAK_TEST_LEGACY_TERM").is_some() {
                for pid in platform::profile_process_ids(&path).unwrap() {
                    let status=std::process::Command::new("/bin/kill").args(["-TERM",&pid.to_string()]).status().unwrap();
                    assert!(status.success());
                }
                platform::wait_profile_closed(&path,Duration::from_secs(8)).unwrap();
            } else { platform::close_profile(&path).unwrap(); }
            #[cfg(not(target_os="macos"))]
            platform::close_profile(&path).unwrap();
            let db=rusqlite::Connection::open_with_flags(path.join("cookies.sqlite"),rusqlite::OpenFlags::SQLITE_OPEN_READ_ONLY).unwrap();
            let cookies=db.prepare("SELECT host,path,value,expiry,originAttributes FROM moz_cookies WHERE name='nodecloak-fixture'").unwrap().query_map([],|row|Ok((row.get::<_,String>(0)?,row.get::<_,String>(1)?,row.get::<_,String>(2)?,row.get::<_,i64>(3)?,row.get::<_,String>(4)?))).unwrap().collect::<Result<Vec<_>,_>>().unwrap();
            println!("Firefox synthetic persisted cookies after close run={run}: {cookies:?}");
            assert!(cookies.iter().any(|(host,_,value,expiry,_)|host=="127.0.0.1"&&value=="kept"&&*expiry>0),"normal close did not persist synthetic cookie: {cookies:?}");
        }
        draft.preferences.regional.enabled = true;
        draft.preferences.language = "de-DE,de,en".into();
        draft.preferences.regional.timezone_mode = "custom".into();
        draft.preferences.regional.timezone = "America/Los_Angeles".into();
        draft.preferences.regional.location_mode = "custom".into();
        draft.preferences.regional.latitude = 1.3521;
        draft.preferences.regional.longitude = 103.8198;
        draft.preferences.advanced.location = PermissionMode::Allow;
        draft.preferences.advanced.hardware_concurrency = 4;
        draft.preferences.advanced.viewport_width = 1000;
        draft.preferences.advanced.viewport_height = 700;
        for ip in [false, true] {
            if ip {
                draft.preferences.regional.language_mode = "ip".into();
                draft.preferences.regional.timezone_mode = "ip".into();
                draft.preferences.regional.location_mode = "ip".into();
            }
            draft.preferences.startup_url = fixture.url(if ip {"/ip-native"} else {"/custom-native"});
            profiles::update(&canonical, &p.id, draft.clone()).unwrap();
            crate::profile_commands::launch_with_lookup(&canonical, &p.id, "__fixture_startup", &runtime, &fixture.url("/region")).unwrap();
            let _ = platform::focus_profile(&path);
            let report = fixture.reports.recv_timeout(Duration::from_secs(25)).unwrap();
            println!("Firefox synthetic native region ip={ip}: {report}");
            assert_eq!(report["webdriver"], false, "regional settings started WebDriver: {report}");
            assert_eq!(report["marker"], true, "regional settings lost fingerprint protection: {report}");
            assert_eq!(report["width"], 1000, "native viewport width: {report}");
            assert_eq!(report["height"], 700, "native viewport height: {report}");
            let (language,zone,jan,jul,lat,lng) = if ip {("ja-JP","Asia/Tokyo",-540,-540,35.68,139.69)} else {("de-DE","America/Los_Angeles",480,420,1.3521,103.8198)};
            assert_eq!(report["language"],language);
            assert_eq!(report["locale"],language);
            assert!(report["header"].as_str().unwrap().starts_with(language));
            for context in [&report,&report["worker"],&report["frame"]] {
                assert_eq!(context["timezone"],zone);assert_eq!(context["jan"],jan);assert_eq!(context["jul"],jul);
            }
            if cfg!(target_os = "macos")
                && std::env::var_os("NODECLOAK_TEST_HEADLESS_BROWSERS").is_some() {
                // The system Permissions API can remain prompt. Actual native
                // coordinate delivery must succeed in both documents with the
                // fixture-only OS-dialog setting above; errors are not waived.
                for (context, frame) in [(&report,false),(&report["frame"],true)] {
                    let geo = &context["geo"];
                    assert!(geo.get("error").is_none(),"native synthetic position failed (frame={frame}): {context}");
                    assert!(matches!(context["permission"].as_str(),Some("prompt" | "granted" | "denied")));
                    assert!((geo["latitude"].as_f64().unwrap()-lat).abs() < 0.000001);
                    assert!((geo["longitude"].as_f64().unwrap()-lng).abs() < 0.000001);
                    println!("macOS native synthetic coordinates verified (frame={frame}); permission query={}",context["permission"]);
                }
            } else if report["geo"].get("error").is_some() {
                assert_eq!(report["visibility"], "hidden", "visible location failed: {report}");
                assert_eq!(report["geo"]["error"], 3, "unexpected location failure: {report}");
                assert_eq!(report["permission"], "granted");
            } else { check(&report,language,zone,jan,jul,lat,lng); }
            assert!(!runtime.1.ready(&p.id));
            platform::close_profile(&path).unwrap();
            use base64::Engine;
            let geo_snapshot = crate::firefox::snapshot(&path,"geo.provider.network.url").unwrap();
            let uri = geo_snapshot["runtime"].as_str().unwrap();
            let geo_bytes = base64::engine::general_purpose::STANDARD.decode(uri.split_once("base64,").unwrap().1).unwrap();
            let native_provider:Value = serde_json::from_slice(&geo_bytes).unwrap();
            assert_eq!(native_provider["location"]["lat"],lat);
            assert_eq!(native_provider["location"]["lng"],lng);
            for (key, expected) in [("geo.enabled",json!(true)),("permissions.default.geo",json!(1))] {
                let values = crate::firefox::snapshot(&path,key).unwrap();
                assert_eq!(values["user"],expected,"native permission configuration: {values}");
                assert!(values["runtime"] == expected || (key == "geo.enabled" && values["runtime"].is_null()),
                    "runtime permission configuration: {values}");
            }
            for (key, expected) in [("devtools.chrome.enabled",Value::Null),("devtools.debugger.remote-enabled",Value::Null)] {
                let values = crate::firefox::snapshot(&path,key).unwrap();
                assert_eq!(values["user"], expected, "startup debugger preference leaked");
                assert!(values["runtime"].is_null() || values["runtime"] == false, "runtime debugger preference leaked: {values}");
            }
            println!("Firefox native region ip={ip}: first document, worker/frame, native location configuration, viewport and fingerprint pass without WebDriver");
        }
        draft.preferences.startup_url = fixture.url("/never-release-on-failed-ip");
        profiles::update(&canonical,&p.id,draft.clone()).unwrap();
        let error = crate::profile_commands::launch_with_lookup(&canonical,&p.id,"startup",&runtime,&fixture.url("/bad-ip-provider")).unwrap_err();
        assert!(error.contains("区域"), "unexpected lookup failure: {error}");
        assert!(fixture.reports.recv_timeout(Duration::from_millis(500)).is_err(), "IP failure released target document");
        platform::ensure_profile_closed(&path).unwrap();
        draft.preferences.regional = crate::regional::RegionalSettings::default();
        draft.preferences.advanced.viewport_width = 0;
        draft.preferences.advanced.viewport_height = 0;
        draft.preferences.startup_url = fixture.url("/separation?read");
        profiles::update(&canonical,&p.id,draft.clone()).unwrap();
        crate::profile_commands::launch(&canonical,&p.id,"__fixture_startup",&runtime).unwrap();
        let reset = fixture.reports.recv_timeout(Duration::from_secs(25)).unwrap();
        assert_eq!(reset["main"]["webdriver"],false);
        assert_eq!(reset["main"]["cookie"],"nodecloak-fixture=kept");
        assert!(!path.join("nodecloak-geolocation-original.json").exists());
        assert!(!path.join("nodecloak-firefox-region-debug-original.json").exists());
        platform::close_profile(&path).unwrap();
        // Built-in local diagnostics must not be redirected by an HTTP content
        // principal: Firefox rejects HTTP -> file/about navigation.
        let local_page = canonical.join("local-destination.html");
        std::fs::write(&local_page, format!("<!doctype html><script>fetch({}+encodeURIComponent(JSON.stringify({{stage:'local-document',webdriver:navigator.webdriver}})))</script>", json!(fixture.url("/report?data=")))).unwrap();
        crate::firefox_extension::launch(&path, reqwest::Url::from_file_path(&local_page).unwrap().as_str(), "direct", None).unwrap();
        let report = fixture.reports.recv_timeout(Duration::from_secs(15)).expect("local destination was not opened");
        assert_eq!(report["stage"], "local-document");
        assert_eq!(report["webdriver"], false);
    }
    #[test]
    #[ignore = "Starts a disposable Firefox through the app launch path with an idle fingerprint seed"]
    fn native_firefox_idle_fingerprint_uses_normal_app_launch() {
        struct ProfileGuard { path: PathBuf, _root: tempfile::TempDir }
        impl Drop for ProfileGuard {
            fn drop(&mut self) {
                let _ = platform::close_profile(&self.path);
                let _ = platform::force_close_profile(&self.path);
            }
        }
        let fixture = Fixture::new();
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let seed = "37".repeat(32);
        let prefs = Preferences {
            startup_url: fixture.url("/separation"),
            fingerprint: Some(crate::fingerprint::Settings {
                version: 1, seed: seed.clone(), enabled: true,
                canvas_noise: false, font_noise: false, webgl_noise: false,
                audio_noise: false, media_noise: false,
            }),
            ..Preferences::default()
        };
        let draft = Draft {
            name: "Idle fingerprint regression".into(), browser: "firefox".into(),
            notes: String::new(), tags: vec![], password: None, clear_password: false,
            proxy: ProxyConfig { mode: "direct".into(), ..ProxyConfig::default() },
            preferences: prefs,
        };
        let p = profiles::create(&canonical, draft, None).unwrap();
        let path = profiles::path(&canonical, &p).unwrap();
        let _guard = ProfileGuard { path: path.clone(), _root: root };
        let runtime = profiles::Runtime::default();
        crate::profile_commands::launch(&canonical, &p.id, "startup", &runtime).unwrap();
        let report = fixture.reports.recv_timeout(Duration::from_secs(25)).unwrap();
        assert_eq!(report["main"]["webdriver"], false, "idle settings started WebDriver: {report}");
        assert_eq!(report["main"]["marker"], false, "idle settings injected a script");
        assert!(!runtime.1.ready(&p.id));
        assert!(!runtime.1.automation_ready(&p.id));
        let saved = profiles::get(&canonical, &p.id, false).unwrap();
        assert!(saved.preferences.fingerprint.as_ref().unwrap().enabled);
        assert_eq!(saved.preferences.fingerprint.as_ref().unwrap().seed, seed);
        assert!(!path.join("WebDriverBiDiServer.json").exists());
        println!("Firefox app launch: idle fingerprint seed retained; no WebDriver, preload or controller");
    }
    #[test]
    #[ignore = "Starts isolated installed browsers to verify manual fingerprint startup"]
    fn native_manual_fingerprint_startup_is_separate_from_mcp() {
        let fixture = Fixture::new();
        for engine in ["chrome", "edge", "firefox"].into_iter().filter(|engine|
            std::env::var("CLAUDE_DONE_TEST_BROWSER").map(|s| s == *engine).unwrap_or(true)) {
            let mut prefs = Preferences::default();
            prefs.startup_url = "about:blank".into();
            prefs.fingerprint = Some(crate::fingerprint::fresh().unwrap());
            prefs.regional.timezone_mode = "custom".into();
            prefs.regional.timezone = "Asia/Tokyo".into();
            prefs.advanced.location = PermissionMode::Block;
            let browser = browser_for_purpose(engine, prefs, "", LaunchPurpose::Manual, |_| {});
            assert!(!browser.control.automation, "{engine}: manual launch opted into MCP");
            browser.control.open(fixture.url("/separation"), false).unwrap();
            let report = fixture.report(&browser);
            for context in [&report["main"], &report["frame"], &report["worker"]] {
                assert_eq!(context["timezone"], "Asia/Tokyo", "{engine}: manual settings not applied: {report}");
            }
            assert_eq!(report["main"]["marker"], true, "{engine}: main fingerprint missing");
            if engine != "firefox" {
                assert_eq!(report["main"]["webdriver"], false, "{engine}: manual settings startup advertises automation");
            }
            assert_eq!(report["frame"]["marker"], true, "{engine}: iframe fingerprint missing");
            if engine != "firefox" {
                assert_eq!(report["worker"]["marker"], true, "{engine}: Worker fingerprint missing");
            }
            assert!(browser.control.alive.load(Ordering::SeqCst), "{engine}: settings controller stopped");
            println!("{engine}: manual fingerprint/timezone startup remains independent of MCP");
        }
    }
    #[test]
    #[ignore = "Starts temporary Chrome/Edge to verify manual login popups across origin and opener-policy changes"]
    fn native_manual_login_popup_survives_cross_origin_redirect() {
        for engine in ["chrome", "edge"].into_iter().filter(|engine|
            std::env::var("CLAUDE_DONE_TEST_BROWSER").map(|s| s == *engine).unwrap_or(true)) {
            let fixture = Fixture::new();
            let mut prefs = Preferences::default();
            prefs.startup_url = "about:blank".into();
            prefs.fingerprint = Some(crate::fingerprint::fresh().unwrap());
            prefs.regional.timezone_mode = "custom".into();
            prefs.regional.timezone = "Europe/London".into();
            let browser = browser_for_purpose(engine, prefs, "", LaunchPurpose::Manual, |_| {});
            let opener_route = if std::env::var_os("NODECLOAK_POPUP_NOOPENER").is_some() { "/popup-opener?noopener" }
                else if std::env::var_os("NODECLOAK_POPUP_WRITE").is_some() { "/popup-opener?write" } else { "/popup-opener" };
            browser.control.open(fixture.url(opener_route), false).unwrap();
            assert_eq!(fixture.reports.recv_timeout(Duration::from_secs(20)).unwrap()["stage"], "opener");
            // A second connection drives only this disposable test profile, as
            // a native user gesture; the settings controller handles the popup.
            let socket = recovery::connect_socket(&endpoint(engine, &browser.path).unwrap()).unwrap();
            let mut driver = Protocol { worker_setups:HashMap::new(),document_setups:HashMap::new(),barrier_owner:None,socket, next: 0, events: VecDeque::new(), last_error: None, navigation: None, reconnect_endpoint: None };
            if std::env::var_os("NODECLOAK_POPUP_DORMANT").is_some() {
                let target = driver.call("Target.createTarget", json!({"url":"","background":true}), None).unwrap()["targetId"].clone();
                // Longer than the old 15-second destructive initialization deadline.
                std::thread::sleep(Duration::from_secs(18));
                let targets = driver.call("Target.getTargets", json!({}), None).unwrap();
                assert!(targets["targetInfos"].as_array().unwrap().iter().any(|t|t["targetId"] == target), "{engine}: delayed blank tab was closed");
                let attached = driver.call("Target.attachToTarget", json!({"targetId":target,"flatten":true}), None).unwrap();
                driver.call("Page.navigate", json!({"url":fixture.url("/popup-final")}), attached["sessionId"].as_str()).unwrap();
            } else {
                let targets = driver.call("Target.getTargets", json!({}), None).unwrap();
                let parent = targets["targetInfos"].as_array().unwrap().iter().find(|target|
                    target["url"].as_str().is_some_and(|url| url.contains("/popup-opener"))).unwrap()["targetId"].clone();
                let attached = driver.call("Target.attachToTarget", json!({"targetId":parent,"flatten":true}), None).unwrap();
                driver.call("Runtime.evaluate", json!({"expression":"document.querySelector('#login').click(); true","userGesture":true,"returnByValue":true}), attached["sessionId"].as_str()).unwrap();
            }
            let first = fixture.reports.recv_timeout(Duration::from_secs(20)).unwrap_or_else(|error|
                panic!("{engine}: login popup never ran its first document: {error}; controller={:?}; diagnostic={}", browser.control.error.lock().unwrap(), std::fs::read_to_string(browser.path.join("nodecloak-control.jsonl")).unwrap_or_default()));
            println!("{engine}: first popup report {first}");
            let alive = fixture.reports.recv_timeout(Duration::from_secs(15)).unwrap_or_else(|error|
                panic!("{engine}: login popup closed or stalled: {error}; first={first}; controller={:?}", browser.control.error.lock().unwrap()));
            assert_eq!(alive["stage"], "popup-alive", "{engine}: {alive}");
            assert_eq!(first["stage"], "popup-first", "{engine}: {first}");
            assert_eq!(first["timezone"], "Europe/London", "{engine}: popup lost timezone: {first}");
            assert_eq!(first["marker"], true, "{engine}: popup first script lost fingerprint: {first}");
            assert_eq!(first["webdriver"], false, "{engine}: manual popup became automated");
            let targets = driver.call("Target.getTargets", json!({}), None).unwrap();
            assert!(targets["targetInfos"].as_array().unwrap().iter().any(|target| target["url"].as_str().is_some_and(|url| url.ends_with("/popup-final"))), "{engine}: login tab was closed");
            assert!(browser.control.alive.load(Ordering::SeqCst), "{engine}: controller stopped");
            println!("{engine}: manual popup remains open with fingerprint/timezone across redirect and COOP");
        }
    }
    #[test]
    #[ignore = "Starts disposable Chrome/Edge and verifies native first-script protection after a rejected preload"]
    fn native_rejected_popup_preload_keeps_first_script_paused() {
        for engine in ["chrome", "edge"].into_iter().filter(|engine|
            std::env::var("CLAUDE_DONE_TEST_BROWSER").map(|s|s == *engine).unwrap_or(true)) {
            let fixture = Fixture::new();
            let mut prefs = Preferences::default();
            prefs.startup_url = "about:blank".into();
            prefs.regional.enabled = true;
            prefs.regional.timezone_mode = "custom".into();
            prefs.regional.timezone = "Europe/London".into();
            prefs.fingerprint = Some(crate::fingerprint::fresh().unwrap());
            let mut browser = browser_for_purpose(engine,prefs.clone(),"",LaunchPurpose::Manual,|_|{});
            browser.control.stop.store(true,Ordering::SeqCst);
            browser.control.worker.take().unwrap().join().unwrap();
            let mut state = BrowserControl::initialize(engine,&browser.path,regional::initial(&prefs).unwrap(),
                PermissionMode::Ask,false,"",prefs.advanced.clone(),prefs.fingerprint.clone(),false,
                ControlPlan::new(engine,&prefs,LaunchPurpose::Manual)).unwrap();
            state.configure().unwrap();
            state.open(&fixture.url("/popup-opener?noopener"),false).unwrap();
            assert_eq!(fixture.reports.recv_timeout(Duration::from_secs(20)).unwrap()["stage"],"opener");
            let socket = recovery::connect_socket(&endpoint(engine,&browser.path).unwrap()).unwrap();
            let mut driver = Protocol { worker_setups:HashMap::new(),document_setups:HashMap::new(),barrier_owner:None,socket,next:0,events:VecDeque::new(),last_error:None,navigation:None,reconnect_endpoint:None };
            let targets = driver.call("Target.getTargets",json!({}),None).unwrap();
            let parent = targets["targetInfos"].as_array().unwrap().iter().find(|t|t["url"].as_str().is_some_and(|u|u.contains("/popup-opener"))).unwrap()["targetId"].clone();
            let attached = driver.call("Target.attachToTarget",json!({"targetId":parent,"flatten":true}),None).unwrap();
            driver.call("Runtime.evaluate",json!({"expression":"document.querySelector('#login').click();true","userGesture":true,"returnByValue":true}),attached["sessionId"].as_str()).unwrap();
            let mut popup = None;
            let mut paused = false;
            let deadline = Instant::now()+Duration::from_secs(12);
            while Instant::now()<deadline {
                if let Some(event) = state.protocol.read().unwrap() {
                    if event["method"] == "Target.attachedToTarget" && event["params"]["targetInfo"]["type"] == "page" {
                        let session = event["params"]["sessionId"].as_str().unwrap().to_owned();
                        assert!(popup.is_none());
                        // A real protocol type error, no production fault-injection flag.
                        state.protocol.configure_document(&session,vec![
                            ("Page.enable",json!({})),
                            ("Page.addScriptToEvaluateOnNewDocument",json!({"source":42})),
                            ("Emulation.setTimezoneOverride",json!({"timezoneId":"Europe/London"})),
                            ("Runtime.runIfWaitingForDebugger",json!({})),
                        ]).unwrap();
                        state.protocol.document_setups.get_mut(&session).unwrap().started=Instant::now()-Duration::from_secs(1);
                        state.protocol.advance_document(&session).unwrap();
                        popup=Some(session);
                    } else if event["method"] == "Debugger.paused" && popup.as_deref()==event["sessionId"].as_str() {
                        paused=event["params"]["reason"]=="instrumentation";
                    } else { state.protocol.events.push_back(event); }
                }
                state.drain().unwrap();
                if paused && popup.as_ref().is_some_and(|s|state.protocol.document_setups[s].error.is_some()) { break; }
            }
            let popup=popup.expect("no popup attached");
            assert!(paused,"{engine}: failed preload did not retain native first-script pause");
            assert!(state.protocol.document_setups[&popup].error.is_some(),"{engine}: native preload rejection ignored");
            assert_eq!(state.protocol.barrier_owner.as_deref(),Some(popup.as_str()));
            assert!(fixture.reports.recv_timeout(Duration::from_secs(2)).is_err(),"{engine}: page script ran after rejected protection");
            assert!(driver.call("Target.getTargets",json!({}),None).unwrap()["targetInfos"].as_array().unwrap().len()>=2);
            println!("{engine}: real preload rejection retains first-script pause and open tab");
        }
    }
    fn check(
        data: &Value,
        language: &str,
        zone: &str,
        jan: i64,
        jul: i64,
        latitude: f64,
        longitude: f64,
    ) {
        assert_eq!(data["language"], language, "first document language {data}");
        assert_eq!(data["locale"], language, "Intl locale {data}");
        assert!(
            data["header"].as_str().unwrap().starts_with(language),
            "Accept-Language {data}"
        );
        for context in [data, &data["worker"], &data["frame"]] {
            assert_eq!(context["timezone"], zone, "timezone {data}");
            assert_eq!(context["jan"], jan, "winter offset {data}");
            assert_eq!(context["jul"], jul, "summer offset {data}");
        }
        assert_eq!(data["permission"], "granted", "permission {data}");
        assert!(
            (data["geo"]["latitude"].as_f64().unwrap() - latitude).abs() < 0.000001,
            "geo {data}"
        );
        assert!(
            (data["geo"]["longitude"].as_f64().unwrap() - longitude).abs() < 0.000001,
            "geo {data}"
        );
        if data["frame"]["geo"].get("error").is_none() {
            assert!(
                (data["frame"]["geo"]["latitude"].as_f64().unwrap() - latitude).abs() < 0.000001,
                "iframe geo {data}"
            );
            assert!(
                (data["frame"]["geo"]["longitude"].as_f64().unwrap() - longitude).abs() < 0.000001,
                "iframe geo {data}"
            );
        } else {
            // Native Geolocation waits for document visibility. Hidden frames
            // must defer rather than expose the actual device coordinates.
            assert_eq!(
                data["frame"]["visibilityNow"], "hidden",
                "visible iframe failed geolocation: {data}"
            );
            assert_eq!(
                data["frame"]["geo"]["error"], 3,
                "unexpected hidden-frame error: {data}"
            );
        }
    }
    #[test]
    #[ignore = "Starts temporary installed browsers and verifies custom/IP regions, new tabs, worker/iframe offsets and simultaneous profile isolation"]
    fn native_regions_are_observed_in_real_browsers() {
        let fixture = Fixture::new();
        for engine in ["chrome", "edge", "firefox"].into_iter().filter(|name| {
            std::env::var("CLAUDE_DONE_TEST_BROWSER")
                .map(|selected| selected == *name)
                .unwrap_or(true)
        }) {
            let mut prefs = Preferences::default();
            prefs.language = "de-DE,de,en".into();
            prefs.regional.timezone_mode = "custom".into();
            prefs.regional.timezone = "America/Los_Angeles".into();
            prefs.regional.location_mode = "custom".into();
            prefs.regional.latitude = 1.3521;
            prefs.regional.longitude = 103.8198;
            prefs.advanced.location = PermissionMode::Allow;
            let first = browser(engine, prefs, &fixture.url("/region"));
            first.control.open(fixture.url("/first"), false).unwrap();
            let data = fixture.report(&first);
            println!("{engine} custom: {data}");
            check(
                &data,
                "de-DE",
                "America/Los_Angeles",
                480,
                420,
                1.3521,
                103.8198,
            );
            assert_eq!(data["webdriver"], true);
            let mut secondprefs = Preferences::default();
            secondprefs.language = "ja-JP,ja,en".into();
            secondprefs.regional.language_mode = "ip".into();
            secondprefs.regional.timezone_mode = "ip".into();
            secondprefs.regional.location_mode = "ip".into();
            secondprefs.advanced.location = PermissionMode::Allow;
            let second = browser(engine, secondprefs, &fixture.url("/region"));
            second.control.open(fixture.url("/second"), false).unwrap();
            let data = fixture.report(&second);
            println!("{engine} IP: {data}");
            check(&data, "ja-JP", "Asia/Tokyo", -540, -540, 35.68, 139.69);
            first.show();
            first.control.open(fixture.url("/new-tab"), true).unwrap();
            let data = fixture.report(&first);
            println!("{engine} new tab: {data}");
            check(
                &data,
                "de-DE",
                "America/Los_Angeles",
                480,
                420,
                1.3521,
                103.8198,
            );
            assert_eq!(data["webdriver"], true);
        }
    }
    #[test]
    #[ignore = "Starts temporary Firefox on a protected welcome page to verify browser-network IP initialization"]
    fn native_firefox_ip_lookup_avoids_protected_startup_page() {
        let fixture = Fixture::new();
        let root = tempfile::tempdir().unwrap();
        let path = root.path().canonicalize().unwrap();
        let exe = platform::browser_path("firefox").unwrap();
        let mut child = platform::command(exe.to_str().unwrap())
            .args(["--headless", "--no-remote", "--remote-debugging-port=0", "--profile"])
            .arg(&path).args(["--new-window", "about:home"])
            .stdin(std::process::Stdio::null()).stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null()).spawn().unwrap();
        let preferences = Preferences::default();
        let result = BrowserControl::initialize("firefox", &path,
            regional::initial(&preferences).unwrap(), PermissionMode::Ask, true,
            &fixture.url("/region"), preferences.advanced.clone(), None, false,
            ControlPlan::new("firefox", &preferences, LaunchPurpose::Manual));
        // Always clean up this synthetic profile, including the failing case.
        let outcome = result.and_then(|mut state| {
            let region = state.region.ip_region.clone();
            let tree = state.protocol.call("browsingContext.getTree", json!({"maxDepth":0}), None);
            let _ = state.protocol.call("browser.close", json!({}), None);
            tree.map(|tree| (region, tree))
        });
        let _ = child.kill();
        let _ = child.wait();
        let (region, tree) = outcome.unwrap();
        assert_eq!(region.unwrap().timezone, "Asia/Tokyo");
        assert!(tree["contexts"].as_array().unwrap().iter().any(|context| context["url"] == "about:home"),
            "initialization replaced the existing Firefox home tab");
    }
    #[test]
    #[ignore = "Starts temporary Firefox; verify browser-network IP retry and restoration; use MOZ_HEADLESS=1 on an occluded test desktop"]
    fn native_firefox_ip_launch_retries_then_restores_provider() {
        let fixture = Fixture::new();
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let mut preferences = Preferences::default();
        preferences.regional.enabled = true;
        preferences.regional.language_mode = "ip".into();
        preferences.regional.timezone_mode = "ip".into();
        preferences.regional.location_mode = "ip".into();
        preferences.advanced.location = PermissionMode::Allow;
        preferences.startup_url = fixture.url("/launch-ip");
        let mut draft = Draft {
            name: "Native provider launch".into(),
            browser: "firefox".into(),
            notes: String::new(),
            tags: vec![],
            proxy: ProxyConfig {
                mode: "direct".into(),
                ..Default::default()
            },
            password: None,
            clear_password: false,
            preferences,
        };
        let p = profiles::create(&canonical, draft.clone(), None).unwrap();
        let path = profiles::path(&canonical, &p).unwrap();
        let original = json!({"user":"https://original.example/location","runtime":"https://original.example/location"});
        crate::firefox::apply(
            &path,
            &[(
                "geo.provider.network.url".into(),
                crate::firefox::snapshot(&path, "geo.provider.network.url").unwrap(),
                original.clone(),
            )],
            false,
        )
        .unwrap();
        let runtime = profiles::Runtime::default();
        let result = crate::profile_commands::launch_with_lookup(
            &canonical,
            &p.id,
            "startup",
            &runtime,
            &fixture.url("/region"),
        )
        .and_then(|_| {
            let data = fixture
                .reports
                .recv_timeout(Duration::from_secs(20))
                .map_err(|_| "Firefox IP report timeout")?;
            println!("Firefox native launch IP: {data}");
            check(&data, "ja-JP", "Asia/Tokyo", -540, -540, 35.68, 139.69);
            assert!(runtime.1.ready(&p.id));
            assert!(profiles::get(&canonical, &p.id, false)?
                .last_region
                .unwrap()
                .ip_region
                .is_some());
            Ok(())
        });
        if let Ok(controls) = runtime.1 .0.lock() {
            if let Some(control) = controls.get(&p.id) {
                let _ = control.quit_before_navigation();
            }
        }
        runtime.1.clear();
        platform::close_profile(&path).unwrap();
        result.unwrap();
        draft.preferences.regional.language_mode = "custom".into();
        draft.preferences.regional.timezone_mode = "system".into();
        draft.preferences.regional.location_mode = "system".into();
        draft.preferences.startup_url = "about:blank".into();
        profiles::update(&canonical, &p.id, draft).unwrap();
        let result = crate::profile_commands::launch_with_lookup(
            &canonical,
            &p.id,
            "startup",
            &runtime,
            &fixture.url("/region"),
        );
        platform::force_close_profile(&path).unwrap();
        result.unwrap();
        assert_eq!(
            crate::firefox::snapshot(&path, "geo.provider.network.url").unwrap(),
            original
        );
        assert!(!path.join("nodecloak-geolocation-original.json").exists());
    }
    #[test]
    #[ignore = "Uses the live HTTPS region provider through a temporary Chrome profile and tests the complete profile launch/relaunch path"]
    fn live_ip_matching_uses_browser_network_path() {
        let fixture = Fixture::new();
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let mut preferences = Preferences::default();
        preferences.regional.language_mode = "ip".into();
        preferences.regional.timezone_mode = "ip".into();
        preferences.regional.location_mode = "ip".into();
        preferences.advanced.location = PermissionMode::Allow;
        preferences.startup_url = fixture.url("/live-ip");
        let p = profiles::create(
            &canonical,
            Draft {
                name: "Live IP verification".into(),
                browser: "chrome".into(),
                notes: String::new(),
                tags: vec![],
                proxy: ProxyConfig {
                    mode: "direct".into(),
                    ..ProxyConfig::default()
                },
                password: None,
                clear_password: false,
                preferences,
            },
            None,
        )
        .unwrap();
        let path = profiles::path(&canonical, &p).unwrap();
        let runtime = profiles::Runtime::default();
        let result = crate::profile_commands::launch(&canonical, &p.id, "startup", &runtime)
            .and_then(|_| {
                let data = fixture
                    .reports
                    .recv_timeout(Duration::from_secs(20))
                    .map_err(|_| "live browser report timeout")?;
                let p = profiles::get(&canonical, &p.id, false)?;
                let actual = p.last_region.unwrap();
                let zone = actual
                    .timezone
                    .as_ref()
                    .unwrap()
                    .parse::<chrono_tz::Tz>()
                    .unwrap();
                use chrono::{Offset, TimeZone};
                let jan = -(zone
                    .with_ymd_and_hms(2026, 1, 15, 12, 0, 0)
                    .unwrap()
                    .offset()
                    .fix()
                    .local_minus_utc() as i64)
                    / 60;
                let jul = -(zone
                    .with_ymd_and_hms(2026, 7, 15, 12, 0, 0)
                    .unwrap()
                    .offset()
                    .fix()
                    .local_minus_utc() as i64)
                    / 60;
                check(
                    &data,
                    actual.language.split(',').next().unwrap(),
                    actual.timezone.as_deref().unwrap(),
                    jan,
                    jul,
                    actual.latitude.unwrap(),
                    actual.longitude.unwrap(),
                );
                assert!(runtime.1.ready(&p.id));
                assert!(actual.ip_region.is_some());
                println!(
                    "live IP matching verified: {} / {}",
                    actual.language,
                    actual.timezone.unwrap()
                );
                Ok(())
            });
        runtime.1.clear();
        platform::close_profile(&path).unwrap();
        result.unwrap();
    }
}
