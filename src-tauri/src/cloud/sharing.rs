//! Individually encrypted, revocable profile grants. Independent of personal sync.
use super::{
    account::Connection,
    browser_session::Snapshot,
    crypto,
    model::{Disk, Document},
    vault,
};
use crate::{data_root, engine, platform, profiles, OperationLock};
use reqwest::Method;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex, OnceLock},
    time::{Duration, Instant},
};
use tauri::{Emitter, Manager};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Summary {
    pub id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub team_id: Option<String>,
    pub source_profile_id: String,
    pub profile_name: String,
    pub browser: String,
    pub sender_id: String,
    pub sender_name: String,
    pub sender_email: String,
    pub recipient_id: String,
    pub recipient_name: String,
    pub recipient_email: String,
    pub includes_session: bool,
    pub note: String,
    pub status: String,
    pub created_at: String,
    pub expires_at: String,
    pub accepted_at: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub device_access: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct View {
    pub id: String,
    pub sender_name: String,
    pub expires_at: String,
    pub status: String,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Binding {
    account: String,
    local_id: String,
    share: Summary,
    ready: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Store {
    bindings: BTreeMap<String, Binding>,
}
impl Store {
    fn load(root: &Path) -> Result<Self, String> {
        let file = root.join("cloud/sharing.json");
        profiles::guard_tree(&file)?;
        match std::fs::read(&file) {
            Ok(bytes) => {
                if bytes.len() > 8 << 20 {
                    return Err("共享授权记录过大".into());
                }
                let s: Self = serde_json::from_slice(&bytes).map_err(|_| "共享授权记录损坏")?;
                for (id, b) in &s.bindings {
                    if id != &b.local_id
                        || uuid::Uuid::parse_str(id).is_err()
                        || uuid::Uuid::parse_str(&b.account).is_err()
                        || uuid::Uuid::parse_str(&b.share.id).is_err()
                        || b.account != b.share.recipient_id
                    {
                        return Err("共享授权绑定无效".into());
                    }
                    timestamp(&b.share.expires_at)?;
                }
                Ok(s)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("无法读取共享授权记录".into()),
        }
    }
    fn save(&self, root: &Path) -> Result<(), String> {
        let file = root.join("cloud/sharing.json");
        profiles::guard_tree(&file)?;
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|_| "无法创建共享授权目录")?;
        engine::atomic_write(
            &file,
            &serde_json::to_vec(self).map_err(|_| "无法保存共享授权")?,
        )
    }
}
pub fn restricted(root: &Path, id: &str) -> Result<bool, String> {
    Ok(Store::load(root)?.bindings.contains_key(id)
        || profiles::load(root)?
            .iter()
            .any(|p| p.id == id && p.received_share.is_some()))
}
pub fn editable(root: &Path, id: &str) -> Result<(), String> {
    if restricted(root, id)? {
        Err("收到的共享副本不能修改、复制、转分享或导出".into())
    } else {
        Ok(())
    }
}
pub fn visible(root: &Path, id: &str) -> Result<bool, String> {
    let s = Store::load(root)?;
    let Some(b) = s.bindings.get(id) else {
        return Ok(!profiles::load(root)?
            .iter()
            .any(|p| p.id == id && p.received_share.is_some()));
    };
    Ok(b.ready
        && Disk::load(root)?
            .account
            .as_ref()
            .is_some_and(|a| a.id == b.account))
}
pub fn view(root: &Path, id: &str) -> Result<Option<View>, String> {
    Ok(Store::load(root)?.bindings.get(id).map(|b| View {
        id: b.share.id.clone(),
        sender_name: b.share.sender_name.clone(),
        expires_at: b.share.expires_at.clone(),
        status: b.share.status.clone(),
    }))
}
fn timestamp(s: &str) -> Result<i64, String> {
    chrono::DateTime::parse_from_rfc3339(s)
        .map(|d| d.timestamp())
        .map_err(|_| "共享有效期无效".into())
}
fn summary(value: &Value) -> Result<Summary, String> {
    let s: Summary = serde_json::from_value(value.clone()).map_err(|_| "共享响应格式无效")?;
    for id in [&s.id, &s.sender_id, &s.recipient_id] {
        uuid::Uuid::parse_str(id).map_err(|_| "共享标识无效")?;
    }
    if let Some(team_id) = &s.team_id {
        uuid::Uuid::parse_str(team_id).map_err(|_| "团队标识无效")?;
        if s.includes_session {
            return Err("团队副本只允许分配浏览器配置".into());
        }
    }
    timestamp(&s.expires_at)?;
    if ![
        "pending", "accepted", "declined", "revoked", "left", "expired",
    ]
    .contains(&s.status.as_str())
    {
        return Err("共享状态无效".into());
    }
    Ok(s)
}
fn context<'a>(sender: &'a str, id: &'a str, writer: &'a str) -> crypto::Context<'a> {
    crypto::Context {
        account: sender,
        object: id,
        kind: "share",
        revision: 1,
        epoch: 1,
        deleted: false,
        generation: 0,
        writer,
    }
}
fn envelope_scope(s: &Summary) -> String {
    format!("share:{}:{}:{}", s.sender_id, s.id, s.recipient_id)
}
#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct Payload {
    schema: u8,
    id: String,
    sender_id: String,
    recipient_id: String,
    source_profile_id: String,
    includes_session: bool,
    document: Document,
    session: Option<Snapshot>,
}

fn team_document(mut profile: profiles::Profile) -> Result<Document, String> {
    profile.proxy = crate::proxy::ProxyConfig::system();
    profile.notes.clear();
    Document::export(&profile)
}

