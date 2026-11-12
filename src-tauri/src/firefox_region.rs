//! Native regional configuration for manual Firefox. No WebDriver session.
use crate::{
    profiles::{PermissionMode, Preferences},
    regional::ResolvedRegion,
};
use serde_json::{json, Value};
use std::{
    io::{Read, Write},
    net::{TcpListener, TcpStream},
    path::Path,
    time::{Duration, Instant},
};

const SCRIPT: &str = include_str!("firefox_region.js");
const SESSION_SCRIPT: &str = include_str!("firefox_session.js");
const BACKUP: &str = "nodecloak-firefox-region-debug-original.json";
const DEBUG_PREFS: [(&str, bool); 4] = [
    ("devtools.debugger.remote-enabled", true),
    ("devtools.chrome.enabled", true),
    ("devtools.debugger.prompt-connection", false),
    ("devtools.debugger.force-local", true),
];

pub(crate) fn required(p: &Preferences) -> bool {
    p.regional.language_mode == "ip"
        || p.regional.timezone_mode != "system"
        || p.regional.location_mode != "system"
        || p.advanced.viewport_width > 0
}

fn read_backup(profile: &Path) -> Result<Option<Vec<(String, Value)>>, String> {
    let file = profile.join(BACKUP);
    crate::profiles::guard_tree(&file)?;
    if !file.exists() {
        return Ok(None);
    }
    if std::fs::metadata(&file)
        .map_err(|_| "无法读取 Firefox 启动配置备份")?
        .len()
        > 8192
    {
        return Err("Firefox 启动配置备份过大".into());
    }
    let values: Vec<(String, Value)> =
        serde_json::from_slice(&std::fs::read(&file).map_err(|_| "无法读取 Firefox 启动配置备份")?)
            .map_err(|_| "Firefox 启动配置备份无效")?;
    if values.len() != DEBUG_PREFS.len()
        || values
            .iter()
            .zip(DEBUG_PREFS)
            .any(|((key, value), (expected, _))| {
                key != expected
                    || value.as_object().is_none_or(|v| v.len() != 2)
                    || ["user", "runtime"].iter().any(|part| {
                        value
                            .get(part)
                            .is_none_or(|v| !v.is_null() && !v.is_boolean())
                    })
            })
    {
        return Err("Firefox 启动配置备份字段无效，原文件已保留".into());
    }
    Ok(Some(values))
}

/// A previous interrupted setup must not leave debugger preferences enabled.
pub(crate) fn recover(profile: &Path) -> Result<(), String> {
    if let Some(before) = read_backup(profile)? {
        crate::platform::ensure_profile_closed(profile)?;
        let changes = before
            .into_iter()
            .map(|(key, value)| {
                crate::firefox::snapshot(profile, &key).map(|current| (key, current, value))
            })
            .collect::<Result<Vec<_>, _>>()?;
        crate::firefox::apply(profile, &changes, false)?;
        std::fs::remove_file(profile.join(BACKUP)).map_err(|_| "无法清理 Firefox 启动配置备份")?;
    }
    Ok(())
}

