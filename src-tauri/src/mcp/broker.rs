use super::{catalog, operations, policy::Grant};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    io::{Read, Write},
    net::TcpListener,
    path::Path,
    sync::{
        atomic::{AtomicBool, Ordering},
        Arc, Mutex,
    },
    time::Duration,
};
use tauri::{Emitter, Manager};

const SERVICE: &str = "com.nodecloak.mcp";
pub fn secret_read(name: &str) -> Result<String, String> {
    keyring::Entry::new(SERVICE, name)
        .map_err(|_| "KEYRING_UNAVAILABLE")?
        .get_password()
        .map_err(|_| "MCP_CREDENTIAL_NOT_FOUND".to_string())
}
fn secret_write(name: &str, value: &str) -> Result<(), String> {
    keyring::Entry::new(SERVICE, name)
        .map_err(|_| "KEYRING_UNAVAILABLE")?
        .set_password(value)
        .map_err(|_| "KEYRING_UNAVAILABLE".into())
}
fn secret_remove(name: &str) {
    if let Ok(e) = keyring::Entry::new(SERVICE, name) {
        let _ = e.delete_credential();
    }
}
pub fn hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))
}
pub fn now() -> i64 {
    chrono::Utc::now().timestamp()
}
#[derive(Default, Serialize, Deserialize)]
pub struct Config {
    pub enabled: bool,
    pub grants: Vec<Grant>,
    #[serde(default, rename = "remoteRoutes")]
    pub remote_routes: Vec<super::remote::RemoteRoute>,
}
pub struct Job {
    pub remote_context: Option<super::remote::Context>,
    pub id: String,
    pub client: String,
    pub tool: String,
    pub args: Value,
    pub revision: Option<String>,
    pub target: Option<Value>,
    pub state: String,
    pub result: Value,
    pub error: Option<String>,
    pub created: i64,
    pub cancel: Arc<AtomicBool>,
    archived: bool,
}
impl Job {
    fn audit_record(&self) -> Value {
        let mut v = self.summary();
        v["arguments"] = json!({"profile_id":self.args["profile_id"],"tab_id":self.args["tab_id"]});
        if let Some(error) = self.error.as_deref() {
            let code = error.split(':').next().unwrap_or("OPERATION_FAILED");
            v["error"] = json!(
                if code.bytes().all(|c| c.is_ascii_uppercase() || c == b'_') {
                    code
                } else {
                    "OPERATION_FAILED"
                }
            );
        }
        v
    }
    fn desktop_view(&self) -> Value {
        let mut v = self.summary();
        if self.state == "awaiting_approval" {
            v["arguments"] = self.args.clone();
            v["target"] = json!(self.target);
        }
        v
    }
    fn view(&self) -> Value {
        let mut value = self.summary();
        value["result"] = self.result.clone();
        value
    }
    fn summary(&self) -> Value {
        let mut args = self.args.clone();
        if let Some(a) = args.as_object_mut() {
            a.remove("value");
            a.remove("text");
        }
        json!({"id":self.id,"clientId":self.client,"tool":self.tool,"arguments":args,"state":self.state,"result":null,"error":self.error,"createdAt":self.created})
    }
}
#[derive(Default)]
pub struct State {
    pub config: Config,
    pub namespace: String,
    pub port: u16,
    pub error: Option<String>,
    pub jobs: Vec<Job>,
    pub history: Vec<Value>,
    pub history_error: Option<String>,
    pub gates: HashMap<String, Arc<Mutex<()>>>,
    pub owned_tabs: HashMap<(String, String, String), Vec<String>>,
    pub remote_error: Option<String>,
    pub remote_seen: i64,
    pub remote_active: Vec<String>,
}
#[derive(Default)]
pub struct Runtime(pub Mutex<State>);
fn expire_approvals(s: &mut State) {
    for j in &mut s.jobs {
        if j.state == "awaiting_approval" && j.created + 600 < now() {
            j.cancel.store(true, Ordering::SeqCst);
            j.state = "cancelled".into();
            j.error = Some("APPROVAL_EXPIRED".into());
        }
    }
}
fn save(root: &Path, c: &Config) -> Result<(), String> {
    let path = root.join("mcp.json");
    crate::profiles::guard_tree(&path)?;
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|_| "MCP_CONFIG_WRITE_FAILED")?;
    serde_json::to_writer(&mut file, c).map_err(|_| "MCP_CONFIG_WRITE_FAILED")?;
    file.as_file_mut()
        .sync_all()
        .map_err(|_| "MCP_CONFIG_WRITE_FAILED")?;
    file.persist(path).map_err(|_| "MCP_CONFIG_WRITE_FAILED")?;
    Ok(())
}
pub fn persist(app: &tauri::AppHandle, c: &Config) -> Result<(), String> {
    save(&app.state::<crate::data_root::DataRoot>().0, c)
}
pub fn start(app: tauri::AppHandle) {
    let result = (|| -> Result<(TcpListener, String), String> {
        let root = &app.state::<crate::data_root::DataRoot>().0;
        let path = root.join("mcp.json");
        crate::profiles::guard_tree(&path)?;
        let config = if path.exists() {
            if std::fs::metadata(&path)
                .map_err(|_| "MCP_CONFIG_READ_FAILED")?
                .len()
                > 1_048_576
            {
                return Err("MCP_CONFIG_TOO_LARGE".into());
            }
            serde_json::from_slice(&std::fs::read(path).map_err(|_| "MCP_CONFIG_READ_FAILED")?)
                .map_err(|_| "MCP_CONFIG_INVALID")?
        } else {
            Config::default()
        };
        let ns = hash(&root.to_string_lossy());
        let listener = TcpListener::bind("127.0.0.1:0").map_err(|_| "MCP_LISTEN_FAILED")?;
        let port = listener
            .local_addr()
            .map_err(|_| "MCP_LISTEN_FAILED")?
            .port();
        if config.enabled {
            secret_write(&format!("{ns}:endpoint"), &port.to_string())?;
        }
        let runtime = app.state::<Runtime>();
        let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
        s.config = config;
        let history = root.join("mcp-history.json");
        if crate::profiles::guard_tree(&history).is_ok() {
            if let Ok(meta) = std::fs::metadata(&history) {
                if meta.len() <= 524288 {
                    s.history = std::fs::read(history)
                        .ok()
                        .and_then(|v| serde_json::from_slice::<Vec<Value>>(&v).ok())
                        .unwrap_or_default();
                    s.history.truncate(128);
                }
            }
        }

        s.namespace = ns.clone();
        s.port = port;
        Ok((listener, ns))
    })();
    match result {
        Err(error) => {
            if let Ok(mut s) = app.state::<Runtime>().0.lock() {
                s.error = Some(error);
            }
        }
        Ok((listener, _)) => {
            std::thread::spawn(move || {
                let active = Arc::new(std::sync::atomic::AtomicUsize::new(0));
                for incoming in listener.incoming() {
                    let Ok(mut stream) = incoming else { break };
                    if active.fetch_add(1, Ordering::SeqCst) >= 16 {
                        active.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let handle = app.clone();
                    let count = active.clone();
                    std::thread::spawn(move || {
                        let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
                        let _ = stream.set_write_timeout(Some(Duration::from_secs(5)));
                        let handshake =
                            (|| -> Result<(String, zeroize::Zeroizing<String>), String> {
                                let hello = read_frame(&mut stream)?;
                                let client = hello["client"].as_str().ok_or("UNAUTHORIZED")?;
                                let nonce = hello["nonce"]
                                    .as_str()
                                    .filter(|s| s.len() == 64)
                                    .ok_or("UNAUTHORIZED")?;
                                current_grant(&handle, client)?;
                                let credential = credential(&handle, client)?;
                                let server_nonce = crate::fingerprint::seed()?;
                                let binding = format!("{nonce}:{server_nonce}");
                                let signature = proof(
                                    &credential,
                                    &binding,
                                    &json!({"phase":"server","client":client}),
                                )?;
                                write_frame(
                                    &mut stream,
                                    &json!({"nonce":server_nonce,"proof":signature}),
                                )?;
                                Ok((binding, credential))
                            })();
                        if let Ok((binding, credential)) = handshake {
                            if let Ok(value) = read_frame(&mut stream)
                                .and_then(|v| open(&credential, &binding, "request", &v))
                            {
                                let reply = handle_request(&handle, value, &binding, None);
                                if let Ok(encrypted) = seal(&credential, &binding, "reply", &reply)
                                {
                                    let _ = write_frame(&mut stream, &encrypted);
                                }
                            }
                        } else if let Err(error) = handshake {
                            let _ = write_frame(&mut stream, &json!({"ok":false,"error":error}));
                        }
                        count.fetch_sub(1, Ordering::SeqCst);
                    });
                }
            });
        }
    }
}
pub fn read_frame(stream: &mut impl Read) -> Result<Value, String> {
    read_bounded(stream, 1_048_576)
}
pub fn read_response(stream: &mut impl Read) -> Result<Value, String> {
    read_bounded(stream, 12_000_000)
}
fn read_bounded(stream: &mut impl Read, max: usize) -> Result<Value, String> {
    let mut magic = [0u8; 8];
    stream
        .read_exact(&mut magic)
        .map_err(|_| "IPC_READ_FAILED")?;
    if &magic != b"NCMCP001" {
        return Err("IPC_INVALID_FRAME".into());
    }
    let mut length = [0; 4];
    stream
        .read_exact(&mut length)
        .map_err(|_| "IPC_READ_FAILED")?;
    let n = u32::from_be_bytes(length) as usize;
    if n == 0 || n > max {
        return Err("IPC_FRAME_TOO_LARGE".into());
    }
    let mut data = vec![0; n];
    stream
        .read_exact(&mut data)
        .map_err(|_| "IPC_READ_FAILED")?;
    serde_json::from_slice(&data).map_err(|_| "IPC_INVALID_JSON".into())
}
pub fn write_frame(stream: &mut impl Write, value: &Value) -> Result<(), String> {
    let data = serde_json::to_vec(value).map_err(|_| "IPC_ENCODE_FAILED")?;
    if data.len() > 12_000_000 {
        return Err("IPC_RESPONSE_TOO_LARGE".into());
    }
    stream
        .write_all(b"NCMCP001")
        .and_then(|_| stream.write_all(&(data.len() as u32).to_be_bytes()))
        .and_then(|_| stream.write_all(&data))
        .map_err(|_| "IPC_WRITE_FAILED".into())
}
pub fn proof(key: &str, nonce: &str, body: &Value) -> Result<String, String> {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| "UNAUTHORIZED")?;
    mac.update(nonce.as_bytes());
    mac.update(&serde_json::to_vec(body).map_err(|_| "IPC_ENCODE_FAILED")?);
    Ok(base64::engine::general_purpose::STANDARD.encode(mac.finalize().into_bytes()))
}
fn session_cipher(
    secret: &str,
    binding: &str,
    direction: &str,
) -> Result<chacha20poly1305::ChaCha20Poly1305, String> {
    use chacha20poly1305::KeyInit;
    let mut key = zeroize::Zeroizing::new([0u8; 32]);
    hkdf::Hkdf::<Sha256>::new(Some(binding.as_bytes()), secret.as_bytes())
        .expand(
            format!("nodecloak-mcp-v1:{direction}").as_bytes(),
            &mut *key,
        )
        .map_err(|_| "IPC_KEY_FAILED")?;
    chacha20poly1305::ChaCha20Poly1305::new_from_slice(&*key).map_err(|_| "IPC_KEY_FAILED".into())
}
pub fn seal(secret: &str, binding: &str, direction: &str, value: &Value) -> Result<Value, String> {
    use base64::Engine;
    use chacha20poly1305::aead::{Aead, Payload};
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "IPC_RANDOM_FAILED")?;
    let data = serde_json::to_vec(value).map_err(|_| "IPC_ENCODE_FAILED")?;
    let encrypted = session_cipher(secret, binding, direction)?
        .encrypt(
            chacha20poly1305::Nonce::from_slice(&nonce),
            Payload {
                msg: &data,
                aad: binding.as_bytes(),
            },
        )
        .map_err(|_| "IPC_ENCRYPT_FAILED")?;
    Ok(
        json!({"nonce":base64::engine::general_purpose::STANDARD.encode(nonce),"sealed":base64::engine::general_purpose::STANDARD.encode(encrypted)}),
    )
}
pub fn open(secret: &str, binding: &str, direction: &str, value: &Value) -> Result<Value, String> {
    use base64::Engine;
    use chacha20poly1305::aead::{Aead, Payload};
    let decode = |key: &str| {
        base64::engine::general_purpose::STANDARD
            .decode(value[key].as_str().ok_or("IPC_INVALID_CIPHERTEXT")?)
            .map_err(|_| "IPC_INVALID_CIPHERTEXT")
    };
    let nonce = decode("nonce")?;
    if nonce.len() != 12 {
        return Err("IPC_INVALID_CIPHERTEXT".into());
    }
    let encrypted = decode("sealed")?;
    let plaintext = zeroize::Zeroizing::new(
        session_cipher(secret, binding, direction)?
            .decrypt(
                chacha20poly1305::Nonce::from_slice(&nonce),
                Payload {
                    msg: &encrypted,
                    aad: binding.as_bytes(),
                },
            )
            .map_err(|_| "IPC_AUTHENTICATION_FAILED")?,
    );
    serde_json::from_slice(&plaintext).map_err(|_| "IPC_INVALID_JSON".into())
}
pub fn verify_proof(key: &str, nonce: &str, body: &Value, signature: &str) -> Result<(), String> {
    use base64::Engine;
    use hmac::{Hmac, Mac};
    let mut mac = Hmac::<Sha256>::new_from_slice(key.as_bytes()).map_err(|_| "UNAUTHORIZED")?;
    mac.update(nonce.as_bytes());
    mac.update(&serde_json::to_vec(body).map_err(|_| "IPC_ENCODE_FAILED")?);
    let data = base64::engine::general_purpose::STANDARD
        .decode(signature)
        .map_err(|_| "UNAUTHORIZED")?;
    mac.verify_slice(&data).map_err(|_| "UNAUTHORIZED".into())
}
fn credential(app: &tauri::AppHandle, id: &str) -> Result<zeroize::Zeroizing<String>, String> {
    let namespace = app
        .state::<Runtime>()
        .0
        .lock()
        .map_err(|_| "MCP_LOCK_FAILED")?
        .namespace
        .clone();
    Ok(zeroize::Zeroizing::new(secret_read(&format!(
        "{namespace}:{id}"
    ))?))
}
fn authenticated(
    app: &tauri::AppHandle,
    id: &str,
    signature: &str,
    nonce: &str,
    body: &Value,
) -> Result<Grant, String> {
    current_grant(app, id)?;
    verify_proof(&credential(app, id)?, nonce, body, signature)?;
    current_grant(app, id)
}