impl Payload {
    fn parse(bytes: &[u8], s: &Summary) -> Result<Self, String> {
        let p: Self = serde_json::from_slice(bytes).map_err(|_| "共享快照格式无效")?;
        if p.schema != 1
            || p.id != s.id
            || p.sender_id != s.sender_id
            || p.recipient_id != s.recipient_id
            || p.source_profile_id != s.source_profile_id
            || p.includes_session != s.includes_session
            || p.session.is_some() != s.includes_session
            || p.document.browser != s.browser
            || p.document.name != s.profile_name
            || p.document.deleted
        {
            return Err("共享快照身份校验失败".into());
        }
        Document::parse(serde_json::to_value(&p.document).map_err(|_| "共享配置无效")?)?;
        if let Some(session) = &p.session {
            Snapshot::parse(serde_json::to_value(session).map_err(|_| "共享登录状态无效")?)?;
        }
        Ok(p)
    }
}
#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    List,
    Create {
        profile_id: String,
        name: String,
        email: String,
        expires_at: String,
        includes_session: bool,
        note: String,
        #[serde(default)]
        team: Option<TeamAssignment>,
    },
    Accept {
        id: String,
        #[serde(default)]
        authorize_device: bool,
    },
    Decline {
        id: String,
    },
    Revoke {
        id: String,
    },
    Leave {
        id: String,
    },
    Cleanup {
        id: String,
    },
    Extend {
        id: String,
        expires_at: String,
    },
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct TeamAssignment {
    account_id: uuid::Uuid,
    team_id: uuid::Uuid,
    member_id: uuid::Uuid,
}
type Deadlines = Arc<Mutex<BTreeMap<String, (Instant, String)>>>;
fn deadlines() -> Deadlines {
    static DEADLINES: OnceLock<Deadlines> = OnceLock::new();
    DEADLINES
        .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
        .clone()
}
fn checks() -> Arc<Mutex<BTreeMap<String, Instant>>> {
    static CHECKS: OnceLock<Arc<Mutex<BTreeMap<String, Instant>>>> = OnceLock::new();
    CHECKS
        .get_or_init(|| Arc::new(Mutex::new(BTreeMap::new())))
        .clone()
}
pub struct Runtime {
    deadlines: Deadlines,
}
impl Default for Runtime {
    fn default() -> Self {
        Self {
            deadlines: deadlines(),
        }
    }
}
fn authorization(c: &mut Connection, s: &Summary) -> Result<(Summary, Duration), String> {
    if c.account_id()? != s.recipient_id {
        return Err("共享副本属于其他账号".into());
    }
    let result = c.request(
        Method::POST,
        &format!("/v1/shares/{}/authorize", s.id),
        Some(&json!({})),
    )?;
    let next = summary(&result["share"])?;
    let now = timestamp(result["serverTime"].as_str().ok_or("授权服务器时间无效")?)?;
    if next.id != s.id
        || next.recipient_id != s.recipient_id
        || next.sender_id != s.sender_id
        || next.status != "accepted"
        || timestamp(&next.expires_at)? <= now
    {
        return Err("共享已过期或被撤销".into());
    }
    Ok((
        next.clone(),
        Duration::from_secs((timestamp(&next.expires_at)? - now) as u64),
    ))
}
fn reserve_import(
    root: &Path,
    account: &str,
    s: &Summary,
    payload: &Payload,
) -> Result<String, String> {
    let mut store = Store::load(root)?;
    let existing = store
        .bindings
        .values()
        .find(|b| b.account == account && b.share.id == s.id)
        .cloned();
    let b = existing.unwrap_or(Binding {
        account: account.into(),
        local_id: uuid::Uuid::new_v4().simple().to_string(),
        share: s.clone(),
        ready: false,
    });
    if ["revoked", "left", "expired", "declined"].contains(&b.share.status.as_str()) {
        return Err("共享授权已结束".into());
    }
    let local = b.local_id.clone();
    store.bindings.insert(local.clone(), b.clone());
    store.save(root)?;
    if !b.ready {
        let p = if profiles::load(root)?.iter().any(|p| p.id == local) {
            profiles::get(root, &local, false)?
        } else {
            profiles::create_synced(root, payload.document.draft(), &local)?
        };
        profiles::mark_shared(root, &local, &s.id)?;
        if let Some(session) = &payload.session {
            super::browser_session::restore_profile(
                &p.browser,
                &profiles::path(root, &p)?,
                session,
            )?;
        }
    }
    Ok(local)
}
fn expose_import(root: &Path, local: &str, s: Summary) -> Result<(), String> {
    let mut store = Store::load(root)?;
    let b = store.bindings.get_mut(local).ok_or("共享绑定不存在")?;
    if ["revoked", "left", "expired", "declined"].contains(&b.share.status.as_str()) {
        return Err("共享授权已结束".into());
    }
    b.ready = true;
    b.share = s;
    store.save(root)
}
pub fn before_launch(root: &Path, id: &str) -> Result<(), String> {
    let mut store = Store::load(root)?;
    let Some(binding) = store.bindings.get_mut(id) else {
        return if profiles::load(root)?
            .iter()
            .any(|p| p.id == id && p.received_share.is_some())
        {
            Err("共享授权绑定不存在".into())
        } else {
            Ok(())
        };
    };
    if !binding.ready
        || ["revoked", "left", "expired", "declined"].contains(&binding.share.status.as_str())
    {
        return Err("共享副本尚未就绪或授权已结束".into());
    }
    let mut c = Connection::load(root)?;
    let (s, remaining) = authorization(&mut c, &binding.share)?;
    deadlines().lock().map_err(|_| "共享授权锁不可用")?.insert(
        id.into(),
        (Instant::now() + remaining, s.expires_at.clone()),
    );
    checks()
        .lock()
        .map_err(|_| "共享授权锁不可用")?
        .insert(id.into(), Instant::now() + Duration::from_secs(60));
    binding.share = s;
    store.save(root)
}
#[tauri::command]
pub async fn sharing_action(app: tauri::AppHandle, action: Action) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move||{
        let mutation=!matches!(&action,Action::List);
        let root=&app.state::<data_root::DataRoot>().0;
        let mut c=Connection::load(root)?; let account=c.account_id()?;
        let result=match action {
            Action::List=>{
                let mut v=c.request(Method::GET,"/v1/shares",None)?;
                let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;
                let mut store=Store::load(root)?;
                if let Some(items)=v["received"].as_array_mut(){for item in items {
                    let s=summary(item)?;
                    if let Some(b)=store.bindings.values_mut().find(|b|b.account==account && b.share.id==s.id){
                        if !["revoked","left","expired","declined"].contains(&b.share.status.as_str()){b.share=s;}
                        item["localDataPresent"]=json!(profiles::load(root)?.iter().any(|p|p.id==b.local_id));
                        if b.ready {item["localProfileId"]=json!(b.local_id);}
                    }
                }}store.save(root)?; v
            }
            Action::Create{profile_id,name,email,expires_at,includes_session,note,team}=>{
                if let Some(target) = &team {
                    if target.account_id.to_string() != account || includes_session || !note.is_empty() {
                        return Err("团队副本只允许分配浏览器配置".into());
                    }
                }
                if timestamp(&expires_at)?<=chrono::Utc::now().timestamp(){return Err("请选择未来的共享截止时间".into())}
                let recipient=c.request(Method::POST,"/v1/shares/recipient",Some(&json!({"name":name,"email":email})))?;
                let recipient_id=recipient["account"]["id"].as_str().ok_or("接收账号无效")?;
                let devices=recipient["devices"].as_array().ok_or("接收设备无效")?;
                if devices.is_empty(){return Err("对方需要先登录 NodeCloak，再重新发送邀请".into())}
                let id=uuid::Uuid::new_v4().to_string();
                let payload={let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;
                    editable(root,&profile_id)?;let p=profiles::get(root,&profile_id,false)?;
                    let session=if includes_session {let path=profiles::path(root,&p)?;platform::ensure_profile_closed(&path)?;Some(super::browser_session::export_profile(&p.browser,&path)?)}else{None};
                    let document=if team.is_some(){team_document(p)?}else{Document::export(&p)?};
                    Payload{schema:1,id:id.clone(),sender_id:account.clone(),recipient_id:recipient_id.into(),source_profile_id:profile_id.clone(),includes_session,document,session}
                };
                let key=crypto::random_key()?;
                let encrypted=crypto::seal(&key,&context(&account,&id,&c.device.id),&zeroize::Zeroizing::new(serde_json::to_vec(&payload).map_err(|_|"共享快照无效")?))?;
                let scope=format!("share:{account}:{id}:{recipient_id}");let mut envelopes=vec![];
                for device in devices {let device_id=device["id"].as_str().ok_or("接收设备无效")?;let public=device["publicKey"].as_str().ok_or("接收设备密钥无效")?;
                    envelopes.push(json!({"deviceId":device_id,"publicKey":public,"envelope":crypto::device_seal(&*vault::decode_key(public)?,&scope,device_id,1,&key)?}));
                }
                let mut request=json!({"id":id,"sourceProfileId":profile_id,"profileName":payload.document.name,"browser":payload.document.browser,"recipientId":recipient_id,"recipientName":name,"recipientEmail":email,"expiresAt":expires_at,"includesSession":includes_session,"note":note,"payload":serde_json::to_string(&encrypted).map_err(|_|"共享加密无效")?,"writer":c.device.id,"envelopes":envelopes});
                if let Some(target) = team {
                    request["teamId"] = json!(target.team_id);
                    request["teamMemberId"] = json!(target.member_id);
                }
                c.request(Method::POST,"/v1/shares",Some(&request))?["share"].clone()
            }
            Action::Accept{id,authorize_device}=>{
                uuid::Uuid::parse_str(&id).map_err(|_|"共享标识无效")?;
                let response=c.request(Method::POST,&format!("/v1/shares/{id}/accept"),Some(&json!({"authorizeDevice":authorize_device})))?;
                let s=summary(&response["share"])?;let (s,_)=authorization(&mut c,&s)?;
                let remote=c.request(Method::GET,&format!("/v1/shares/{id}/payload"),None)?;
                let returned=summary(&remote["share"])?;
                if returned.id!=s.id || returned.sender_id!=s.sender_id || returned.recipient_id!=s.recipient_id {return Err("共享身份不匹配".into())}
                let key=crypto::device_open(&*vault::decode_key(&c.device.private_key)?,&envelope_scope(&s),&c.device.id,1,remote["envelope"].as_str().ok_or("共享设备未授权")?)?;
                let encrypted:crypto::Envelope=serde_json::from_str(remote["payload"].as_str().ok_or("共享快照不存在")?).map_err(|_|"共享加密格式无效")?;
                let bytes=crypto::open(&key,&context(&s.sender_id,&s.id,remote["writer"].as_str().ok_or("共享来源无效")?),&encrypted)?;
                let payload=Payload::parse(&bytes,&s)?;
                let local={let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;
                    reserve_import(root,&account,&s,&payload)?
                };
                let (current,remaining)=authorization(&mut c,&s)?;
                {let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;expose_import(root,&local,current.clone())?;}
                app.state::<Runtime>().deadlines.lock().map_err(|_|"共享授权锁不可用")?.insert(local.clone(),(Instant::now()+remaining,current.expires_at.clone()));
                let mut out=json!(current);out["localProfileId"]=json!(local);out
            }
            Action::Cleanup{id}=>{
                let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;
                let mut store=Store::load(root)?;let locals=store.bindings.iter().filter(|(_,b)|b.account==account && b.share.id==id).map(|(local,_)|local.clone()).collect::<Vec<_>>();
                let result=locals.first().and_then(|local|store.bindings.get(local)).map(|b|b.share.clone()).ok_or("本机没有此共享副本数据")?;
                for local in &locals {let b=store.bindings.get_mut(local).unwrap();b.ready=false;}
                store.save(root)?;
                for local in &locals {close(root,local,&app.state::<profiles::Runtime>())?;if profiles::load(root)?.iter().any(|p|&p.id==local){profiles::remove(root,local,false,&app.state::<profiles::Runtime>())?;profiles::remove(root,local,true,&app.state::<profiles::Runtime>())?;}}
                let mut out=json!(result);out["localDataPresent"]=json!(false);out
            }
            other=>{
                let (id,method,suffix,body)=match other {Action::Decline{id}=>(id,Method::POST,"/decline",json!({})),Action::Revoke{id}=>(id,Method::POST,"/revoke",json!({})),Action::Leave{id}=>(id,Method::POST,"/leave",json!({})),Action::Extend{id,expires_at}=>(id,Method::PATCH,"",json!({"expiresAt":expires_at})),_=>unreachable!()};
                uuid::Uuid::parse_str(&id).map_err(|_|"共享标识无效")?;
                let out=c.request(method,&format!("/v1/shares/{id}{suffix}"),Some(&body))?;
                let s=summary(&out["share"])?;
                {let guard=app.state::<OperationLock>();let _lock=guard.0.lock().map_err(|_|"操作锁不可用")?;let mut store=Store::load(root)?;for b in store.bindings.values_mut().filter(|b|b.account==account && b.share.id==id){b.share=s.clone();}store.save(root)?;
                    if s.status!="accepted" {for b in store.bindings.values().filter(|b|b.account==account && b.share.id==id){close(root,&b.local_id,&app.state::<profiles::Runtime>())?;}}
                    if s.status=="left" {for b in store.bindings.values_mut().filter(|b|b.account==account && b.share.id==id){b.ready=false;if profiles::load(root)?.iter().any(|p|p.id==b.local_id){profiles::remove(root,&b.local_id,false,&app.state::<profiles::Runtime>())?;profiles::remove(root,&b.local_id,true,&app.state::<profiles::Runtime>())?;}}store.save(root)?;}
                }
                out["share"].clone()
            }
        };
        if mutation {let _=app.emit("sharing-changed",());let _=app.emit("profiles-changed",());}Ok(result)
    }).await.map_err(|_|"共享操作未完成")?
}
fn close(root: &Path, id: &str, runtime: &profiles::Runtime) -> Result<(), String> {
    let result = profiles::load_snapshot(root).and_then(|all| match all.iter().find(|p| p.id == id) {
        Some(p) => profiles::path(root, p).and_then(|path| platform::force_close_profile(&path)),
        None => Ok(()),
    });
    runtime.1.stop(id);
    if let Ok(mut r) = runtime.0.lock() {
        r.remove(id);
    }
    result
}
fn close_async(app: &tauri::AppHandle, id: &str, reason: &str) {
    static CLOSING: OnceLock<Arc<Mutex<std::collections::BTreeSet<String>>>> = OnceLock::new();
    let closing = CLOSING
        .get_or_init(|| Arc::new(Mutex::new(std::collections::BTreeSet::new())))
        .clone();
    if !closing.lock().is_ok_and(|mut c| c.insert(id.into())) {
        return;
    }
    let app = app.clone();
    let id = id.to_string();
    let reason = reason.to_string();
    std::thread::spawn(move || {
        let root = &app.state::<data_root::DataRoot>().0;
        if close(root, &id, &app.state::<profiles::Runtime>()).is_ok() {
            let grant = view(root, &id).ok().flatten().map(|v| v.id);
            let _ = app.emit(
                "sharing-changed",
                json!({"id":grant,"localProfileId":id,"reason":reason}),
            );
            let _ = app.emit("profiles-changed", ());
        }
        if let Ok(mut c) = closing.lock() {
            c.remove(&id);
        }
    });
}
fn allowed_running(
    binding: &Binding,
    account: Option<&str>,
    deadline: Option<Instant>,
    lease: Option<Instant>,
    now: Instant,
) -> bool {
    binding.ready
        && account == Some(binding.account.as_str())
        && binding.share.status == "accepted"
        && deadline.is_some_and(|d| now < d)
        && lease.is_some_and(|d| now < d)
}