pub(crate) fn launch(
    profile: &Path,
    destination: &str,
    p: &Preferences,
    mode: &str,
    relay: Option<u16>,
    lookup_url: &str,
) -> Result<ResolvedRegion, String> {
    launch_impl(profile, destination, p, mode, relay, lookup_url, None)
}
pub(crate) fn launch_resuming(
    profile: &Path,
    destination: &str,
    p: &Preferences,
    mode: &str,
    relay: Option<u16>,
    lookup_url: &str,
    saved: &mut crate::session_restore::FirefoxSession,
) -> Result<ResolvedRegion, String> {
    launch_impl(
        profile,
        destination,
        p,
        mode,
        relay,
        lookup_url,
        Some(saved),
    )
}
fn launch_impl(
    profile: &Path,
    destination: &str,
    p: &Preferences,
    mode: &str,
    relay: Option<u16>,
    lookup_url: &str,
    mut saved: Option<&mut crate::session_restore::FirefoxSession>,
) -> Result<ResolvedRegion, String> {
    crate::platform::ensure_profile_closed(profile)?;
    recover(profile)?;
    if p.regional.location_mode != "system" && p.advanced.location != PermissionMode::Block {
        crate::firefox::reserve_geolocation(profile)?;
    }
    let before = DEBUG_PREFS
        .iter()
        .map(|(key, _)| crate::firefox::snapshot(profile, key).map(|v| (key.to_string(), v)))
        .collect::<Result<Vec<_>, _>>()?;
    if before.iter().any(|(_, v)| {
        ["user", "runtime"]
            .iter()
            .any(|part| !v[part].is_null() && !v[part].is_boolean())
    }) {
        return Err("Firefox 原有调试设置类型无效，原配置已保留".into());
    }
    crate::engine::atomic_write(
        &profile.join(BACKUP),
        &serde_json::to_vec(&before).map_err(|_| "启动配置无效")?,
    )?;
    let changes = before
        .iter()
        .zip(DEBUG_PREFS)
        .map(|((key, old), (_, value))| {
            (
                key.clone(),
                old.clone(),
                json!({"user":value,"runtime":value}),
            )
        })
        .collect::<Vec<_>>();
    crate::firefox::apply(profile, &changes, false)?;
    let fingerprint = p
        .fingerprint
        .as_ref()
        .is_some_and(|f| f.requires_injection());
    let mut deferred = None;
    // A real HTTP gate is required even when the normal startup URL is blank:
    // native session documents are released only after its protection readback.
    let resuming = saved.is_some();
    let spawn = |start_url: &str| {
        let reservation =
            TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配 Firefox 配置端口")?;
        let port = reservation
            .local_addr()
            .map_err(|_| "无法读取 Firefox 配置端口")?
            .port();
        drop(reservation);
        crate::platform::spawn_firefox_region_setup(profile, mode, relay, port)?;
        let mut rdp = Rdp::connect(port)?;
        // Bind the parent target only after browser chrome has built a tab.
        // A target captured during the startup placeholder can refer to a
        // document which is replaced before its queued handoff runs.
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let tabs = rdp.call("root", "listTabs", json!({}))?;
            if tabs["tabs"].as_array().is_some_and(|v| !v.is_empty()) {
                break;
            }
            if Instant::now() >= deadline {
                return Err("Firefox 浏览器窗口尚未完成初始化，请关闭启动提示后重试".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        let descriptor = rdp.call("root", "getProcess", json!({"id":0}))?["processDescriptor"]
            ["actor"]
            .as_str()
            .ok_or("Firefox 未提供原生配置接口")?
            .to_owned();
        let target = rdp.call(&descriptor, "getTarget", json!({}))?;
        let console = target["process"]["consoleActor"]
            .as_str()
            .ok_or("Firefox 未提供原生配置上下文")?
            .to_owned();
        let ip = if crate::regional::uses_ip(p) {
            Some(rdp.lookup(&console, &crate::regional::lookup_urls(lookup_url))?)
        } else {
            None
        };
        let actual = crate::regional::resolve(p, ip)?;
        let geo = actual.latitude.zip(actual.longitude).map(|(lat, lng)| {
            use base64::Engine;
            format!(
                "data:application/json;base64,{}",
                base64::engine::general_purpose::STANDARD.encode(
                    json!({"location":{"lat":lat,"lng":lng},"accuracy":actual.accuracy})
                        .to_string()
                )
            )
        });
        let config = json!({"language":actual.language,"locale":actual.language.split(',').next().unwrap_or("en-US"),
            "timezone":actual.timezone,"geoUri":geo,"width":p.advanced.viewport_width,"height":p.advanced.viewport_height});
        crate::browser_control::log_startup_endpoint(
            profile,
            "firefox",
            "native_region_waiting_for_window",
            "waiting for browser delayed startup and initial URI handling",
        );
        let deadline = Instant::now() + Duration::from_secs(25);
        loop {
            let status = rdp.evaluate(
                &console,
                &if required(p) {
                    format!("{SCRIPT};nodecloakConfigureRegion({config})")
                } else {
                    format!("{SCRIPT};nodecloakRegionWindow()?'configured':'pending'")
                },
            )?;
            if status == "configured" {
                break;
            }
            if status != "pending" || Instant::now() >= deadline {
                return Err("Firefox 浏览器窗口尚未完成初始化，请关闭启动提示后重试".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        crate::browser_control::log_startup_endpoint(
            profile,
            "firefox",
            "native_region_configured",
            "native regional configuration applied after initial URI handling",
        );
        // Firefox writes prefs.js itself while running. Restore only user.js
        // here, and restore runtime values through its own preference service.
        crate::firefox::restore_user_preferences(profile, &changes)?;
        let restore = before
            .iter()
            .map(|(key, value)| {
                (
                    key,
                    if value["user"].is_null() {
                        &value["runtime"]
                    } else {
                        &value["user"]
                    },
                )
            })
            .collect::<Vec<_>>();
        if let Some(session) = saved.as_deref() {
            rdp.evaluate(&console,&format!("{{const win=nodecloakRegionWindow();const sandbox=Cu.Sandbox(win,{{sandboxPrototype:win,wantXrays:false}});Cu.evalInSandbox({},sandbox);win[Symbol.for('nodecloak.saved-session-sandbox')]=sandbox;sandbox.nodecloakPrepareSavedSession({},{},{})}}",json!(SESSION_SCRIPT),json!(session.sources()?),json!(session.receipt()),json!(session.token())))?;
            let deadline = Instant::now() + Duration::from_secs(10);
            loop {
                let status=rdp.evaluate(&console,"nodecloakRegionWindow()[Symbol.for('nodecloak.saved-session-sandbox')].nodecloakSavedSessionStatus()")?;
                if status == "ready" {
                    break;
                }
                if status != "pending" || Instant::now() >= deadline {
                    return Err("无法读取上次 Firefox 会话，原始数据已保留".into());
                }
                std::thread::sleep(Duration::from_millis(50));
            }
            rdp.evaluate(&console,&format!("nodecloakRegionWindow()[Symbol.for('nodecloak.saved-session-sandbox')].nodecloakSessionReadinessPage({},nodecloakRegionWindow())",json!(start_url)))?;
            deferred = Some((rdp, console, restore));
        } else {
            let finish = format!(
                "{SCRIPT};nodecloakFinishRegion({}, {})",
                json!(start_url),
                json!(restore)
            );
            rdp.finish(&console, &finish)?;
        }
        crate::browser_control::log_startup_endpoint(
            profile,
            "firefox",
            "native_region_handoff",
            "temporary debugger closed; browser startup page handoff queued",
        );
        #[cfg(test)]
        eprintln!("Synthetic Firefox native configuration handed off; original debugger preferences restored");
        Ok(actual)
    };
    let result = if resuming {
        crate::firefox_extension::guarded_resume_launch(profile, fingerprint, spawn)
    } else {
        crate::firefox_extension::guarded_launch(profile, destination, fingerprint, spawn)
    };
    let result=result.and_then(|actual|{
        if let Some((mut rdp,console,restore))=deferred.take(){
            // The HTTP gate acknowledged native region and companion settings.
            // Keep this non-WebDriver configuration context alive until native
            // restoration has persisted; close its socket before returning.
            rdp.evaluate(&console,"nodecloakRegionWindow()[Symbol.for('nodecloak.saved-session-sandbox')].nodecloakRestoreSavedSession()")?;
            let session=saved.as_deref_mut().ok_or("缺少会话恢复快照")?;
            let deadline=Instant::now()+Duration::from_secs(15);
            while !session.commit()? {
                if Instant::now()>=deadline{return Err("Firefox 会话恢复未完成，原始会话快照已保留".into());}
                std::thread::sleep(Duration::from_millis(50));
            }
            rdp.finish(&console,&format!("{SCRIPT};nodecloakFinishRegion('about:blank',{},true)",json!(restore)))?;
        }
        Ok(actual)
    });
    if let Err(error) = &result {
        // This process belongs to this new setup attempt; no destination has
        // been released on a failed lookup/configuration. Do not leave a local
        // debugger listener alive when graceful close is unavailable or blocked.
        if crate::platform::close_profile(profile)
            .or_else(|_| crate::platform::force_close_profile(profile))
            .is_err()
        {
            return Err(format!(
                "{error}；Firefox 初始化窗口仍在运行，请关闭该副本后重试"
            ));
        }
        recover(profile)?;
        crate::firefox::prepare_geolocation(profile, None)?;
    } else {
        // Runtime originals have been restored by the browser and user.js is
        // back to its original values. Retain backup until the next closed
        // launch to also recover a process crash before prefs.js was flushed.
        crate::browser_control::log_startup_endpoint(
            profile,
            "firefox",
            "native_region_ready",
            "native regional configuration ready; temporary debugger listener closed",
        );
    }
    result
}

struct Rdp {
    socket: TcpStream,
}
impl Rdp {
    fn connect(port: u16) -> Result<Self, String> {
        let deadline = Instant::now() + Duration::from_secs(25);
        let socket = loop {
            if let Ok(s) = TcpStream::connect_timeout(
                &([127, 0, 0, 1], port).into(),
                Duration::from_millis(150),
            ) {
                break s;
            }
            if Instant::now() >= deadline {
                return Err("Firefox 原生配置接口初始化超时，请更新 Firefox 后重试".into());
            }
            std::thread::sleep(Duration::from_millis(50));
        };
        socket
            .set_read_timeout(Some(Duration::from_secs(15)))
            .map_err(|_| "无法配置 Firefox 连接")?;
        socket
            .set_write_timeout(Some(Duration::from_secs(5)))
            .map_err(|_| "无法配置 Firefox 连接")?;
        let mut this = Self { socket };
        let greeting = packet(&mut this.socket)?;
        if greeting["from"] != "root" || greeting["applicationType"] != "browser" {
            return Err("Firefox 原生配置协议无效".into());
        }
        Ok(this)
    }
    fn send(&mut self, actor: &str, method: &str, mut args: Value) -> Result<(), String> {
        if actor.is_empty() || actor.len() > 512 {
            return Err("Firefox 配置标识无效".into());
        }
        args["to"] = json!(actor);
        args["type"] = json!(method);
        let bytes = serde_json::to_vec(&args).map_err(|_| "Firefox 配置请求无效")?;
        self.socket
            .write_all(format!("{}:", bytes.len()).as_bytes())
            .and_then(|_| self.socket.write_all(&bytes))
            .map_err(|_| "Firefox 原生配置连接中断".into())
    }
    fn call(&mut self, actor: &str, method: &str, args: Value) -> Result<Value, String> {
        self.send(actor, method, args)?;
        for _ in 0..256 {
            let value = packet(&mut self.socket)?;
            if value["from"] == actor {
                if value.get("error").is_some() {
                    return Err("Firefox 拒绝原生配置请求，请更新浏览器后重试".into());
                }
                // Notifications share the actor with replies. In particular,
                // startup tabListChanged must not consume listTabs/getProcess
                // replies and shift every subsequent request by one packet.
                let reply = match method {
                    "listTabs" => value["tabs"].is_array(),
                    "getProcess" => value["processDescriptor"].is_object(),
                    "getTarget" => value["process"].is_object(),
                    "evaluateJSAsync" => value["resultID"].is_string() && value.get("type").is_none(),
                    _ => value.get("type").is_none(),
                };
                if reply { return Ok(value); }
            }
        }
        Err("Firefox 原生配置响应过多".into())
    }
    fn evaluate(&mut self, actor: &str, script: &str) -> Result<Value, String> {
        let receipt = self.call(actor, "evaluateJSAsync", json!({"text":script}))?;
        let id = receipt["resultID"].as_str().ok_or("Firefox 配置响应无效")?;
        for _ in 0..256 {
            let value = packet(&mut self.socket)?;
            if value["from"] == actor
                && value["resultID"] == id
                && value["type"] == "evaluationResult"
            {
                if value.get("exception").is_some_and(|v| !v.is_null())
                    || value["hasException"] == true
                {
                    #[cfg(test)]
                    eprintln!(
                        "Synthetic Firefox native configuration fixture: {}",
                        value["exceptionMessage"]
                    );
                    return Err("Firefox 无法应用原生区域设置，请更新 Firefox 后重试".into());
                }
                return Ok(value["result"].clone());
            }
        }
        Err("Firefox 配置结果缺失".into())
    }
    fn lookup(
        &mut self,
        actor: &str,
        urls: &[String],
    ) -> Result<crate::regional::IpRegion, String> {
        let key = format!("nodecloak.region.{}", crate::fingerprint::seed()?);
        self.evaluate(
            actor,
            &format!(
                "{SCRIPT};nodecloakQueryRegion({}, {})",
                json!(urls),
                json!(key)
            ),
        )?;
        let deadline = Instant::now() + Duration::from_secs(15);
        loop {
            let response = self.evaluate(
                actor,
                &format!("JSON.stringify(globalThis[Symbol.for({})])", json!(key)),
            )?;
            let state: Value = serde_json::from_str(response.as_str().ok_or("区域查询响应无效")?)
                .map_err(|_| "区域查询响应无效")?;
            if state["status"] == "ready" {
                let body = state["body"]
                    .as_str()
                    .filter(|v| v.len() <= 900)
                    .ok_or("区域查询响应无效")?;
                let region = crate::regional::parse_ip_region(
                    &serde_json::from_str(body).map_err(|_| "区域查询响应无效")?,
                );
                self.evaluate(
                    actor,
                    &format!("delete globalThis[Symbol.for({})]", json!(key)),
                )?;
                return region;
            }
            if state["status"] == "failed" || Instant::now() >= deadline {
                let detail = state["error"]
                    .as_str()
                    .unwrap_or("")
                    .chars()
                    .take(240)
                    .collect::<String>();
                return Err(if detail.is_empty() {
                    "无法通过 Firefox 的网络路径匹配 IP 区域，请检查代理后重试；尚未打开目标网站"
                        .into()
                } else {
                    format!("无法通过 Firefox 的网络路径匹配 IP 区域（{detail}），请检查代理后重试；尚未打开目标网站")
                });
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }
    fn finish(&mut self, actor: &str, script: &str) -> Result<(), String> {
        self.send(actor, "evaluateJSAsync", json!({"text":script}))?;
        // Closing the listener also retires its accepted connections. EOF is
        // expected only for this final fixed command, never other requests.
        for _ in 0..256 {
            match packet(&mut self.socket) {
                Err(error) if error == "FIREFOX_RDP_EOF" => return Ok(()),
                Err(error) => return Err(error),
                Ok(v) if v["hasException"] == true || v.get("error").is_some() => {
                    return Err("Firefox 无法完成原生配置或关闭临时接口".into())
                }
                Ok(_) => {}
            }
        }
        Err("Firefox 临时配置接口未关闭".into())
    }
}

fn packet(input: &mut impl Read) -> Result<Value, String> {
    let mut header = Vec::new();
    loop {
        let mut byte = [0];
        input.read_exact(&mut byte).map_err(|e| {
            if e.kind() == std::io::ErrorKind::UnexpectedEof {
                "FIREFOX_RDP_EOF".to_owned()
            } else {
                "Firefox 原生配置响应超时或连接中断".to_owned()
            }
        })?;
        if byte[0] == b':' {
            break;
        }
        if !byte[0].is_ascii_digit() || header.len() >= 8 {
            return Err("Firefox 原生配置响应格式无效".into());
        }
        header.push(byte[0]);
    }
    let size = std::str::from_utf8(&header)
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .filter(|n| *n > 0 && *n <= 1024 * 1024)
        .ok_or("Firefox 原生配置响应过大")?;
    let mut bytes = vec![0; size];
    input
        .read_exact(&mut bytes)
        .map_err(|_| "Firefox 原生配置响应不完整")?;
    serde_json::from_slice(&bytes).map_err(|_| "Firefox 原生配置响应无效".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn malformed_or_oversized_packets_fail_without_unbounded_allocation() {
        for data in [b"99999999:".as_slice(), b"0:", b"abc:", b"10:{}", b"2:[]x"] {
            let result = packet(&mut std::io::Cursor::new(data));
            if data == b"2:[]x" {
                assert_eq!(result.unwrap(), json!([]));
            } else {
                assert!(result.is_err());
            }
        }
        assert_eq!(
            packet(&mut std::io::Cursor::new(b"2:{}")).unwrap(),
            json!({})
        );
    }
    #[test]
    fn native_replies_ignore_same_actor_window_and_console_notifications() {
        fn send(socket:&mut TcpStream,value:Value) {
            let bytes=serde_json::to_vec(&value).unwrap();
            socket.write_all(format!("{}:",bytes.len()).as_bytes()).unwrap();
            socket.write_all(&bytes).unwrap();
        }
        let listener=TcpListener::bind("127.0.0.1:0").unwrap();
        let socket=TcpStream::connect(listener.local_addr().unwrap()).unwrap();
        socket.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
        let server=std::thread::spawn(move||{
            let (mut peer,_)=listener.accept().unwrap();
            for (method,actor,event,reply) in [
                ("listTabs","root",json!({"type":"tabListChanged"}),json!({"tabs":[{}]})),
                ("getProcess","root",json!({"type":"processListChanged"}),json!({"processDescriptor":{"actor":"parent"}})),
                ("getTarget","parent",json!({"type":"resources-available-array","array":[]}),json!({"process":{"consoleActor":"console"}})),
                ("evaluateJSAsync","console",json!({"type":"consoleAPICall"}),json!({"resultID":"result-1"})),
            ] {
                let request=packet(&mut peer).unwrap();assert_eq!(request["type"],method);
                let mut event=event;event["from"]=json!(actor);send(&mut peer,event);
                let mut reply=reply;reply["from"]=json!(actor);send(&mut peer,reply);
                if method=="evaluateJSAsync" {
                    send(&mut peer,json!({"from":actor,"type":"evaluationResult","resultID":"result-1","result":{"value":1}}));
                }
            }
        });
        let mut rdp=Rdp{socket};
        let result=(||->Result<(),String>{
            assert!(rdp.call("root","listTabs",json!({}))?["tabs"].is_array());
            assert_eq!(rdp.call("root","getProcess",json!({"id":0}))?["processDescriptor"]["actor"],"parent");
            assert_eq!(rdp.call("parent","getTarget",json!({}))?["process"]["consoleActor"],"console");
            assert_eq!(rdp.evaluate("console","1")?["value"],1);
            Ok(())
        })();
        drop(rdp);server.join().unwrap();result.unwrap();
    }
    #[test]
    fn interrupted_setup_restores_only_recorded_preferences() {
        let root = tempfile::tempdir().unwrap();
        let canonical = root.path().canonicalize().unwrap();
        let p = canonical.as_path();
        let before = DEBUG_PREFS
            .iter()
            .map(|(k, _)| (k.to_string(), json!({"user":null,"runtime":null})))
            .collect::<Vec<_>>();
        crate::engine::atomic_write(&p.join(BACKUP), &serde_json::to_vec(&before).unwrap())
            .unwrap();
        let changes = before
            .iter()
            .zip(DEBUG_PREFS)
            .map(|((k, b), (_, v))| (k.clone(), b.clone(), json!({"user":v,"runtime":v})))
            .collect::<Vec<_>>();
        crate::firefox::apply(p, &changes, false).unwrap();
        recover(p).unwrap();
        assert!(!p.join(BACKUP).exists());
        for (k, _) in DEBUG_PREFS {
            assert_eq!(
                crate::firefox::snapshot(p, k).unwrap(),
                json!({"user":null,"runtime":null})
            );
        }
        std::fs::write(p.join(BACKUP), br#"[["unrelated",{}]]"#).unwrap();
        assert!(recover(p).is_err());
        assert!(p.join(BACKUP).exists());
    }
}
