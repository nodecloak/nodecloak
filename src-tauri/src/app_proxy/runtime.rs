use super::{
    discovery::{self, DetectedApp},
    model::*,
};
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tokio::sync::Mutex as AsyncMutex;
#[cfg(any(windows, target_os = "macos"))]
async fn wait_cancelled(cancel: Arc<AtomicBool>) {
    while !cancel.load(Ordering::Acquire) {
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}
/// Only read-only preparation may run here. Dropping a cancelled blocking
/// task must never leave an authorization launcher or privileged side effect.
#[cfg(any(windows, target_os = "macos"))]
pub(super) async fn prepare<T: Send + 'static>(
    cancel: Arc<AtomicBool>,
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    if cancel.load(Ordering::Acquire) {
        return Err("应用代理启动已取消".into());
    }
    let task = tokio::task::spawn_blocking(work);
    tokio::select! {
        biased;
        _=wait_cancelled(cancel.clone())=>Err("应用代理启动已取消".into()),
        result=task=>{
            if cancel.load(Ordering::Acquire){return Err("应用代理启动已取消".into())}
            result.map_err(|_|"应用代理准备任务中断")?
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Connection {
    pub id: String,
    pub app: String,
    pub process_path: String,
    pub destination: String,
    pub network: String,
    pub proxy_id: String,
    pub upload: u64,
    pub download: u64,
    pub seen_at: String,
}
#[derive(Clone, Serialize, Deserialize)]
pub struct Event {
    pub at: String,
    pub event: String,
    pub message: String,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Status {
    pub phase: String,
    pub interface: Option<String>,
    pub since: Option<String>,
    pub error: Option<String>,
    pub needs_cleanup: bool,
    pub protection_active: bool,
    pub connections: Vec<Connection>,
    pub events: Vec<Event>,
    pub warnings: Vec<String>,
    #[serde(skip)]
    pub session: Option<String>,
}
impl Default for Status {
    fn default() -> Self {
        Self {
            phase: "off".into(),
            interface: None,
            since: None,
            error: None,
            needs_cleanup: false,
            protection_active: false,
            connections: vec![],
            events: vec![],
            warnings: vec![],
            session: None,
        }
    }
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    #[cfg(windows)]
    pub wsl: Vec<super::wsl::Distribution>,
    pub supported: bool,
    pub platform: &'static str,
    pub settings: Settings,
    pub apps: Vec<DetectedApp>,
    pub discovery_checked: bool,
    pub runtime: Status,
}
pub struct Runtime {
    #[cfg(windows)]
    wsl: Mutex<Vec<super::wsl::Distribution>>,
    pub gate: AsyncMutex<()>,
    pub state: Arc<Mutex<Status>>,
    pub apps: Mutex<(bool, Vec<DetectedApp>)>,
    cancel: Arc<AtomicBool>,
    history_loaded: AtomicBool,
    #[cfg(any(windows, target_os = "macos"))]
    active: AsyncMutex<Option<Session>>,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            #[cfg(windows)]
            wsl: Mutex::new(vec![]),
            gate: AsyncMutex::new(()),
            state: Arc::new(Mutex::new(Status::default())),
            apps: Mutex::new((false, discovery::empty_apps())),
            cancel: Arc::new(AtomicBool::new(false)),
            history_loaded: AtomicBool::new(false),
            #[cfg(any(windows, target_os = "macos"))]
            active: AsyncMutex::new(None),
        }
    }
}
impl Runtime {
    pub fn rejected_start(&self, root: &Path, error: &str) {
        let log = if let Ok(mut state) = self.state.lock() {
            if ["off", "error"].contains(&state.phase.as_str()) {
                state.phase = "error".into();
                state.error = Some(error.into());
            }
            state
                .events
                .last()
                .is_none_or(|entry| entry.message != error)
        } else {
            false
        };
        if log {
            self.event(root, "start_rejected", error);
        }
    }
    pub fn view(&self, root: &Path) -> Result<View, String> {
        #[cfg(windows)]
        {
            let mut status=self.state.lock().map_err(|_|"应用代理状态不可用")?;
            status.protection_active=super::windows::protection::active()?||super::wsl::protected(root)?;
            if super::windows::protection::pending()? && status.phase!="running" && status.phase!="starting" {
                status.needs_cleanup=true;
                status.phase="error".into();
                status.error=Some("断网保护初始化未完成，请点击停止应用代理以恢复，再重新开启".into());
            }
        }
        if !self.history_loaded.swap(true, Ordering::AcqRel) {
            let path = root.join("application-proxy-events.jsonl");
            if crate::profiles::guard_tree(&path).is_ok()
                && std::fs::metadata(&path).is_ok_and(|m| m.len() <= 262144)
            {
                if let Ok(text) = std::fs::read_to_string(path) {
                    if let Ok(mut state) = self.state.lock() {
                        let mut history: Vec<Event> = text
                            .lines()
                            .filter_map(|s| serde_json::from_str(s).ok())
                            .collect();
                        history.append(&mut state.events);
                        let start = history.len().saturating_sub(100);
                        state.events = history.drain(start..).collect();
                    }
                }
            }
        }
        let apps = self.apps.lock().map_err(|_| "应用识别状态不可用")?;
        Ok(View {
            #[cfg(windows)]
            wsl: self.wsl.lock().map_err(|_|"WSL 接入状态不可用")?.clone(),
            supported: cfg!(any(windows, target_os = "macos")),
            platform: if cfg!(target_os = "macos") {
                "macos"
            } else if cfg!(windows) {
                "windows"
            } else {
                "unsupported"
            },
            settings: super::store::load(root)?,
            apps: apps.1.clone(),
            discovery_checked: apps.0,
            runtime: self.state.lock().map_err(|_| "应用代理状态不可用")?.clone(),
        })
    }
    pub fn editable(&self, _root:&Path) -> Result<(), String> {
        #[cfg(windows)]
        if super::windows::protection::needs_recovery()? || super::wsl::protected(_root)? {
            return Err("请先停止应用代理并解除断网保护，再修改规则或节点".into());
        }
        let status = self.state.lock().map_err(|_| "应用代理状态不可用")?;
        if ["starting", "running", "stopping"].contains(&status.phase.as_str())
            || status.needs_cleanup
            || status.protection_active
        {
            return Err("请先停止应用代理，再修改规则或节点".into());
        }
        Ok(())
    }
    fn event(&self, root: &Path, event: &str, message: &str) {
        record(&self.state, root, event, message);
    }
    pub async fn discover(&self, _root:&Path) -> Result<(), String> {
        #[cfg(windows)]
        {
            let root=_root.to_owned();
            let entries=tokio::task::spawn_blocking(move||super::wsl::discover(&root)).await.map_err(|_|"WSL 识别中断")??;
            *self.wsl.lock().map_err(|_|"WSL 接入状态不可用")?=entries;
        }
        let apps = tokio::task::spawn_blocking(discovery::discover)
            .await
            .map_err(|_| "应用识别任务中断")??;
        *self.apps.lock().map_err(|_| "应用识别状态不可用")? = (true, apps);
        Ok(())
    }
    pub async fn start(&self, root: &Path) -> Result<(), String> {
        #[cfg(not(any(windows, target_os = "macos")))]
        {
            let _ = root;
            return Err("应用代理支持 Windows 和 macOS".into());
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let _gate = self.gate.lock().await;
            self.editable(root)?;
            let stale = self.active.lock().await.take();
            if let Some(mut stale) = stale {
                if let Err(error) = stale.close().await {
                    *self.active.lock().await = Some(stale);
                    return Err(error);
                }
            }
            self.cancel.store(false, Ordering::Release);
            let settings = super::store::load(root)?;
            let apps = prepare(self.cancel.clone(), discovery::discover).await?;
            *self.apps.lock().map_err(|_| "应用识别状态不可用")? = (true, apps);
            let apps = self
                .apps
                .lock()
                .map_err(|_| "应用识别状态不可用")?
                .1
                .clone();
            let settings_copy = settings.clone();
            #[cfg(windows)]
            let wsl=super::wsl::prepare(root,&settings).await?;
            #[cfg(not(windows))]
            let wsl:Vec<super::wsl::Ingress>=vec![];
            if self.cancel.load(Ordering::Acquire){return Err("应用代理启动已取消".into());}
            let wsl_copy=wsl.clone();
            let (rules, warnings) =
                prepare(self.cancel.clone(), move || resolve_with_wsl(&settings_copy, &apps,&wsl_copy)).await?;
            let mut filtered = settings.clone();
            filtered
                .rules
                .retain(|r| rules.iter().any(|rule| rule.id == r.id)||wsl.iter().any(|i|i.app==r.app&&r.executable.is_empty()&&r.enabled));
            let proxies =
                super::store::resolved_proxies(&filtered, &super::store::NativeCredentials)?;
            let reserved = tokio::net::TcpListener::bind("127.0.0.1:0")
                .await
                .map_err(|_| "无法分配应用代理诊断端口")?;
            let port = reserved
                .local_addr()
                .map_err(|_| "无法读取应用代理诊断端口")?
                .port();
            drop(reserved);
            let session = uuid::Uuid::new_v4().to_string();
            let secret = format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            );
            super::config::compile_for_wsl(super::config::Platform::current(),&rules, &proxies, port, &secret, &session,&wsl)?;
            let interface = super::config::interface_name(&session)?;
            super::driver::network::preflight(&super::driver::network::snapshot()?, &interface)?;
            {
                let mut state = self.state.lock().map_err(|_| "应用代理状态不可用")?;
                state.phase = "starting".into();
                state.error = None;
                state.interface = Some(interface.clone());
                state.session = Some(session.clone());
                state.warnings = warnings;
                state.connections.clear();
            }
            self.event(
                root,
                "starting",
                if cfg!(target_os = "macos") {
                    "等待 macOS 管理员授权，正在准备应用代理"
                } else {
                    "等待 Windows 管理员授权，正在准备应用代理"
                },
            );
            let request = super::ipc::StartRequest {
                wsl,
            stop_only: false,
                session: session.clone(),
                controller_port: port,
                controller_secret: secret.clone(),
                rules: rules.clone(),
                proxies,
            };
            let result = Session::start(
                request,
                self.state.clone(),
                self.cancel.clone(),
                settings,
                rules,
                root.to_owned(),
            )
            .await;
            match result {
                Ok(active) => {
                    *self.active.lock().await = Some(active);
                    self.event(
                        root,
                        "started",
                        "应用代理已开启；未匹配的应用直连，系统 DNS 设置保持不变",
                    );
                    Ok(())
                }
                Err(error) => {
                    let mut state = self.state.lock().map_err(|_| "应用代理状态不可用")?;
                    state.phase = "error".into();
                    state.error = Some(error.clone());
                    state.needs_cleanup = state.needs_cleanup
                        || super::driver::network::interface_luid(&interface)
                            .ok()
                            .flatten()
                            .is_some();
                    drop(state);
                    self.event(root, "start_failed", &error);
                    Err(error)
                }
            }
        }
    }
    pub async fn stop(&self, root: &Path) -> Result<(), String> {
        self.cancel.store(true, Ordering::Release);
        let _gate = self.gate.lock().await;
        #[cfg(windows)]
        let protected=super::windows::protection::needs_recovery()?||super::wsl::protected(root)?;
        #[cfg(not(windows))]
        let protected=false;
        if !protected && self
            .state
            .lock()
            .is_ok_and(|s| s.phase == "off" && !s.needs_cleanup)
        {
            return Ok(());
        }
        #[cfg(any(windows, target_os = "macos"))]
        {
            let active = self.active.lock().await.take();
            if let Some(mut active) = active {
                if let Ok(mut state) = self.state.lock() {
                    state.phase = "stopping".into();
                }
                if let Err(error) = active.close().await {
                    if let Ok(mut state) = self.state.lock() {
                        state.phase = "error".into();
                        state.error = Some(error.clone());
                        state.needs_cleanup = true;
                    }
                    *self.active.lock().await = Some(active);
                    self.event(root, "stop_failed", &error);
                    return Err(error);
                }
            } else if !protected && self.state.lock().is_ok_and(|s| s.needs_cleanup) {
                let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
                while self.state.lock().is_ok_and(|s| s.needs_cleanup)
                    && tokio::time::Instant::now() < deadline
                {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
                if self.state.lock().is_ok_and(|s| s.needs_cleanup) {
                    return Err("应用代理接口仍待释放，请查看诊断记录后重试".into());
                }
            }
        }
        #[cfg(windows)]
        if super::windows::protection::needs_recovery()? {
            if let Err(error)=clear_protection().await {
                self.event(root,"protection_release_failed",&error);
                return Err(error);
            }
        }
        #[cfg(windows)]
        if let Err(error)=super::wsl::stop(root).await {
            if let Ok(mut state)=self.state.lock(){state.phase="error".into();state.error=Some(error.clone());state.needs_cleanup=true;}
            self.event(root,"wsl_stop_failed",&error);return Err(error);
        }
        if let Ok(mut state) = self.state.lock() {
            state.phase = "off".into();
            state.interface = None;
            state.since = None;
            state.error = None;
            state.needs_cleanup = false;
            state.protection_active = false;
            state.connections.clear();
            state.session = None;
        }
        self.event(
            root,
            "stopped",
            "应用代理已停止，已释放本次会话的网络接口和路由",
        );
        Ok(())
    }
}
fn record(state: &Arc<Mutex<Status>>, root: &Path, event: &str, message: &str) {
    let entry = Event {
        at: chrono::Utc::now().to_rfc3339(),
        event: event.into(),
        message: message.into(),
    };
    // Serialize memory and disk appends so background errors retain chronology.
    let Ok(mut state) = state.lock() else { return };
    state.events.push(entry.clone());
    if state.events.len() > 100 {
        state.events.remove(0);
    }
    let path = root.join("application-proxy-events.jsonl");
    if crate::profiles::guard_tree(&path).is_ok() {
        use std::io::Write;
        if std::fs::metadata(&path).is_ok_and(|m| m.len() > 262144) {
            let _ = std::fs::remove_file(&path);
        }
        if let Ok(mut file) = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(path)
        {
            if let Ok(mut bytes) = serde_json::to_vec(&entry) {
                bytes.push(b'\n');
                let _ = file.write_all(&bytes);
            }
        }
    }
}
#[cfg(windows)]
pub(super) async fn clear_protection()->Result<(),String>{
    use super::ipc::{self,StartRequest,Reply};
    let (mut pipe,helper)=super::windows::transport::connect_helper().await?;
    let request=StartRequest{stop_only:true,wsl:vec![],session:uuid::Uuid::new_v4().to_string(),controller_port:0,controller_secret:String::new(),rules:vec![],proxies:vec![]};
    ipc::write_frame(&mut pipe,&request).await?;
    let reply:Reply=tokio::time::timeout(Duration::from_secs(15),ipc::read_frame(&mut pipe)).await.map_err(|_|"解除断网保护超时，保护仍保留")??;
    match reply{Reply::Stopped=>{},Reply::Failure{message}=>return Err(message),_=>return Err("解除断网保护响应无效".into())}
    drop(pipe);
    let ended=tokio::task::spawn_blocking(move||helper.wait(12000)).await.map_err(|_|"等待断网保护控制程序退出失败")?;
    if !ended||super::windows::protection::active()?{return Err("断网保护尚未解除，请重试停止应用代理".into());}
    Ok(())
}
pub(super) fn resolve(
    settings: &Settings,
    apps: &[DetectedApp],
) -> Result<(Vec<ResolvedRule>, Vec<String>), String> {
    resolve_with_wsl(settings,apps,&[])
}
fn resolve_with_wsl(settings:&Settings,apps:&[DetectedApp],wsl:&[super::wsl::Ingress])->Result<(Vec<ResolvedRule>, Vec<String>), String> {
    let mut rules = vec![];
    let mut warnings = vec![];
    for rule in settings.rules.iter().filter(|r| r.enabled) {
        let (name, paths) = if !rule.executable.is_empty() {
            let file = std::path::Path::new(&rule.executable);
            (
                file.file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned(),
                if cfg!(target_os = "macos") {
                    super::macos_bundle::executables(file)?
                } else {
                    vec![rule.executable.clone()]
                },
            )
        } else if let Some(app) = apps.iter().find(|a| a.id == rule.app) {
            (app.name.clone(), app.paths.clone())
        } else {
            continue;
        };
        let paths: Vec<_> = paths
            .into_iter()
            .filter(|p| std::path::Path::new(p).is_file())
            .map(|p| discovery::canonical_executable(&p))
            .collect();
        if paths.is_empty() {
            if wsl.iter().any(|i|i.app==rule.app&&rule.executable.is_empty()){continue;}
            warnings.push(format!("未找到 {name} 的程序文件，本次不启用该规则"));
            continue;
        }
        let proxy_id = rule.proxy_id.clone().ok_or("请先为应用选择代理节点")?;
        rules.push(ResolvedRule {
            id: rule.id.clone(),
            name,
            paths,
            proxy_id,
            exact_paths: rule.app == "custom" || !rule.executable.is_empty(),
        });
    }
    if rules.is_empty()&&wsl.is_empty() {
        return Err("请至少为一个已安装应用选择代理节点".into());
    }
    warnings.push("开启后新建立的连接按规则分流；已有连接请通过应用自身重新连接。".into());
    warnings.push("系统 DNS 保持原设置。应用自身的代理设置和本机回环连接不会被 TUN 覆盖。".into());
    Ok((rules, warnings))
}

#[cfg(any(windows, target_os = "macos"))]
struct Session {
    helper: Arc<super::driver::transport::Handle>,
    writer: Option<tokio::io::WriteHalf<super::driver::transport::Server>>,
    reader: tokio::task::JoinHandle<()>,
    monitor: tokio::task::JoinHandle<()>,
    interface: String,
    luid: u64,
}
#[cfg(any(windows, target_os = "macos"))]
impl Session {
    async fn start(
        request: super::ipc::StartRequest,
        state: Arc<Mutex<Status>>,
        cancel: Arc<AtomicBool>,
        settings: Settings,
        rules: Vec<ResolvedRule>,
        root: std::path::PathBuf,
    ) -> Result<Self, String> {
        use super::ipc::{self, Reply};
        // Validate the complete expanded request before opening an elevation
        // prompt. The receiver independently keeps the same frame limit.
        ipc::validate_size(&request)?;
        #[cfg(target_os = "macos")]
        let connect = super::driver::transport::connect_helper_cancellable(cancel.clone());
        #[cfg(windows)]
        let connect = super::driver::transport::connect_helper();
        let (pipe, helper) = tokio::select! {
            biased;
            _=wait_cancelled(cancel.clone())=>return Err("应用代理启动已取消".into()),
            result=connect=>result?,
        };
        let helper = Arc::new(helper);
        let mut startup = StartingSession {
            pipe: Some(pipe),
            helper: helper.clone(),
            state: state.clone(),
            session: request.session.clone(),
            interface: super::config::interface_name(&request.session)?,
            armed: true,
        };
        if cancel.load(Ordering::Acquire) {
            return Err("应用代理启动已取消".into());
        }
        let pipe = startup.pipe.as_mut().unwrap();
        ipc::write_frame(pipe, &request).await?;
        let ready: Reply = tokio::select! {
            ready=tokio::time::timeout(Duration::from_secs(65),ipc::read_frame(pipe))=>ready.map_err(|_|"等待应用代理内核和接口超时")??,
            _=wait_cancelled(cancel.clone())=>return Err("应用代理启动已取消".into()),
        };
        let (interface, luid) = match ready {
            Reply::Ready { interface, luid } => (interface, luid),
            Reply::Failure { message } => return Err(message),
            _ => return Err("应用代理启动响应无效".into()),
        };
        if interface != super::config::interface_name(&request.session)?
            || !helper.alive()
            || super::driver::network::interface_luid(&interface)? != Some(luid)
        {
            return Err("应用代理接口身份或进程状态不匹配".into());
        }
        if cancel.load(Ordering::Acquire) {
            ipc::write_frame(pipe, &ipc::Control::Stop).await.ok();
            return Err("应用代理启动已取消".into());
        }
        let session = request.session.clone();
        let reader_state = state.clone();
        let reader_session = session.clone();
        {
            let mut status = state.lock().map_err(|_| "应用代理状态不可用")?;
            status.phase = "running".into();
            status.since = Some(chrono::Utc::now().to_rfc3339());
            status.needs_cleanup = true;
        }
        startup.armed = false;
        let (mut input, writer) = tokio::io::split(startup.pipe.take().unwrap());
        let reader = tokio::spawn(async move {
            let reply = ipc::read_frame::<_, Reply>(&mut input).await;
            let message = match reply {
                Ok(Reply::Failure { message }) => message,
                Ok(Reply::Stopped) => "应用代理已停止".into(),
                _ => "应用代理控制连接已关闭；正在确认网络接口释放".into(),
            };
            let unexpected = if let Ok(mut status) = reader_state.lock() {
                if status.session.as_deref() == Some(&reader_session) && status.phase != "stopping"
                {
                    status.phase = "error".into();
                    status.error = Some(message.clone());
                    true
                } else {
                    false
                }
            } else {
                false
            };
            if unexpected {
                record(&reader_state, &root, "runtime_failed", &message);
            }
        });
        let monitor_helper = helper.clone();
        let monitor_interface = interface.clone();
        let monitor = tokio::spawn(async move {
            let Ok(client) = reqwest::Client::builder()
                .no_proxy()
                .redirect(reqwest::redirect::Policy::none())
                .timeout(Duration::from_secs(2))
                .build()
            else {
                return;
            };
            loop {
                if !monitor_helper.alive() {
                    let released =
                        super::driver::network::released(&monitor_interface, luid).unwrap_or(false);
                    if let Ok(mut status) = state.lock() {
                        if status.session.as_deref() == Some(&session) {
                            status.needs_cleanup = !released;
                        }
                    }
                    break;
                }
                if let Ok(mut response) = client
                    .get(format!(
                        "http://127.0.0.1:{}/connections",
                        request.controller_port
                    ))
                    .bearer_auth(&request.controller_secret)
                    .send()
                    .await
                {
                    if response.status().is_success() {
                        let mut bytes = Vec::new();
                        let mut complete = true;
                        loop {
                            match response.chunk().await {
                                Ok(Some(chunk)) if bytes.len() + chunk.len() <= 2_097_152 => {
                                    bytes.extend_from_slice(&chunk)
                                }
                                Ok(None) => break,
                                _ => {
                                    complete = false;
                                    break;
                                }
                            }
                        }
                        if complete {
                            if let Ok(value) = serde_json::from_slice(&bytes) {
                                let connections = connections(&value, &settings, &rules, &request.wsl);
                                if let Ok(mut status) = state.lock() {
                                    if status.session.as_deref() == Some(&session)
                                        && status.phase == "running"
                                    {
                                        status.connections = connections;
                                    }
                                }
                            }
                        }
                    }
                }
                tokio::time::sleep(Duration::from_secs(1)).await;
            }
        });
        Ok(Self {
            helper,
            writer: Some(writer),
            reader,
            monitor,
            interface,
            luid,
        })
    }
    async fn close(&mut self) -> Result<(), String> {
        if let Some(writer) = self.writer.as_mut() {
            let _ = tokio::time::timeout(
                Duration::from_secs(1),
                super::ipc::write_frame(writer, &super::ipc::Control::Stop),
            )
            .await;
        }
        // Keep both halves alive until the helper acknowledges Stop and exits.
        // Dropping a named-pipe server can discard a queued unread Stop frame,
        // which must correctly be treated as an unexpected EOF by the helper.
        #[cfg(target_os = "macos")]
        self.helper.close_control();
        let helper = self.helper.clone();
        let exited = tokio::task::spawn_blocking(move || helper.wait(12000))
            .await
            .map_err(|_| "等待应用代理停止的任务中断")?;
        if !exited {
            return Err("应用代理仍在退出，请重试停止".into());
        }
        self.reader.abort();
        self.writer.take();
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while tokio::time::Instant::now() < deadline {
            if super::driver::network::released(&self.interface, self.luid)? {
                self.monitor.abort();
                return Ok(());
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        Err("应用代理接口仍待释放，请重试停止".into())
    }
}
#[cfg(any(windows, target_os = "macos"))]
impl Drop for Session {
    fn drop(&mut self) {
        self.reader.abort();
        self.monitor.abort();
        self.writer.take();
        #[cfg(target_os = "macos")]
        self.helper.close_control();
    }
}

#[cfg(any(windows, target_os = "macos"))]
struct StartingSession {
    pipe: Option<super::driver::transport::Server>,
    helper: Arc<super::driver::transport::Handle>,
    state: Arc<Mutex<Status>>,
    session: String,
    interface: String,
    armed: bool,
}
#[cfg(any(windows, target_os = "macos"))]
impl Drop for StartingSession {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        self.pipe.take();
        #[cfg(target_os = "macos")]
        self.helper.close_control();
        if let Ok(mut state) = self.state.lock() {
            if state.session.as_deref() == Some(&self.session) {
                state.needs_cleanup = true;
            }
        }
        let helper = self.helper.clone();
        let state = self.state.clone();
        let session = self.session.clone();
        let interface = self.interface.clone();
        tokio::spawn(async move {
            let wait = helper.clone();
            let _ = tokio::task::spawn_blocking(move || wait.wait(15000)).await;
            let deadline = tokio::time::Instant::now() + Duration::from_secs(30);
            while helper.alive() && tokio::time::Instant::now() < deadline {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let released = !helper.alive()
                && super::driver::network::session_released(&interface, None).unwrap_or(false);
            if let Ok(mut status) = state.lock() {
                if status.session.as_deref() == Some(&session) {
                    status.needs_cleanup = !released;
                }
            }
        });
    }
}

pub(super) fn connections(
    value: &serde_json::Value,
    _settings: &Settings,
    rules: &[ResolvedRule],
    wsl: &[super::wsl::Ingress],
) -> Vec<Connection> {
    let mut result = vec![];
    for row in value["connections"]
        .as_array()
        .into_iter()
        .flatten()
        .take(2000)
    {
        let metadata = &row["metadata"];
        let path = metadata["processPath"].as_str().filter(|s| s.len() <= 2048).unwrap_or("");
        let rule = rules.iter().find(|r| {
            row["chains"].as_array().is_some_and(|chains| {
                chains
                    .iter()
                    .any(|c| c.as_str() == Some(&format!("proxy-{}", r.proxy_id)))
            }) && r.paths.iter().any(|known| {
                if r.exact_paths {
                    discovery::normalize(&discovery::canonical_executable(known))
                        == discovery::normalize(&discovery::canonical_executable(path))
                } else {
                    super::config::path_pattern(known) == super::config::path_pattern(path)
                }
            })
        });
        let ingress=wsl.iter().find(|i|metadata["type"]==format!("http/{}",i.tag())&&row["chains"].as_array().is_some_and(|chains|chains.iter().any(|c|c==&format!("proxy-{}",i.proxy_id))));
        let (app,process_path,proxy_id)=if let Some(i)=ingress {
            (format!("{} (WSL)",if i.app=="codex"{"Codex"}else{"Claude"}),format!("WSL / {}",i.app),i.proxy_id.clone())
        } else if let Some(rule)=rule {(rule.name.clone(),path.to_owned(),rule.proxy_id.clone())}else{continue;};
        let host = metadata["host"]
            .as_str()
            .filter(|s| !s.is_empty())
            .or_else(|| metadata["destinationIP"].as_str())
            .unwrap_or("");
        if host.is_empty()
            || host.len() > 253
            || host.contains(['/', '@', '?', '#'])
            || host.chars().any(char::is_control)
        {
            continue;
        }
        let port = metadata["destinationPort"]
            .as_str()
            .and_then(|p| p.parse::<u16>().ok())
            .map(|p| p.to_string())
            .unwrap_or_default();
        let id = row["id"].as_str().filter(|s| s.len() <= 128).unwrap_or("");
        if id.is_empty() {
            continue;
        }
        result.push(Connection {
            id: id.into(),
            app,
            process_path,
            destination: format!("{host}:{port}"),
            network: metadata["network"]
                .as_str()
                .filter(|s| ["tcp", "udp"].contains(s))
                .unwrap_or("tcp")
                .into(),
            proxy_id,
            upload: row["upload"].as_u64().unwrap_or(0),
            download: row["download"].as_u64().unwrap_or(0),
            seen_at: chrono::Utc::now().to_rfc3339(),
        });
        if result.len() >= 100 {
            break;
        }
    }
    result
}

#[cfg(test)]
mod checks {
    use super::*;
    use serde_json::json;
    #[test]
    fn inactive_runtime_cannot_edit_rules_while_direct_connection_protection_remains(){
        let runtime=Runtime::default();
        runtime.state.lock().unwrap().protection_active=true;
        assert!(runtime.editable(tempfile::tempdir().unwrap().path()).is_err());
    }
    #[test]
    fn restart_keeps_diagnostics_but_never_restores_a_running_session() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let first = Runtime::default();
        first.event(root.path(), "synthetic", "Synthetic lifecycle event");
        let next = Runtime::default();
        let view = next.view(root.path()).unwrap();
        assert_eq!(view.runtime.phase, "off");
        assert!(!view.runtime.needs_cleanup);
        assert!(view.runtime.connections.is_empty());
        assert_eq!(view.runtime.events.len(), 1);
        assert_eq!(view.runtime.events[0].event, "synthetic");
        assert_eq!(next.view(root.path()).unwrap().runtime.events.len(), 1);
    }
    #[cfg(any(windows, target_os = "macos"))]
    #[tokio::test]
    async fn cancellation_wakes_startup_without_waiting_for_readiness_timeout() {
        let cancel = Arc::new(AtomicBool::new(false));
        let wake = cancel.clone();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(20)).await;
            wake.store(true, Ordering::Release);
        });
        tokio::time::timeout(Duration::from_millis(500), wait_cancelled(cancel))
            .await
            .unwrap();
    }
    #[cfg(any(windows, target_os = "macos"))]
    #[tokio::test]
    async fn cancellation_during_blocking_scan_cannot_later_request_authorization() {
        let cancel = Arc::new(AtomicBool::new(false));
        let authorized = Arc::new(AtomicBool::new(false));
        let observed = authorized.clone();
        let (release, blocked) = std::sync::mpsc::channel();
        let (entered, started) = tokio::sync::oneshot::channel();
        let wake = cancel.clone();
        let start = tokio::spawn(async move {
            prepare(cancel, move || {
                let _ = entered.send(());
                let _ = blocked.recv();
                Ok(())
            })
            .await?;
            authorized.store(true, Ordering::Release);
            Ok::<_, String>(())
        });
        started.await.unwrap();
        wake.store(true, Ordering::Release);
        let result = tokio::time::timeout(Duration::from_millis(500), start)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
        assert!(!observed.load(Ordering::Acquire));
        release.send(()).unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!observed.load(Ordering::Acquire));
    }
    #[test]
    fn connection_evidence_requires_both_the_assigned_proxy_and_the_application_path() {
        let id = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
        let rules = vec![ResolvedRule {
            id: "d75f9dd8-9a66-4e26-b21c-4ce4e8f1863d".into(),
            name: "Codex".into(),
            paths: vec![r"C:\Apps\Codex.exe".into()],
            proxy_id: id.into(),
            exact_paths: true,
        }];
        let value = json!({"connections":[
            {"id":"matched","metadata":{"processPath":"C:\\APPS\\CODEX.exe","host":"example.com","destinationIP":"203.0.113.1","destinationPort":"443","network":"tcp"},"chains":[format!("proxy-{id}")],"upload":12,"download":34},
            {"id":"direct","metadata":{"processPath":"C:\\Apps\\Codex.exe","host":"private.example","destinationPort":"443","network":"tcp"},"chains":["direct"]},
            {"id":"unrelated","metadata":{"processPath":"C:\\Apps\\Other.exe","host":"private.example","destinationPort":"443","network":"tcp"},"chains":[format!("proxy-{id}")]}
        ]});
        let results = connections(&value, &Settings::default(), &rules, &[]);
        assert_eq!(results.len(), 1);
        assert_eq!(results[0].id, "matched");
        assert_eq!(results[0].app, "Codex");
        assert_eq!(results[0].destination, "example.com:443");
        assert_eq!(results[0].download, 34);
    }
}