/// Local deadlines/account changes are checked independently of network/operation locks.
pub fn start(app: tauri::AppHandle) {
    std::thread::Builder::new().name("nodecloak-share-deadlines".into()).spawn(move||{
        let mut checking=std::collections::BTreeSet::new();
        let (tx,rx)=std::sync::mpsc::channel::<(String,Result<(Summary,Duration),String>)>();
        let mut last_check=BTreeMap::<String,Instant>::new();
        let mut warnings=BTreeMap::<String,String>::new();
        loop {
            let root=&app.state::<data_root::DataRoot>().0;
            while let Ok((id,result))=rx.try_recv(){
                checking.remove(&id);
                match result {
                    Ok((s,remaining))=>{
                        if let Ok(mut lease)=checks().lock(){lease.insert(id.clone(),Instant::now()+Duration::from_secs(60));}
                        if let Ok(mut deadlines)=app.state::<Runtime>().deadlines.lock(){deadlines.insert(id.clone(),(Instant::now()+remaining,s.expires_at.clone()));}
                        // Persistence is best effort here; launch authorization always rechecks.
                        let guard=app.state::<OperationLock>();
                        if let Ok(_lock)=guard.0.try_lock(){if let Ok(mut store)=Store::load(root){if let Some(b)=store.bindings.get_mut(&id){if !["revoked","left","expired","declined"].contains(&b.share.status.as_str()){b.share=s;let _=store.save(root);}}}};
                    }
                    Err(_)=>{close_async(&app,&id,"authorizationFailed");if let Ok(mut deadlines)=app.state::<Runtime>().deadlines.lock(){deadlines.remove(&id);}}
                }
            }
            let account=Disk::load(root).ok().and_then(|d|d.account).map(|a|a.id);
            let store=match Store::load(root){Ok(s)=>s,Err(_)=>{if let Ok(remembered)=deadlines().lock(){for id in remembered.keys(){close_async(&app,id,"authorizationFailed");}}if let Ok(all)=profiles::load_snapshot(root){for p in all.iter().filter(|p|p.received_share.is_some()){close_async(&app,&p.id,"authorizationFailed");}}std::thread::sleep(Duration::from_secs(1));continue}};
            let all=profiles::load_snapshot(root).unwrap_or_default();
            let processes=platform::browser_processes();
            let running=all.iter().filter_map(|p|profiles::path(root,p).ok().filter(|path|!platform::profile_ids_from(&processes,path).is_empty()).map(|_|p.id.clone())).collect::<std::collections::BTreeSet<_>>();
            for p in all.iter().filter(|p|p.received_share.is_some() && !store.bindings.contains_key(&p.id)){if running.contains(&p.id){close_async(&app,&p.id,"authorizationFailed");}}
            for (id,b) in &store.bindings {
                if !running.contains(id) {continue}
                let bound=app.state::<Runtime>();
                let deadline=bound.deadlines.lock().ok().and_then(|d|d.get(id).cloned());
                let lease=checks().lock().ok().and_then(|c|c.get(id).copied());
                let invalid=!allowed_running(b,account.as_deref(),deadline.as_ref().map(|(d,_)|*d),lease,Instant::now());
                if invalid {close_async(&app,id,"authorizationEnded");continue}
                if let Some((until,expiry))=&deadline {if until.saturating_duration_since(Instant::now())<=Duration::from_secs(300) && warnings.get(id)!=Some(expiry){warnings.insert(id.clone(),expiry.clone());let _=app.emit("sharing-changed",json!({"id":b.share.id,"localProfileId":id,"reason":"expiring","expiresAt":expiry}));}}
                if !checking.contains(id) && last_check.get(id).is_none_or(|t|t.elapsed()>=Duration::from_secs(20)) {
                    checking.insert(id.clone());last_check.insert(id.clone(),Instant::now());
                    let root=root.clone();let s=b.share.clone();let id=id.clone();let tx=tx.clone();
                    std::thread::spawn(move||{let result=Connection::load(&root).and_then(|mut c|authorization(&mut c,&s));let _=tx.send((id,result));});
                }
            }
            std::thread::sleep(Duration::from_secs(1));
        }
    }).expect("sharing authorization watchdog");
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn accept_consent_is_explicit_and_defaults_to_false() {
        let explicit: Action =
            serde_json::from_value(json!({"type":"accept","id":"grant","authorizeDevice":true}))
                .unwrap();
        match explicit {
            Action::Accept {
                authorize_device, ..
            } => assert!(authorize_device),
            _ => panic!("accept action"),
        }
        let implicit: Action =
            serde_json::from_value(json!({"type":"accept","id":"grant"})).unwrap();
        match implicit {
            Action::Accept {
                authorize_device, ..
            } => assert!(!authorize_device),
            _ => panic!("accept action"),
        }
    }
    fn share() -> Summary {
        Summary {
            id: uuid::Uuid::new_v4().to_string(),
            team_id: None,
            source_profile_id: "source".into(),
            profile_name: "Shared".into(),
            browser: "chrome".into(),
            sender_id: uuid::Uuid::new_v4().to_string(),
            sender_name: "Owner".into(),
            sender_email: "owner@example.com".into(),
            recipient_id: uuid::Uuid::new_v4().to_string(),
            recipient_name: "Friend".into(),
            recipient_email: "friend@example.com".into(),
            includes_session: false,
            note: String::new(),
            status: "accepted".into(),
            created_at: chrono::Utc::now().to_rfc3339(),
            expires_at: (chrono::Utc::now() + chrono::Duration::days(7)).to_rfc3339(),
            accepted_at: None,
            device_access: None,
        }
    }
    #[test]
    fn restricted_binding_survives_restart_and_failed_import() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let s = share();
        let id = uuid::Uuid::new_v4().simple().to_string();
        let mut store = Store::default();
        store.bindings.insert(
            id.clone(),
            Binding {
                account: s.recipient_id.clone(),
                local_id: id.clone(),
                share: s,
                ready: false,
            },
        );
        store.save(root.path()).unwrap();
        assert!(restricted(root.path(), &id).unwrap());
        assert!(editable(root.path(), &id).is_err());
        assert!(!visible(root.path(), &id).unwrap());
        assert!(before_launch(root.path(), &id).is_err());
        assert!(!restricted(root.path(), "ordinary").unwrap());
    }
    #[test]
    fn team_snapshot_does_not_read_or_send_the_source_proxy_credentials() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let draft = profiles::Draft {
            name: "Team configuration".into(),
            browser: "chrome".into(),
            notes: "Private operator notes".into(),
            tags: vec![],
            proxy: crate::proxy::ProxyConfig::system(),
            password: None,
            clear_password: false,
            preferences: profiles::Preferences::default(),
        };
        let mut profile = profiles::create(root.path(), draft, None).unwrap();
        profile.proxy.mode = "socks5".into();
        profile.proxy.host = "proxy.example.test".into();
        profile.proxy.port = 1080;
        profile.proxy.username = "private-user".into();
        profile.proxy.credential_ref = Some("missing-fixture-credential".into());
        let doc = team_document(profile).unwrap();
        assert_eq!(doc.name, "Team configuration");
        assert_eq!(doc.proxy.mode, "system");
        assert!(doc.proxy.host.is_empty() && doc.proxy.username.is_empty() && doc.proxy.password.is_empty());
        assert!(doc.proxy.node.is_none() && doc.proxy.api_url.is_none() && doc.proxy.pre_proxy.is_none());
        assert!(doc.notes.is_empty());
    }
    #[test]
    fn share_key_only_opens_for_addressed_grant_and_device() {
        let s = share();
        let device = crypto::DeviceKeys::generate().unwrap();
        let key = crypto::random_key().unwrap();
        let envelope =
            crypto::device_seal(&device.public_key, &envelope_scope(&s), "device", 1, &key)
                .unwrap();
        assert!(crypto::device_open(
            &device.private_key,
            &envelope_scope(&s),
            "device",
            1,
            &envelope
        )
        .is_ok());
        let mut other = s.clone();
        other.id = uuid::Uuid::new_v4().to_string();
        assert!(crypto::device_open(
            &device.private_key,
            &envelope_scope(&other),
            "device",
            1,
            &envelope
        )
        .is_err());
        let encrypted = crypto::seal(
            &key,
            &context(&s.sender_id, &s.id, "writer"),
            b"secret cookie",
        )
        .unwrap();
        assert!(!serde_json::to_string(&encrypted)
            .unwrap()
            .contains("secret cookie"));
        assert!(crypto::open(
            &key,
            &context(&s.sender_id, &other.id, "writer"),
            &encrypted
        )
        .is_err());
        let mut bad = encrypted;
        bad.ciphertext = "AAAA".into();
        assert!(crypto::open(&key, &context(&s.sender_id, &s.id, "writer"), &bad).is_err());
    }
    #[test]
    fn deadline_uses_server_time_and_monotonic_remaining() {
        let server = timestamp("2026-10-11T10:00:00Z").unwrap();
        let expires = timestamp("2026-10-11T10:05:00+00:00").unwrap();
        assert_eq!(expires - server, 300);
        let deadline = Instant::now() + Duration::from_secs((expires - server) as u64);
        assert!(deadline > Instant::now());
        assert!(timestamp("tomorrow").is_err());
    }
    #[test]
    fn authorization_policy_fails_closed_at_deadline_without_lease_and_after_account_change() {
        let s = share();
        let mut b = Binding {
            account: s.recipient_id.clone(),
            local_id: uuid::Uuid::new_v4().simple().to_string(),
            share: s,
            ready: true,
        };
        let now = Instant::now();
        let future = now + Duration::from_secs(120);
        let lease = now + Duration::from_secs(60);
        assert!(allowed_running(
            &b,
            Some(&b.account),
            Some(future),
            Some(lease),
            now
        ));
        for (account, expiry, authorization) in [
            (None, Some(future), Some(lease)),
            (Some("other"), Some(future), Some(lease)),
            (Some(b.account.as_str()), None, Some(lease)),
            (Some(b.account.as_str()), Some(future), None),
            (Some(b.account.as_str()), Some(now), Some(lease)),
            (Some(b.account.as_str()), Some(future), Some(now)),
        ] {
            assert!(!allowed_running(&b, account, expiry, authorization, now));
        }
        assert!(!allowed_running(
            &b,
            Some(&b.account),
            Some(future),
            Some(lease),
            lease
        ));
        for state in ["revoked", "expired", "left", "declined", "pending"] {
            b.share.status = state.into();
            assert!(!allowed_running(
                &b,
                Some(&b.account),
                Some(future),
                Some(lease),
                now
            ));
        }
        b.share.status = "accepted".into();
        b.ready = false;
        assert!(!allowed_running(
            &b,
            Some(&b.account),
            Some(future),
            Some(lease),
            now
        ));
    }
    struct Cleanup(Vec<(String, String)>);
    struct Headless(std::process::Child);
    impl Drop for Headless {
        fn drop(&mut self) {
            let _ = self.0.kill();
            let _ = self.0.wait();
        }
    }
    fn start_headless(path: &Path) -> Headless {
        let mut command = std::process::Command::new(platform::browser_path("firefox").unwrap());
        command
            .args(["-headless", "-no-remote", "-profile"])
            .arg(path)
            .arg("about:blank")
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        #[cfg(windows)]
        {
            use std::os::windows::process::CommandExt;
            command.creation_flags(0x08000000);
        }
        let child = Headless(command.spawn().unwrap());
        for _ in 0..100 {
            if !platform::profile_process_ids(path).unwrap().is_empty() {
                return child;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        panic!("synthetic headless Firefox did not start")
    }
    impl Drop for Cleanup {
        fn drop(&mut self) {
            for (ns, account) in &self.0 {
                for suffix in [
                    "device".to_string(),
                    "session".to_string(),
                    format!("{account}:root:1"),
                    format!("{account}:root-pending:1"),
                    format!("{account}:recovery-pending"),
                ] {
                    let _ = vault::remove(&format!("{ns}:{suffix}"));
                }
            }
        }
    }
    fn login(c: &mut Connection, dir: &Path, email: &str, name: &str, enroll_vault: bool) {
        use sha2::{Digest, Sha256};
        c.client
            .call(
                Method::POST,
                "/v1/auth/email/request",
                None,
                Some(&json!({"email":email})),
            )
            .unwrap();
        let code = std::fs::read_to_string(
            dir.join(format!("{:x}.code", Sha256::digest(email.as_bytes()))),
        )
        .unwrap();
        let response = c
            .client
            .call(
                Method::POST,
                "/v1/auth/email/verify",
                None,
                Some(&json!({"email":email,"code":code.trim(),"device":c.device.registration()})),
            )
            .unwrap();
        c.finish_login(response).unwrap();
        if enroll_vault {
            c.initialize().unwrap();
            c.acknowledge_recovery().unwrap();
        } else {
            assert!(c.unlocked_root().is_err());
            assert_eq!(c.disk.accounts[&c.account_id().unwrap()].epoch, 0);
        }
        let profile = c
            .request(
                Method::PATCH,
                "/v1/account",
                Some(&json!({"name":name,"avatar":""})),
            )
            .unwrap();
        c.disk.account = Some(serde_json::from_value(profile).unwrap());
        c.disk.save(&c.root).unwrap();
    }
    #[test]
    #[ignore = "requires isolated real Go/PostgreSQL harness and OS credential store"]
    fn desktop_real_sharing_encryption_import_guards_and_revocation() {
        let dir = std::path::PathBuf::from(
            std::env::var("SHARING_INTEROP_DIR").expect("sharing harness directory"),
        );
        let endpoint = std::fs::read_to_string(dir.join("endpoint")).unwrap();
        for enroll_vault in [true, false] {
            real_sharing_flow(&dir, endpoint.trim(), enroll_vault);
        }
        std::fs::write(dir.join("done"), b"passed").unwrap();
    }
    fn real_sharing_flow(dir: &Path, endpoint: &str, enroll_vault: bool) {
        let temp = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let aroot = temp.path().join("sender");
        let broot = temp.path().join("recipient");
        let ans = format!("sharing-test-{}", uuid::Uuid::new_v4());
        let bns = format!("sharing-test-{}", uuid::Uuid::new_v4());
        let mut cleanup = Cleanup(vec![]);
        let mut a = Connection::test_connection(
            &aroot,
            super::super::client::Client::new(endpoint.trim(), true).unwrap(),
            &ans,
        )
        .unwrap();
        let mut b = Connection::test_connection(
            &broot,
            super::super::client::Client::new(endpoint.trim(), true).unwrap(),
            &bns,
        )
        .unwrap();
        let token = uuid::Uuid::new_v4().simple().to_string();
        let ae = format!("sharing-owner-{token}@example.com");
        let be = format!("sharing-friend-{token}@example.com");
        login(&mut a, &dir, &ae, "Share Owner", enroll_vault);
        cleanup.0.push((ans, a.account_id().unwrap()));
        login(&mut b, &dir, &be, "Share Friend", enroll_vault);
        cleanup.0.push((bns, b.account_id().unwrap()));
        let draft:profiles::Draft=serde_json::from_value(json!({"name":"Portable shared fixture","browser":"firefox","notes":"owner original","tags":[],"proxy":{"mode":"direct","host":"","port":0,"username":""},"preferences":profiles::Preferences::default()})).unwrap();
        let original = profiles::create(&aroot, draft.clone(), None).unwrap();
        let runtime = profiles::Runtime::default();
        let input=Snapshot::parse(json!({"schema":1,"cookies":[{"name":"synthetic_share","value":"share-cookie-secret","domain":"sharing-fixture.invalid","path":"/","secure":true,"httpOnly":true,"sameSite":"Lax","expires":chrono::Utc::now().timestamp()+3600}],"origins":[{"origin":"https://sharing-fixture.invalid","state":{"local":[["login","synthetic-shared-login"]],"databases":[]}}]})).unwrap();
        if platform::browser_path("firefox").is_ok() {
            super::super::browser_session::restore_profile(
                "firefox",
                &profiles::path(&aroot, &original).unwrap(),
                &input,
            )
            .unwrap();
        }
        for include in [false, true] {
            if include && platform::browser_path("firefox").is_err() {
                continue;
            }
            let recipient = a
                .request(
                    Method::POST,
                    "/v1/shares/recipient",
                    Some(&json!({"name":"Share Friend","email":be})),
                )
                .unwrap();
            let id = uuid::Uuid::new_v4().to_string();
            let sender = a.account_id().unwrap();
            let recipient_id = b.account_id().unwrap();
            let payload = Payload {
                schema: 1,
                id: id.clone(),
                sender_id: sender.clone(),
                recipient_id: recipient_id.clone(),
                source_profile_id: original.id.clone(),
                includes_session: include,
                document: Document::export(&original).unwrap(),
                session: if include {
                    Some(
                        super::super::browser_session::export_profile(
                            "firefox",
                            &profiles::path(&aroot, &original).unwrap(),
                        )
                        .unwrap(),
                    )
                } else {
                    None
                },
            };
            let key = crypto::random_key().unwrap();
            let encrypted = crypto::seal(
                &key,
                &context(&sender, &id, &a.device.id),
                &serde_json::to_vec(&payload).unwrap(),
            )
            .unwrap();
            let ciphertext = serde_json::to_string(&encrypted).unwrap();
            assert!(!ciphertext.contains("share-cookie-secret"));
            let scope = format!("share:{sender}:{id}:{recipient_id}");
            let mut envelopes = vec![];
            for d in recipient["devices"].as_array().unwrap() {
                let public = d["publicKey"].as_str().unwrap();
                let device = d["id"].as_str().unwrap();
                envelopes.push(json!({"deviceId":device,"publicKey":public,"envelope":crypto::device_seal(&*vault::decode_key(public).unwrap(),&scope,device,1,&key).unwrap()}));
            }
            let response=a.request(Method::POST,"/v1/shares",Some(&json!({"id":id,"sourceProfileId":original.id,"profileName":original.name,"browser":original.browser,"recipientId":recipient_id,"recipientName":"Share Friend","recipientEmail":be,"expiresAt":(chrono::Utc::now()+chrono::Duration::days(7)).to_rfc3339(),"includesSession":include,"note":"fixture","payload":ciphertext,"writer":a.device.id,"envelopes":envelopes}))).unwrap();
            let invitation = summary(&response["share"]).unwrap();
            assert!(authorization(&mut b, &invitation).is_err());
            assert!(b
                .request(Method::GET, &format!("/v1/shares/{id}/payload"), None)
                .is_err());
            let listing = b.request(Method::GET, "/v1/shares", None).unwrap();
            let listed = listing["received"]
                .as_array()
                .unwrap()
                .iter()
                .find(|item| item["id"].as_str() == Some(&id))
                .unwrap();
            assert_eq!(
                listed["deviceAccess"],
                if enroll_vault {
                    "ready"
                } else {
                    "authorizationRequired"
                }
            );
            if !enroll_vault {
                assert!(b
                    .request(
                        Method::POST,
                        &format!("/v1/shares/{id}/accept"),
                        Some(&json!({}))
                    )
                    .is_err());
                let unchanged = b.request(Method::GET, "/v1/shares", None).unwrap();
                assert_eq!(
                    unchanged["received"]
                        .as_array()
                        .unwrap()
                        .iter()
                        .find(|item| item["id"].as_str() == Some(&id))
                        .unwrap()["status"],
                    "pending"
                );
                if !include {
                    let extra_root = temp.path().join("unaddressed");
                    let extra_ns = format!("sharing-test-{}", uuid::Uuid::new_v4());
                    let mut extra = Connection::test_connection(
                        &extra_root,
                        super::super::client::Client::new(endpoint.trim(), true).unwrap(),
                        &extra_ns,
                    )
                    .unwrap();
                    login(&mut extra, dir, &be, "Share Friend", false);
                    cleanup.0.push((extra_ns, extra.account_id().unwrap()));
                    let extra_list = extra.request(Method::GET, "/v1/shares", None).unwrap();
                    assert_eq!(
                        extra_list["received"]
                            .as_array()
                            .unwrap()
                            .iter()
                            .find(|item| item["id"].as_str() == Some(&id))
                            .unwrap()["deviceAccess"],
                        "unavailable"
                    );
                    assert!(extra
                        .request(
                            Method::POST,
                            &format!("/v1/shares/{id}/accept"),
                            Some(&json!({"authorizeDevice":true}))
                        )
                        .unwrap_err()
                        .contains("share_device_unavailable"));
                    assert!(authorization(&mut extra, &invitation).is_err());
                } else {
                    a.logout().unwrap();
                }
            }
            let response = b
                .request(
                    Method::POST,
                    &format!("/v1/shares/{id}/accept"),
                    Some(&json!({"authorizeDevice":!enroll_vault})),
                )
                .unwrap();
            let grant = summary(&response["share"]).unwrap();
            if !enroll_vault {
                assert!(b.unlocked_root().is_err());
                assert!(a.unlocked_root().is_err());
                let personal = b.request(Method::GET, "/v1/vault", None).unwrap();
                assert_eq!(personal["initialized"], false);
                assert!(!personal["approved"].as_bool().unwrap_or(false));
                assert!(b.disk.accounts[&recipient_id].authorized_device.is_none());
            }
            if !include {
                // Two stale connections must refresh once, never revoke the refresh-token family.
                let saved = vault::read(&format!("{}:session", b.ns)).unwrap().unwrap();
                let mut expired: super::super::account::Session =
                    serde_json::from_str(&saved).unwrap();
                expired.expires_at = 0;
                b.save_session(expired).unwrap();
                let mut calls = vec![];
                for _ in 0..2 {
                    let mut connection = Connection::test_connection(
                        &broot,
                        super::super::client::Client::new(endpoint.trim(), true).unwrap(),
                        &b.ns,
                    )
                    .unwrap();
                    calls.push(std::thread::spawn(move || {
                        connection.request(Method::GET, "/v1/shares", None)
                    }));
                }
                for call in calls {
                    call.join().unwrap().unwrap();
                }
            }
            authorization(&mut b, &grant).unwrap();
            let remote = b
                .request(Method::GET, &format!("/v1/shares/{id}/payload"), None)
                .unwrap();
            let opened_key = crypto::device_open(
                &*vault::decode_key(&b.device.private_key).unwrap(),
                &scope,
                &b.device.id,
                1,
                remote["envelope"].as_str().unwrap(),
            )
            .unwrap();
            let remote_envelope: crypto::Envelope =
                serde_json::from_str(remote["payload"].as_str().unwrap()).unwrap();
            let bytes = crypto::open(
                &opened_key,
                &context(&sender, &id, remote["writer"].as_str().unwrap()),
                &remote_envelope,
            )
            .unwrap();
            let frozen = Payload::parse(&bytes, &grant).unwrap();
            let local = reserve_import(&broot, &recipient_id, &grant, &frozen).unwrap();
            assert!(!visible(&broot, &local).unwrap());
            let (live, _) = authorization(&mut b, &grant).unwrap();
            expose_import(&broot, &local, live).unwrap();
            assert!(visible(&broot, &local).unwrap());
            assert_eq!(
                reserve_import(&broot, &recipient_id, &grant, &frozen).unwrap(),
                local
            ); // Retry never creates an unrestricted duplicate.
            let imported = profiles::get(&broot, &local, false).unwrap();
            assert!(Document::export(&imported).is_err());
            assert!(profiles::update(&broot, &local, draft.clone()).is_err());
            assert!(profiles::create(&broot, draft.clone(), Some(&local)).is_err());
            assert!(super::super::sync::setup(
                &mut b,
                "upload",
                &[local.clone()],
                &[],
                false,
                &runtime
            )
            .is_err());
            assert!(editable(&broot, &local).is_err());
            if include {
                let restored = super::super::browser_session::export_profile(
                    "firefox",
                    &profiles::path(&broot, &imported).unwrap(),
                )
                .unwrap();
                assert!(restored
                    .cookies
                    .iter()
                    .any(|c| c.name == "synthetic_share" && c.value == "share-cookie-secret"));
                assert_eq!(restored.origins[0].state, input.origins[0].state);
            }
            assert_eq!(
                profiles::get(&aroot, &original.id, false).unwrap().notes,
                "owner original"
            );
            if include {
                let own_path = profiles::path(&aroot, &original).unwrap();
                let shared_path = profiles::path(&broot, &imported).unwrap();
                let _owner = start_headless(&own_path);
                let _shared = start_headless(&shared_path);
                close(&broot, &local, &runtime).unwrap();
                assert!(platform::profile_process_ids(&shared_path)
                    .unwrap()
                    .is_empty());
                assert!(!platform::profile_process_ids(&own_path).unwrap().is_empty());
            }
            if include {
                if !enroll_vault {
                    login(&mut a, dir, &ae, "Share Owner", false);
                }
                a.request(
                    Method::POST,
                    &format!("/v1/shares/{id}/revoke"),
                    Some(&json!({})),
                )
                .unwrap();
            } else {
                a.request(Method::PATCH,&format!("/v1/shares/{id}"),Some(&json!({"expiresAt":(chrono::Utc::now()+chrono::Duration::seconds(2)).to_rfc3339()}))).unwrap();
                let (_, remaining) = authorization(&mut b, &grant).unwrap();
                assert!(remaining <= Duration::from_secs(2));
                std::thread::sleep(Duration::from_secs(3));
            }
            assert!(authorization(&mut b, &grant).is_err());
            assert!(b
                .request(Method::GET, &format!("/v1/shares/{id}/payload"), None)
                .is_err());
            let mut wrong = grant.clone();
            wrong.recipient_id = sender;
            assert!(authorization(&mut b, &wrong).is_err());
            // A missing/corrupt sharing store cannot remove the registry restriction.
            std::fs::remove_file(broot.join("cloud/sharing.json")).unwrap();
            assert!(restricted(&broot, &local).unwrap());
            assert!(profiles::create(&broot, draft.clone(), Some(&local)).is_err());
            assert!(!visible(&broot, &local).unwrap());
        }
    }
}