fn handle_request(
    app: &tauri::AppHandle,
    value: Value,
    nonce: &str,
    remote_context: Option<super::remote::Context>,
) -> Value {
    let result: Result<Value, String> = (|| {
        let g = authenticated(
            app,
            value["client"].as_str().ok_or("UNAUTHORIZED")?,
            value["proof"].as_str().ok_or("UNAUTHORIZED")?,
            nonce,
            &json!({"client":value["client"],"tool":value["tool"],"args":value["args"]}),
        )?;
        let tool = value["tool"].as_str().ok_or("INVALID_ARGUMENTS")?;
        let args = &value["args"];
        catalog::validate(tool, args)?;
        g.authorize(
            catalog::permission(tool).ok_or("UNKNOWN_TOOL")?,
            args["profile_id"].as_str(),
            now(),
        )?;
        if tool == "jobs_get" || tool == "jobs_cancel" {
            let runtime = app.state::<Runtime>();
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            if !s.config.enabled {
                return Err("MCP_DISABLED".into());
            }
            s.config
                .grants
                .iter()
                .find(|x| x.id == g.id)
                .ok_or("UNAUTHORIZED")?
                .authorize("read", None, now())?;
            let job = s
                .jobs
                .iter_mut()
                .find(|j| j.id == args["job_id"].as_str().unwrap_or("") && j.client == g.id)
                .ok_or("JOB_NOT_FOUND")?;
            if tool == "jobs_cancel" {
                job.cancel.store(true, Ordering::SeqCst);
                if job.state == "awaiting_approval" {
                    job.state = "cancelled".into();
                }
            }
            let view = job.view();
            archive_finished(app, &mut s);
            drop(s);
            if tool == "jobs_cancel" {
                approval_event(app);
            }
            return Ok(view);
        }
        let approval = [
            "profiles_trash",
            "fingerprint_regenerate",
            "page_upload",
            "sync_resolve",
        ]
        .contains(&tool)
            || (g.confirm_interactions && catalog::permission(tool) == Some("page_write"));
        let preview = approval && args["tab_id"].is_string();
        let revision = if ["profiles_trash", "fingerprint_regenerate"].contains(&tool) {
            Some(
                crate::profiles::get(
                    &app.state::<crate::data_root::DataRoot>().0,
                    args["profile_id"].as_str().ok_or("PROFILE_REQUIRED")?,
                    true,
                )?
                .updated_at,
            )
        } else {
            None
        };
        let id = uuid::Uuid::new_v4().to_string();
        {
            let runtime = app.state::<Runtime>();
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            expire_approvals(&mut s);
            if s.jobs
                .iter()
                .filter(|j| ["running", "queued", "awaiting_approval"].contains(&j.state.as_str()))
                .count()
                >= 32
            {
                return Err("TOO_MANY_ACTIVE_JOBS".into());
            }
            if s.jobs.len() >= 128 {
                if let Some(i) = s.jobs.iter().position(|j| {
                    !["running", "queued", "awaiting_approval"].contains(&j.state.as_str())
                }) {
                    s.jobs.remove(i);
                }
            }
            s.jobs.push(Job {
                remote_context,
                id: id.clone(),
                client: g.id,
                tool: tool.into(),
                args: args.clone(),
                revision,
                target: None,
                state: if approval && !preview {
                    "awaiting_approval"
                } else {
                    "queued"
                }
                .into(),
                result: Value::Null,
                error: None,
                created: now(),
                cancel: Arc::new(AtomicBool::new(false)),
                archived: false,
            });
        }
        if preview {
            prepare_approval(app.clone(), id.clone());
        } else if !approval {
            run_job(app.clone(), id.clone());
        } else {
            approval_event(app);
        }
        // Give quick metadata calls a chance to complete, without holding any runtime lock.
        for _ in 0..15 {
            let runtime = app.state::<Runtime>();
            let s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let job = s.jobs.iter().find(|j| j.id == id).ok_or("JOB_NOT_FOUND")?;
            if approval || !["queued", "running"].contains(&job.state.as_str()) {
                return Ok(job.view());
            }
            drop(s);
            std::thread::sleep(Duration::from_millis(20));
        }
        let runtime = app.state::<Runtime>();
        let s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
        Ok(s.jobs
            .iter()
            .find(|j| j.id == id)
            .ok_or("JOB_NOT_FOUND")?
            .view())
    })();
    match result {
        Ok(result) => json!({"ok":true,"result":result}),
        Err(error) => json!({"ok":false,"error":error}),
    }
}
/// Only native desktop code can enter here; the grant/keyring proof path stays identical to STDIO.
pub(crate) fn local_request(
    app: &tauri::AppHandle,
    client: &str,
    tool: &str,
    args: Value,
) -> Value {
    request_with_context(app, client, tool, args, None)
}
pub(crate) fn remote_request(
    app: &tauri::AppHandle,
    client: &str,
    tool: &str,
    args: Value,
    context: super::remote::Context,
) -> Value {
    request_with_context(app, client, tool, args, Some(context))
}
fn request_with_context(
    app: &tauri::AppHandle,
    client: &str,
    tool: &str,
    args: Value,
    context: Option<super::remote::Context>,
) -> Value {
    let outcome = (|| -> Result<Value, String> {
        let nonce = crate::fingerprint::seed()?;
        let mut value = json!({"client":client,"tool":tool,"args":args});
        value["proof"] = json!(proof(&credential(app, client)?, &nonce, &value)?);
        Ok(handle_request(app, value, &nonce, context))
    })();
    outcome.unwrap_or_else(|e| json!({"ok":false,"error":e}))
}
fn prepare_approval(app: tauri::AppHandle, id: String) {
    std::thread::spawn(move || {
        let result = (|| {
            let (client, tool, args, cancel) = {
                let runtime = app.state::<Runtime>();
                let s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
                let j = s.jobs.iter().find(|j| j.id == id).ok_or("JOB_NOT_FOUND")?;
                (
                    j.client.clone(),
                    j.tool.clone(),
                    j.args.clone(),
                    j.cancel.clone(),
                )
            };
            let grant = current_grant(&app, &client)?;
            grant.authorize(
                catalog::permission(&tool).ok_or("UNKNOWN_TOOL")?,
                args["profile_id"].as_str(),
                now(),
            )?;
            operations::execute(
                &app,
                &grant,
                if tool == "page_dialog" {
                    "__preview_dialog"
                } else {
                    "__preview"
                },
                &args,
                cancel,
            )
        })();
        if let Ok(mut s) = app.state::<Runtime>().0.lock() {
            if let Some(j) = s.jobs.iter_mut().find(|j| j.id == id) {
                if j.cancel.load(Ordering::SeqCst) {
                    j.state = "cancelled".into();
                } else {
                    match result {
                        Ok(target) => {
                            j.target = Some(target);
                            j.state = "awaiting_approval".into();
                        }
                        Err(error) => {
                            j.error = Some(error);
                            j.state = "failed".into();
                        }
                    }
                }
            }
        }
        approval_event(&app);
    });
}
pub fn current_grant(app: &tauri::AppHandle, id: &str) -> Result<Grant, String> {
    let runtime = app.state::<Runtime>();
    let s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
    if !s.config.enabled {
        return Err("MCP_DISABLED".into());
    }
    let g = s
        .config
        .grants
        .iter()
        .find(|g| g.id == id)
        .ok_or("UNAUTHORIZED")?
        .clone();
    g.authorize("read", None, now())?;
    Ok(g)
}
fn run_job(app: tauri::AppHandle, id: String) {
    std::thread::spawn(move || {
        let data = (|| {
            let runtime = app.state::<Runtime>();
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let j = s
                .jobs
                .iter_mut()
                .find(|j| j.id == id)
                .ok_or("JOB_NOT_FOUND")?;
            if j.cancel.load(Ordering::SeqCst) {
                j.state = "cancelled".into();
                return Err("CANCELLED");
            }
            j.state = "running".into();
            let mut args = j.args.clone();
            if let Some(context) = &j.remote_context {
                args["_remote_context"] = json!(context);
            }
            if let Some(revision) = &j.revision {
                args["_expected_revision"] = json!(revision);
            }
            if let Some(target) = &j.target {
                args["_expected_target"] = target.clone();
            }
            let data = (j.client.clone(), j.tool.clone(), args, j.cancel.clone());
            let gate = s
                .gates
                .entry(data.2["profile_id"].as_str().unwrap_or("metadata").into())
                .or_default()
                .clone();
            Ok((data, gate))
        })();
        let Ok(((client, tool, args, cancel), gate)) = data else {
            return;
        };
        let _guard = gate.lock().ok();
        let result = (|| {
            let grant = super::remote::bounded_grant(&app, &current_grant(&app, &client)?, &args)?;
            grant.authorize(
                catalog::permission(&tool).ok_or("UNKNOWN_TOOL")?,
                args["profile_id"].as_str(),
                now(),
            )?;
            if cancel.load(Ordering::SeqCst) {
                return Err("CANCELLED".into());
            }
            operations::execute(&app, &grant, &tool, &args, cancel.clone())
        })();
        if let Ok(mut s) = app.state::<Runtime>().0.lock() {
            if let Some(j) = s.jobs.iter_mut().find(|j| j.id == id) {
                match result {
                    Ok(value) => {
                        j.result = value;
                        j.state = "completed".into();
                    }
                    Err(error) => {
                        j.error = Some(error);
                        j.state = if cancel.load(Ordering::SeqCst) {
                            "cancelled"
                        } else {
                            "failed"
                        }
                        .into();
                    }
                }
            }
            archive_finished(&app, &mut s);
            let mut budget = 32_000_000usize;
            for j in s.jobs.iter_mut().rev() {
                let size = j.result.to_string().len();
                if size > budget {
                    j.result = json!({"result_expired":true,"message":"Read large results promptly; repeat read operations if needed."});
                } else {
                    budget -= size;
                }
            }
        }
    });
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn audit_does_not_keep_form_values_page_content_or_configuration() {
        let j = Job {
            remote_context: None,
            id: "job".into(),
            client: "client".into(),
            tool: "page_fill".into(),
            args: json!({"profile_id":"one","value":"form-secret","text":"prompt-secret","notes":"private-notes","url":"https://example.test/?token=url-secret"}),
            revision: None,
            target: None,
            state: "completed".into(),
            result: json!({"text":"page-secret","image_base64":"image-secret"}),
            error: Some("PAGE_OPERATION_FAILED: private-error".into()),
            created: 1,
            cancel: Arc::new(AtomicBool::new(false)),
            archived: false,
        };
        let text = j.audit_record().to_string();
        for secret in [
            "form-secret",
            "prompt-secret",
            "private-notes",
            "url-secret",
            "page-secret",
            "image-secret",
            "private-error",
        ] {
            assert!(!text.contains(secret), "audit persisted {secret}");
        }
        assert_eq!(j.audit_record()["arguments"]["profile_id"], "one");
        let mut expired = j;
        expired.state = "awaiting_approval".into();
        expired.created = now() - 601;
        let mut state = State::default();
        state.jobs.push(expired);
        expire_approvals(&mut state);
        assert_eq!(state.jobs[0].state, "cancelled");
        assert_eq!(state.jobs[0].error.as_deref(), Some("APPROVAL_EXPIRED"));
        assert!(state.jobs[0].cancel.load(Ordering::SeqCst));
    }
    #[test]
    fn ipc_rejects_http_and_large_frames_before_allocating() {
        assert!(read_frame(&mut &b"GET / HTTP/1.1\r\n"[..]).is_err());
        let mut bytes = b"NCMCP001".to_vec();
        bytes.extend(2_000_000u32.to_be_bytes());
        assert!(read_frame(&mut bytes.as_slice())
            .unwrap_err()
            .contains("TOO_LARGE"));
        let value = json!({"client":"fixture","tool":"profiles_list","args":{}});
        let mut bytes = vec![];
        write_frame(&mut bytes, &value).unwrap();
        assert_eq!(read_frame(&mut bytes.as_slice()).unwrap(), value);
    }
    #[test]
    fn mutual_proofs_bind_role_nonce_and_immutable_arguments() {
        let key = hash("test-only-key");
        let body = json!({"client":"one","tool":"page_fill","args":{"value":"secret"}});
        let signature = proof(&key, "first-connection", &body).unwrap();
        assert!(verify_proof(&key, "first-connection", &body, &signature).is_ok());
        assert!(verify_proof(&key, "another-connection", &body, &signature).is_err());
        assert!(verify_proof(
            &key,
            "first-connection",
            &json!({"phase":"server","client":"one"}),
            &signature
        )
        .is_err());
        let mut changed = body.clone();
        changed["args"]["value"] = json!("changed");
        assert!(verify_proof(&key, "first-connection", &changed, &signature).is_err());
        let sealed = seal("keyring-only-secret", "connection-one", "request", &body).unwrap();
        assert!(!sealed.to_string().contains("secret"));
        assert_eq!(
            open("keyring-only-secret", "connection-one", "request", &sealed).unwrap(),
            body
        );
        assert!(open("keyring-only-secret", "connection-one", "reply", &sealed).is_err());
        assert!(open("keyring-only-secret", "connection-two", "request", &sealed).is_err());
        assert!(open("different-key", "connection-one", "request", &sealed).is_err());
        let mut tampered = sealed.clone();
        tampered["nonce"] = json!("AAAA");
        assert!(open(
            "keyring-only-secret",
            "connection-one",
            "request",
            &tampered
        )
        .is_err());
    }
}
pub fn snapshot(app: &tauri::AppHandle) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
    expire_approvals(&mut s);
    archive_finished(app, &mut s);
    let grants: Vec<_> = s
        .config
        .grants
        .iter()
        .filter(|g| !g.revoked)
        .map(|g| {
            let mut v = serde_json::to_value(g).unwrap_or_default();
            v.as_object_mut().unwrap().remove("tokenHash");
            v
        })
        .collect();
    let mut jobs: Vec<_> = s.jobs.iter().rev().map(Job::desktop_view).collect();
    for previous in s.history.iter().rev() {
        if !jobs.iter().any(|j| j["id"] == previous["id"]) {
            jobs.push(previous.clone());
        }
    }
    jobs.truncate(128);
    Ok(
        json!({"enabled":s.config.enabled,"ready":s.port!=0&&s.error.is_none(),"error":s.error.as_ref().or(s.history_error.as_ref()),"grants":grants,"jobs":jobs,"remote":{"routes":s.config.remote_routes,"activeRoutes":s.remote_active,"lastConnected":s.remote_seen,"error":s.remote_error,"resource":format!("{}/mcp",option_env!("NODECLOAK_API_URL").unwrap_or("https://nodecloak.com").trim_end_matches('/'))}}),
    )
}
pub fn action(app: &tauri::AppHandle, action: Value) -> Result<Value, String> {
    let root = &app.state::<crate::data_root::DataRoot>().0;
    let runtime = app.state::<Runtime>();
    match action["kind"].as_str().unwrap_or("") {
        "remote_request" => {
            return super::remote::inspect(
                app,
                action["user_code"].as_str().ok_or("INVALID_ARGUMENTS")?,
            )
        }
        "remote_grant" => return super::remote::approve(app, &action),
        "grant_sync_permission" => {
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let id = action["id"]
                .as_str()
                .ok_or("INVALID_ARGUMENTS")?
                .to_string();
            let enabled = action["enabled"].as_bool().ok_or("INVALID_ARGUMENTS")?;
            if s.config
                .remote_routes
                .iter()
                .any(|r| r.local_grant_id == id)
            {
                return Err("REMOTE_REAUTHORIZATION_REQUIRED".into());
            }
            let g = s
                .config
                .grants
                .iter_mut()
                .find(|g| g.id == id && !g.revoked)
                .ok_or("GRANT_NOT_FOUND")?;
            let previous = g.permissions.clone();
            g.permissions.retain(|p| p != "sync");
            if enabled {
                g.permissions.push("sync".into())
            }
            if let Err(e) = save(root, &s.config) {
                s.config
                    .grants
                    .iter_mut()
                    .find(|g| g.id == id)
                    .unwrap()
                    .permissions = previous;
                return Err(e);
            }
            if !enabled {
                for j in &mut s.jobs {
                    if j.client == id && catalog::permission(&j.tool) == Some("sync") {
                        j.cancel.store(true, Ordering::SeqCst);
                        if matches!(j.state.as_str(), "queued" | "awaiting_approval") {
                            j.state = "cancelled".into();
                        }
                    }
                }
            }
        }
        "grant_confirmation" => {
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let id = action["id"].as_str().ok_or("INVALID_ARGUMENTS")?;
            let enabled = action["confirm"].as_bool().ok_or("INVALID_ARGUMENTS")?;
            let grant = s
                .config
                .grants
                .iter_mut()
                .find(|g| g.id == id && !g.revoked)
                .ok_or("GRANT_NOT_FOUND")?;
            let previous = grant.confirm_interactions;
            grant.confirm_interactions = enabled;
            if let Err(e) = save(root, &s.config) {
                s.config
                    .grants
                    .iter_mut()
                    .find(|g| g.id == id)
                    .unwrap()
                    .confirm_interactions = previous;
                return Err(e);
            }
        }
        "enabled" => {
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let enabled = action["enabled"].as_bool().ok_or("INVALID_ARGUMENTS")?;
            if enabled {
                if s.port == 0 {
                    return Err("MCP_NOT_READY".into());
                }
                secret_write(&format!("{}:endpoint", s.namespace), &s.port.to_string())?;
            }
            s.config.enabled = enabled;
            if !s.config.enabled {
                for j in &mut s.jobs {
                    j.cancel.store(true, Ordering::SeqCst);
                    if ["queued", "awaiting_approval"].contains(&j.state.as_str()) {
                        j.state = "cancelled".into();
                    }
                }
            }
            let saved = save(root, &s.config);
            drop(s);
            if !enabled {
                app.state::<crate::profiles::Runtime>()
                    .1
                    .invalidate_downloads(None);
            }
            saved?;
        }
        "grant" => {
            let mut g: Grant =
                serde_json::from_value(action["grant"].clone()).map_err(|_| "INVALID_GRANT")?;
            g.id = uuid::Uuid::new_v4().to_string();
            g.revoked = false;
            g.token_hash = String::new();
            if g.name.trim().is_empty()
                || g.name.len() > 100
                || g.profile_ids.len() > 200
                || g.origins.len() > 100
                || g.file_roots.len() > 10
                || g.expires_at <= now()
                || g.expires_at > now() + 31_536_000
            {
                return Err("INVALID_GRANT".into());
            }
            if !g.permissions.iter().any(|p| p == "read")
                || g.permissions.iter().any(|p| {
                    !["read", "manage", "page_read", "page_write", "files", "sync"]
                        .contains(&p.as_str())
                })
            {
                return Err("INVALID_PERMISSIONS".into());
            }
            let list = crate::profiles::load(root)?;
            for id in &g.profile_ids {
                if !list.iter().any(|p| &p.id == id) {
                    return Err("PROFILE_NOT_FOUND".into());
                }
            }
            for origin in &g.origins {
                let u = reqwest::Url::parse(origin).map_err(|_| "INVALID_ORIGIN")?;
                if !["http", "https"].contains(&u.scheme())
                    || u.origin().ascii_serialization() != *origin
                {
                    return Err("INVALID_ORIGIN".into());
                }
            }
            for dir in &mut g.file_roots {
                let p = std::fs::canonicalize(&*dir).map_err(|_| "FILE_DIRECTORY_NOT_FOUND")?;
                if !p.is_dir() {
                    return Err("FILE_DIRECTORY_NOT_FOUND".into());
                }
                *dir = p.to_string_lossy().into();
            }
            let token = crate::fingerprint::seed()?;
            g.token_hash = hash(&token);
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            if s.port == 0 {
                return Err("MCP_NOT_READY".into());
            }
            let deadline = now();
            for expired in s
                .config
                .grants
                .iter()
                .filter(|g| g.revoked || g.expires_at <= deadline)
            {
                secret_remove(&format!("{}:{}", s.namespace, expired.id));
            }
            s.config.grants.retain(|g| !g.revoked && g.expires_at > deadline);
            if s.config.grants.len() >= 100 {
                return Err("TOO_MANY_CLIENTS".into());
            }
            let name = format!("{}:{}", s.namespace, g.id);
            secret_write(&name, &token)?;
            s.config.grants.push(g);
            if let Err(e) = save(root, &s.config) {
                s.config.grants.pop();
                secret_remove(&name);
                return Err(e);
            }
        }
        "revoke" => {
            let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let id = action["id"].as_str().ok_or("INVALID_ARGUMENTS")?;
            let ns = s.namespace.clone();
            if let Some(g) = s.config.grants.iter_mut().find(|g| g.id == id) {
                g.revoked = true;
                secret_remove(&format!("{ns}:{id}"));
            }
            for j in &mut s.jobs {
                if j.client == id {
                    j.cancel.store(true, Ordering::SeqCst);
                    if ["queued", "awaiting_approval"].contains(&j.state.as_str()) {
                        j.state = "cancelled".into();
                    }
                }
            }
            let saved = save(root, &s.config);
            drop(s);
            app.state::<crate::profiles::Runtime>()
                .1
                .invalidate_downloads(Some(id));
            saved?;
        }
        "approve" | "cancel" => {
            let id = action["id"]
                .as_str()
                .ok_or("INVALID_ARGUMENTS")?
                .to_string();
            {
                let mut s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
                let j = s
                    .jobs
                    .iter_mut()
                    .find(|j| j.id == id)
                    .ok_or("JOB_NOT_FOUND")?;
                if action["kind"] == "cancel" {
                    j.cancel.store(true, Ordering::SeqCst);
                    if j.state == "awaiting_approval" {
                        j.state = "cancelled".into();
                    }
                } else {
                    if j.state != "awaiting_approval" || j.created + 600 < now() {
                        return Err("APPROVAL_EXPIRED".into());
                    }
                    j.state = "queued".into();
                }
            }
            if action["kind"] == "approve" {
                run_job(app.clone(), id);
            }
        }
        "configuration" => {
            let s = runtime.0.lock().map_err(|_| "MCP_LOCK_FAILED")?;
            let id = action["id"].as_str().ok_or("INVALID_ARGUMENTS")?;
            if !s.config.grants.iter().any(|g| g.id == id && !g.revoked) {
                return Err("GRANT_NOT_FOUND".into());
            }
            return Ok(
                json!({"mcpServers":{"nodecloak":{"command":crate::platform::current_app_executable()?,"args":["--mcp","--instance",s.namespace,"--client",id]}}}),
            );
        }
        _ => return Err("UNKNOWN_ACTION".into()),
    }
    approval_event(app);
    snapshot(app)
}

fn approval_event(app: &tauri::AppHandle) {
    if let Ok(s) = app.state::<Runtime>().0.lock() {
        let pending = s
            .jobs
            .iter()
            .filter(|j| j.state == "awaiting_approval")
            .count();
        let _ = app.emit("mcp-approval", json!({"pending":pending}));
    }
}

fn save_history(app: &tauri::AppHandle, history: &[Value]) -> Result<(), String> {
    let root = &app.state::<crate::data_root::DataRoot>().0;
    let path = root.join("mcp-history.json");
    crate::profiles::guard_tree(&path)?;
    let mut file = tempfile::NamedTempFile::new_in(root).map_err(|_| "MCP_HISTORY_WRITE_FAILED")?;
    serde_json::to_writer(&mut file, history).map_err(|_| "MCP_HISTORY_WRITE_FAILED")?;
    file.as_file_mut()
        .sync_all()
        .map_err(|_| "MCP_HISTORY_WRITE_FAILED")?;
    file.persist(path).map_err(|_| "MCP_HISTORY_WRITE_FAILED")?;
    Ok(())
}

fn archive_finished(app: &tauri::AppHandle, s: &mut State) {
    let records: Vec<_> = s
        .jobs
        .iter_mut()
        .filter(|j| ["completed", "failed", "cancelled"].contains(&j.state.as_str()) && !j.archived)
        .map(|j| {
            j.archived = true;
            j.audit_record()
        })
        .collect();
    if records.is_empty() && s.history_error.is_none() {
        return;
    }
    s.history.extend(records);
    if s.history.len() > 128 {
        s.history.drain(0..s.history.len() - 128);
    }
    match save_history(app, &s.history) {
        Ok(()) => s.history_error = None,
        Err(error) => s.history_error = Some(error),
    }
}
