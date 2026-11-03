use crate::{
    engine, platform,
    proxy::{self, ProxyConfig, ProxyTest, Relay, Upstream},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashMap,
    path::{Path, PathBuf},
    sync::Mutex,
};
pub(crate) mod reset;

#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "lowercase")]
pub enum PermissionMode {
    #[default]
    Ask,
    Block,
    Allow,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum DoNotTrack {
    #[default]
    Inherit,
    On,
    Off,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(rename_all = "camelCase")]
pub enum WebRtcPolicy {
    #[default]
    Inherit,
    Default,
    Public,
    Proxy,
}
#[derive(Clone, Serialize, Deserialize, PartialEq, Default)]
#[serde(default, rename_all = "camelCase")]
pub struct AdvancedSettings {
    pub notifications: PermissionMode,
    pub location: PermissionMode,
    pub camera: PermissionMode,
    pub microphone: PermissionMode,
    pub block_images: bool,
    pub block_webgl: bool,
    pub resist_fingerprinting: bool,
    pub do_not_track: DoNotTrack,
    pub web_rtc: WebRtcPolicy,
    pub block_webgpu: bool,
    pub block_speech: bool,
    pub block_battery: bool,
    pub canvas_protection: bool,
    pub hardware_concurrency: u8,
    pub viewport_width: u32,
    pub viewport_height: u32,
}

#[derive(Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "camelCase")]
pub struct Preferences {
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub fingerprint: Option<crate::fingerprint::Settings>,
    pub language: String,
    pub privacy: bool,
    pub font_restriction: bool,
    pub startup_url: String,
    #[serde(default)]
    pub advanced: AdvancedSettings,
    #[serde(default)]
    pub regional: crate::regional::RegionalSettings,
}
impl Default for Preferences {
    fn default() -> Self {
        Self {
            fingerprint: None,
            language: "en-US,en".into(),
            privacy: true,
            font_restriction: false,
            startup_url: "https://ip.nodecloak.com/".into(),
            advanced: AdvancedSettings::default(),
            regional: crate::regional::RegionalSettings::default(),
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Profile {
    pub id: String,
    pub name: String,
    pub browser: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub proxy: ProxyConfig,
    pub preferences: Preferences,
    pub created_at: String,
    pub updated_at: String,
    pub deleted_at: Option<String>,
    pub last_test: Option<ProxyTest>,
    pub legacy: bool,
    #[serde(default)]
    pub relay_port: u16,
    #[serde(default)]
    pub active_proxy: Option<ProxyConfig>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub active_proxy_endpoint: Option<proxy::ProxyEndpoint>,
    #[serde(default)]
    pub active_preferences: Option<Preferences>,
    #[serde(default)]
    pub preferences_managed: bool,
    #[serde(default)]
    pub preferences_dirty: bool,
    #[serde(default)]
    pub credential_refs: Vec<String>,
    #[serde(default)]
    pub last_region: Option<crate::regional::ResolvedRegion>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub received_share: Option<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ProfileView {
    #[serde(flatten)]
    pub profile: Profile,
    pub running: bool,
    pub pending_restart: bool,
    pub browser_available: bool,
    pub has_password: bool,
    pub proxy_ready: bool,
    pub regional_ready: bool,
    pub regional_error: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub sharing: Option<crate::cloud::sharing::View>,
}
#[derive(Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct Draft {
    pub name: String,
    pub browser: String,
    #[serde(default)]
    pub notes: String,
    #[serde(default)]
    pub tags: Vec<String>,
    pub proxy: ProxyConfig,
    #[serde(default)]
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
    #[serde(default)]
    pub preferences: Preferences,
}
#[derive(Serialize, Deserialize)]
struct Registry {
    schema: u32,
    profiles: Vec<Profile>,
}
pub struct Runtime(
    pub Mutex<HashMap<String, Relay>>,
    pub crate::browser_control::Runtime,
);
impl Runtime {
    pub fn proxy_endpoint(&self,id:&str)->Result<Option<proxy::ProxyEndpoint>,String> {
        Ok(self.0.lock().map_err(|_|"代理管理器不可用")?.get(id).and_then(|r|r.endpoint.clone()))
    }
}
impl Default for Runtime {
    fn default() -> Self {
        Self(
            Mutex::new(HashMap::new()),
            crate::browser_control::Runtime::default(),
        )
    }
}
fn stamp() -> String {
    chrono::Utc::now().to_rfc3339()
}
fn identifier() -> String {
    uuid::Uuid::new_v4().simple().to_string()
}
fn valid_id(id: &str) -> bool {
    id.len() == 32
        && id
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        || ["legacy-chrome", "legacy-edge", "legacy-firefox"].contains(&id)
}
fn existing_preferences(root: &Path, browser: &str) -> Preferences {
    let dir = root.join(format!("browser-{browser}"));
    let mut prefs = Preferences::default();
    prefs.privacy = false;
    if browser == "firefox" {
        if let Ok(snapshot) = crate::firefox::snapshot(&dir, "intl.accept_languages") {
            if let Some(language) = snapshot["user"]
                .as_str()
                .or_else(|| snapshot["runtime"].as_str())
            {
                prefs.language = language.into();
            }
        }
        prefs.font_restriction = crate::firefox::configured(&dir, "fonts").unwrap_or(false);
        prefs.privacy = crate::firefox::configured(&dir, "webrtc").unwrap_or(false);
        for (key, target) in [
            (
                "permissions.default.desktop-notification",
                &mut prefs.advanced.notifications,
            ),
            ("permissions.default.geo", &mut prefs.advanced.location),
            ("permissions.default.camera", &mut prefs.advanced.camera),
            (
                "permissions.default.microphone",
                &mut prefs.advanced.microphone,
            ),
        ] {
            if let Ok(value) = crate::firefox::snapshot(&dir, key) {
                if value["user"] == 2 || (value["user"].is_null() && value["runtime"] == 2) {
                    *target = PermissionMode::Block;
                }
            }
        }
        for (key, target, expected) in [
            (
                "permissions.default.image",
                &mut prefs.advanced.block_images,
                serde_json::json!(2),
            ),
            (
                "webgl.disabled",
                &mut prefs.advanced.block_webgl,
                serde_json::json!(true),
            ),
            (
                "privacy.resistFingerprinting",
                &mut prefs.advanced.resist_fingerprinting,
                serde_json::json!(true),
            ),
            (
                "privacy.fingerprintingProtection",
                &mut prefs.advanced.canvas_protection,
                serde_json::json!(true),
            ),
            (
                "dom.webgpu.enabled",
                &mut prefs.advanced.block_webgpu,
                serde_json::json!(false),
            ),
            (
                "media.webspeech.synth.enabled",
                &mut prefs.advanced.block_speech,
                serde_json::json!(false),
            ),
            (
                "dom.battery.enabled",
                &mut prefs.advanced.block_battery,
                serde_json::json!(false),
            ),
        ] {
            if let Ok(value) = crate::firefox::snapshot(&dir, key) {
                *target = value["user"] == expected
                    || (value["user"].is_null() && value["runtime"] == expected);
            }
        }
    } else if let Ok(bytes) = std::fs::read(dir.join("Default/Preferences")) {
        if let Ok(data) = serde_json::from_slice::<serde_json::Value>(&bytes) {
            if let Some(language) = data
                .pointer("/intl/accept_languages")
                .and_then(|v| v.as_str())
            {
                prefs.language = language.into();
            }
            prefs.privacy = data
                .pointer("/webrtc/ip_handling_policy")
                .and_then(|v| v.as_str())
                == Some("disable_non_proxied_udp");
            let settings = &data["profile"]["default_content_setting_values"];
            for (key, target) in [
                ("notifications", &mut prefs.advanced.notifications),
                ("geolocation", &mut prefs.advanced.location),
                ("media_stream_camera", &mut prefs.advanced.camera),
                ("media_stream_mic", &mut prefs.advanced.microphone),
            ] {
                if settings[key] == 2 {
                    *target = PermissionMode::Block;
                }
            }
            prefs.advanced.block_images = settings["images"] == 2;
        }
    }
    prefs
}
pub(crate) fn guard_tree(root: &Path) -> Result<(), String> {
    let mut path = PathBuf::new();
    for part in root.components() {
        path.push(part);
        if let Ok(info) = std::fs::symlink_metadata(&path) {
            if info.file_type().is_symlink() {
                return Err("配置路径包含链接，已停止操作".into());
            }
            #[cfg(windows)]
            {
                use std::os::windows::fs::MetadataExt;
                if info.file_attributes() & 0x400 != 0 {
                    return Err("配置路径包含重解析点，已停止操作".into());
                }
            }
        }
    }
    Ok(())
}
pub fn load(root: &Path) -> Result<Vec<Profile>, String> {
    let _reset=reset::lock()?;
    load_unlocked(root)
}
/// The expiry watchdog must not initialize profiles or wait for reset cleanup.
pub(crate) fn load_snapshot(root: &Path) -> Result<Vec<Profile>,String> { load_registry(root,false) }
fn load_unlocked(root: &Path) -> Result<Vec<Profile>, String> {
    load_registry(root,true)
}
fn load_registry(root: &Path,recover_reset: bool) -> Result<Vec<Profile>, String> {
    guard_tree(root)?;
    let file = root.join("profiles.json");
    guard_tree(&file)?;
    if !file.exists() {
        if !recover_reset {return Err("副本清单不存在".into());}
        let profiles = ["chrome", "edge", "firefox"]
            .into_iter()
            .map(|browser| Profile {
                id: format!("legacy-{browser}"),
                name: match browser {
                    "chrome" => "Chrome",
                    "edge" => "Edge",
                    _ => "Firefox",
                }
                .into(),
                browser: browser.into(),
                notes: String::new(),
                tags: vec!["默认".into()],
                proxy: ProxyConfig::system(),
                preferences: existing_preferences(root, browser),
                created_at: stamp(),
                updated_at: stamp(),
                deleted_at: None,
                last_test: None,
                legacy: true,
                relay_port: 0,
                active_proxy: None,
                active_proxy_endpoint: None,
                active_preferences: None,
                preferences_managed: false,
                preferences_dirty: false,
                credential_refs: vec![],
                last_region: None,
                received_share: None,
            })
            .collect::<Vec<_>>();
        save(root, &profiles)?;
        return Ok(profiles);
    }
    if std::fs::metadata(&file)
        .map_err(|_| "无法读取副本清单")?
        .len()
        > 4 * 1024 * 1024
    {
        return Err("副本清单过大".into());
    }
    let mut registry: Registry =
        serde_json::from_slice(&std::fs::read(&file).map_err(|_| "无法读取副本清单")?)
            .map_err(|_| "副本清单损坏；未覆盖原数据，请恢复备份")?;
    if registry.schema != 1 {
        return Err("副本清单版本不兼容".into());
    }
    let mut unique = std::collections::HashSet::new();
    for profile in &registry.profiles {
        if !valid_id(&profile.id)
            || !unique.insert(&profile.id)
            || !["chrome", "edge", "firefox"].contains(&profile.browser.as_str())
            || profile.legacy != profile.id.starts_with("legacy-")
            || (profile.legacy && profile.id != format!("legacy-{}", profile.browser))
        {
            return Err("副本清单包含无效或重复条目".into());
        }
    }
    if recover_reset {reset::recover(root, &mut registry.profiles)?;}
    for profile in &mut registry.profiles {
        let browser_name = match profile.browser.as_str() {
            "chrome" => "Chrome",
            "edge" => "Edge",
            _ => "Firefox",
        };
        if profile.legacy && profile.name == format!("默认 {browser_name}") {
            profile.name = browser_name.into();
        }
        if profile.legacy && profile.notes == "从原专用浏览器接入，原有数据保留" {
            profile.notes.clear();
        }
        if crate::regional::normalize_sources(&mut profile.preferences) {
            profile.preferences_dirty = true;
            profile.last_region = None;
        }
    }
    Ok(registry.profiles)
}
fn save(root: &Path, profiles: &[Profile]) -> Result<(), String> {
    guard_tree(root)?;
    let bytes = serde_json::to_vec_pretty(&Registry {
        schema: 1,
        profiles: profiles.to_vec(),
    })
    .map_err(|_| "无法保存副本清单")?;
    if bytes.len() > 4 * 1024 * 1024 {
        return Err("副本清单已达大小限制，请减少备注或清理最近删除".into());
    }
    engine::atomic_write(&root.join("profiles.json"), &bytes)
}
pub fn get(root: &Path, id: &str, deleted: bool) -> Result<Profile, String> {
    if !valid_id(id) {
        return Err("副本 ID 无效".into());
    }
    load(root)?
        .into_iter()
        .find(|p| p.id == id && (deleted || p.deleted_at.is_none()))
        .ok_or_else(|| "副本不存在或已删除".into())
}
pub fn base(root: &Path, p: &Profile) -> Result<PathBuf, String> {
    let base = if p.legacy {
        root.to_path_buf()
    } else {
        root.join("profiles").join(&p.id)
    };
    guard_tree(&base)?;
    Ok(base)
}
pub fn path(root: &Path, p: &Profile) -> Result<PathBuf, String> {
    let path = base(root, p)?.join(format!("browser-{}", p.browser));
    guard_tree(&path)?;
    Ok(path)
}
pub fn context(root: &Path, browser: &str, id: Option<&str>) -> Result<PathBuf, String> {
    if let Some(id) = id {
        crate::cloud::sharing::editable(root, id)?;
        let p = get(root, id, false)?;
        if p.browser != browser {
            return Err("副本浏览器类型不匹配".into());
        }
        base(root, &p)
    } else {
        Ok(root.to_path_buf())
    }
}
pub fn running(root: &Path, p: &Profile) -> Result<bool, String> {
    Ok(!platform::profile_process_ids(&path(root, p)?)?.is_empty())
}
pub fn ensure_all_closed(root: &Path) -> Result<(), String> {
    for p in load(root)?.iter().filter(|p| p.deleted_at.is_none()) {
        platform::ensure_profile_closed(&path(root, p)?)?;
    }
    Ok(())
}
fn secret_entry(id: &str, reference: &str) -> Result<keyring::Entry, String> {
    if !valid_id(id) || reference.len() != 32 || !reference.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("代理凭据标识无效".into());
    }
    keyring::Entry::new("com.claudedone.proxy", &format!("{id}:{reference}"))
        .map_err(|_| "系统凭据存储不可用".to_owned())
}
fn password(id: &str, config: &ProxyConfig) -> Result<String, String> {
    match &config.credential_ref {
        Some(reference) => secret_entry(id, reference)?
            .get_password()
            .map_err(|_| "无法读取代理密码，请重新填写或解锁系统钥匙串".into()),
        None => Ok(String::new()),
    }
}
struct PendingCredentials {
    id: String,
    references: Vec<String>,
    committed: bool,
}
impl Drop for PendingCredentials {
    fn drop(&mut self) {
        if !self.committed {
            for reference in &self.references {
                if let Ok(entry) = secret_entry(&self.id, reference) {
                    let _ = entry.delete_credential();
                }
            }
        }
    }
}
pub fn upstream(p: &Profile, active: bool) -> Result<Upstream, String> {
    let config = if active {
        p.active_proxy.as_ref().unwrap_or(&p.proxy)
    } else {
        &p.proxy
    };
    let mut up=Upstream {
        config: config.clone(),
        password: if config.has_secret() {
            password(&p.id, config)?
        } else {
            String::new()
        },
    };
    if active && config.api_url.is_some() {
        let endpoint=p.active_proxy_endpoint.as_ref().ok_or("运行中的 API 代理地址未保存，请关闭并重新启动副本")?;
        endpoint.validate()?;
        up.config.host=endpoint.host.clone();up.config.port=endpoint.port;up.config.api_url=None;
    }
    Ok(up)
}
// A stored proxy secret belongs to one authentication destination. Callers may
// change profile settings without knowing it, but must explicitly supply a new
// password when changing the proxy protocol, address, port, or account.
fn same_proxy_credentials(previous: &ProxyConfig, next: &ProxyConfig) -> bool {
    previous.custom()
        && next.custom()
        && next.has_secret()
        && previous.mode == next.mode
        && previous.host.eq_ignore_ascii_case(&next.host)
        && previous.port == next.port
        && previous.username == next.username
        && previous.api_url == next.api_url
        && previous.node == next.node
}
pub fn draft_upstream(root: &Path, id: Option<&str>, draft: &Draft) -> Result<Upstream, String> {
    if let Some(id) = id {
        crate::cloud::sharing::editable(root, id)?;
    }
    let mut normalized = draft.clone();
    crate::regional::normalize_sources(&mut normalized.preferences);
    let draft = &normalized;
    validate(draft)?;
    let old = id.map(|id| get(root, id, false)).transpose()?;
    let password = match &draft.password {
        _ if !draft.proxy.has_secret() => String::new(),
        Some(value) if !value.is_empty() => value.clone(),
        _ if !draft.clear_password => old
            .as_ref()
            .filter(|p| same_proxy_credentials(&p.proxy, &draft.proxy))
            .map(|p| password(&p.id, &p.proxy))
            .transpose()?
            .unwrap_or_default(),
        _ => String::new(),
    };
    let mut config = draft.proxy.clone();
    config.credential_ref = None;
    let up = Upstream { config, password };
    up.validate()?;
    Ok(up)
}
pub(crate) fn validate(draft: &Draft) -> Result<(), String> {
    crate::regional::validate(&draft.preferences)?;
    let advanced = &draft.preferences.advanced;
    if ![0, 2, 4, 8, 16].contains(&advanced.hardware_concurrency)
        || ((advanced.viewport_width != 0 || advanced.viewport_height != 0)
            && (!(320..=3840).contains(&advanced.viewport_width)
                || !(200..=2160).contains(&advanced.viewport_height)))
    {
        return Err("硬件提示或页面视口尺寸无效".into());
    }
    if draft.browser != "firefox"
        && (advanced.resist_fingerprinting
            || advanced.canvas_protection
            || advanced.block_battery
            || draft.preferences.font_restriction)
    {
        return Err("字体限制、Canvas 保护、电池保护和严格指纹保护仅适用于 Firefox".into());
    }
    if draft.name.trim().is_empty()
        || draft.name.chars().count() > 60
        || draft.notes.chars().count() > 1000
        || draft.tags.len() > 10
        || draft
            .tags
            .iter()
            .any(|s| s.trim().is_empty() || s.chars().count() > 24)
        || !["chrome", "edge", "firefox"].contains(&draft.browser.as_str())
    {
        return Err("名称、备注、标签或浏览器类型无效".into());
    }
    if draft.preferences.language.len() > 160
        || draft
            .preferences
            .language
            .split(',')
            .any(|s| s.is_empty() || !s.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-'))
    {
        return Err("浏览器语言格式无效，例如 en-US,en".into());
    }
    let url = &draft.preferences.startup_url;
    if url.len() > 4096 {
        return Err("启动页面地址过长，最多 4096 字节".into());
    }
    if url != "about:blank" {
        let parsed = reqwest::Url::parse(url).map_err(|_| "启动页面地址无效")?;
        if !["https", "http"].contains(&parsed.scheme())
            || parsed.host_str().is_none()
            || !parsed.username().is_empty()
            || parsed.password().is_some()
        {
            return Err("启动页面只支持 HTTP、HTTPS 或 about:blank".into());
        }
    }
    draft.proxy.validate()
}
pub fn create(root: &Path, draft: Draft, copy_id: Option<&str>) -> Result<Profile, String> {
    if let Some(id) = copy_id {
        crate::cloud::sharing::editable(root, id)?;
    }
    create_with_id(root, draft, copy_id, identifier(), false)
}
pub(crate) fn create_synced(root: &Path, draft: Draft, id: &str) -> Result<Profile, String> {
    if !valid_id(id) || id.starts_with("legacy-") || load(root)?.iter().any(|p| p.id == id) {
        return Err("同步副本标识冲突".into());
    }
    create_with_id(root, draft, None, id.into(), true)
}
fn create_with_id(
    root: &Path,
    mut draft: Draft,
    copy_id: Option<&str>,
    id: String,
    preserve_fingerprint: bool,
) -> Result<Profile, String> {
    crate::regional::normalize_sources(&mut draft.preferences);
    validate(&draft)?;
    let mut profiles = load(root)?;
    if !preserve_fingerprint {
        let mut fingerprint = match &draft.preferences.fingerprint {
            Some(f) => f.clone(),
            None => crate::fingerprint::fresh()?,
        };
        if profiles.iter().any(|p| {
            p.preferences
                .fingerprint
                .as_ref()
                .is_some_and(|f| f.seed == fingerprint.seed)
        }) {
            fingerprint.seed = crate::fingerprint::seed()?;
        }
        // Copying settings must preserve the source's native login mode. A
        // legacy source has no stable fingerprint; allocate a seed without
        // enabling automation. Explicit choices in the draft still take effect.
        if draft.preferences.fingerprint.is_none()
            && (copy_id.is_some()
                || draft.preferences.advanced.resist_fingerprinting
                || draft.preferences.advanced.canvas_protection)
        {
            fingerprint.enabled = false;
        }
        draft.preferences.fingerprint = Some(fingerprint);
    }
    if profiles.len() >= 500 {
        return Err("副本数量已达 500，请先清理最近删除".into());
    }
    let mut pending = PendingCredentials {
        id: id.clone(),
        references: vec![],
        committed: false,
    };
    let mut proxy = draft.proxy.clone();
    proxy.credential_ref = None;
    let mut refs = vec![];
    if let Some(value) = draft.password.filter(|s| !s.is_empty()) {
        Upstream {
            config: proxy.clone(),
            password: value.clone(),
        }
        .validate()?;
        let reference = identifier();
        secret_entry(&id, &reference)?
            .set_password(&value)
            .map_err(|_| "代理密码无法保存到系统凭据存储")?;
        proxy.credential_ref = Some(reference.clone());
        pending.references.push(reference.clone());
        refs.push(reference);
    }
    if let Some(copy) = copy_id {
        let original = get(root, copy, false)?;
        if original.proxy.credential_ref.is_some() {
            if proxy.credential_ref.is_none()
                && !draft.clear_password
                && same_proxy_credentials(&original.proxy, &proxy)
            {
                let value = password(&original.id, &original.proxy)?;
                let new = identifier();
                secret_entry(&id, &new)?
                    .set_password(&value)
                    .map_err(|_| "无法复制代理凭据")?;
                proxy.credential_ref = Some(new.clone());
                pending.references.push(new.clone());
                refs.push(new);
            }
        }
    }
    let mut p = Profile {
        id,
        name: draft.name.trim().into(),
        browser: draft.browser,
        notes: draft.notes,
        tags: draft.tags,
        proxy,
        preferences: draft.preferences,
        created_at: stamp(),
        updated_at: stamp(),
        deleted_at: None,
        last_test: None,
        legacy: false,
        relay_port: 0,
        active_proxy: None,
        active_proxy_endpoint: None,
        active_preferences: None,
        preferences_managed: true,
        preferences_dirty: true,
        credential_refs: refs,
        last_region: None,
        received_share: None,
    };
    if crate::proxy::node::is_node(&p.proxy.mode) {upstream(&p,false)?.validate()?;}
    apply_preferences(root, &p)?;
    p.preferences_dirty = false;
    profiles.push(p.clone());
    save(root, &profiles)?;
    pending.committed = true;
    Ok(p)
}
pub fn update(root: &Path, id: &str, mut draft: Draft) -> Result<Profile, String> {
    crate::cloud::sharing::editable(root, id)?;
    crate::regional::normalize_sources(&mut draft.preferences);
    validate(&draft)?;
    let mut pending = PendingCredentials {
        id: id.into(),
        references: vec![],
        committed: false,
    };
    let mut profiles = load(root)?;
    let p = profiles
        .iter_mut()
        .find(|p| p.id == id && p.deleted_at.is_none())
        .ok_or("副本不存在")?;
    if p.browser != draft.browser {
        return Err("已创建的副本不能更换浏览器类型，请复制配置新建".into());
    }
    let old = p.proxy.clone();
    let mut proxy = draft.proxy;
    proxy.credential_ref = if draft.clear_password || !same_proxy_credentials(&old, &proxy) {
        None
    } else {
        old.credential_ref
    };
    if let Some(value) = draft.password.filter(|s| !s.is_empty()) {
        Upstream {
            config: proxy.clone(),
            password: value.clone(),
        }
        .validate()?;
        let reference = identifier();
        secret_entry(id, &reference)?
            .set_password(&value)
            .map_err(|_| "代理密码无法保存到系统凭据存储")?;
        proxy.credential_ref = Some(reference.clone());
        pending.references.push(reference.clone());
        p.credential_refs.push(reference);
    }
    if crate::proxy::node::is_node(&proxy.mode) {Upstream{password:password(id,&proxy)?,config:proxy.clone()}.validate()?;}
    p.preferences_dirty = p.preferences_dirty || p.preferences != draft.preferences;
    p.preferences_managed = p.preferences_managed || p.preferences != draft.preferences;
    p.name = draft.name.trim().into();
    p.notes = draft.notes;
    p.tags = draft.tags;
    p.proxy = proxy;
    p.preferences = draft.preferences;
    p.updated_at = stamp();
    p.last_test = None;
    let saved = p.clone();
    save(root, &profiles)?;
    pending.committed = true;
    Ok(saved)
}
pub(crate) fn mark_shared(root: &Path, id: &str, grant: &str) -> Result<(), String> {
    let mut all = load(root)?;
    let p = all
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("共享副本不存在")?;
    p.received_share = Some(grant.into());
    save(root, &all)
}
pub fn apply_preferences(root: &Path, p: &Profile) -> Result<(), String> {
    apply_preferences_for_control(root,p,false)
}
pub(crate) fn apply_preferences_for_control(root: &Path, p: &Profile, force_control:bool) -> Result<(), String> {
    let dir = path(root, p)?;
    platform::ensure_profile_closed(&dir)?;
    std::fs::create_dir_all(&dir).map_err(|_| "无法创建副本目录")?;
    if p.preferences_managed {
        engine::atomic_write(
            &dir.join("nodecloak-runtime-settings.json"),
            &serde_json::to_vec(&p.preferences.advanced).map_err(|_| "高级设置格式无效")?,
        )?;
    }
    if p.browser == "firefox" {
        crate::firefox::prepare_stable_canvas(
            &dir,
            p.preferences
                .fingerprint
                .as_ref()
                .is_some_and(|f| f.enabled),
        )?;
        let mut changes = vec![];
        let mut fields = vec![];
        if p.preferences_managed && p.preferences_dirty {
            fields.extend([
                (
                    "intl.accept_languages",
                    serde_json::json!(p.preferences.language),
                ),
                (
                    "intl.locale.requested",
                    serde_json::json!(p.preferences.language.split(',').next().unwrap_or("en-US")),
                ),
                (
                    "intl.regional_prefs.use_os_locales",
                    serde_json::json!(false),
                ),
                (
                    "privacy.globalprivacycontrol.enabled",
                    serde_json::json!(p.preferences.privacy),
                ),
                (
                    "privacy.globalprivacycontrol.functionality.enabled",
                    serde_json::json!(p.preferences.privacy),
                ),
                (
                    "font.system.whitelist",
                    if p.preferences.font_restriction {
                        serde_json::json!(crate::firefox::FONT_LIST)
                    } else {
                        serde_json::Value::Null
                    },
                ),
            ]);
            let advanced = &p.preferences.advanced;
            for (key, mode) in [
                (
                    "permissions.default.desktop-notification",
                    &advanced.notifications,
                ),
                ("permissions.default.geo", &advanced.location),
                ("permissions.default.camera", &advanced.camera),
                ("permissions.default.microphone", &advanced.microphone),
            ] {
                fields.push((
                    key,
                    serde_json::json!(match mode {
                        PermissionMode::Block => 2,
                        PermissionMode::Allow => 1,
                        _ => 0,
                    }),
                ));
            }
            fields.extend([
                (
                    "geo.enabled",
                    serde_json::json!(advanced.location != PermissionMode::Block),
                ),
                (
                    "permissions.default.image",
                    serde_json::json!(if advanced.block_images { 2 } else { 1 }),
                ),
                ("webgl.disabled", serde_json::json!(advanced.block_webgl)),
                (
                    "privacy.resistFingerprinting",
                    serde_json::json!(advanced.resist_fingerprinting),
                ),
                (
                    "privacy.donottrackheader.enabled",
                    serde_json::json!(match advanced.do_not_track {
                        DoNotTrack::On => true,
                        DoNotTrack::Off => false,
                        _ => p.preferences.privacy,
                    }),
                ),
                (
                    "dom.webgpu.enabled",
                    if advanced.block_webgpu {
                        serde_json::json!(false)
                    } else {
                        serde_json::Value::Null
                    },
                ),
                (
                    "media.webspeech.synth.enabled",
                    if advanced.block_speech {
                        serde_json::json!(false)
                    } else {
                        serde_json::Value::Null
                    },
                ),
                (
                    "dom.battery.enabled",
                    if advanced.block_battery {
                        serde_json::json!(false)
                    } else {
                        serde_json::Value::Null
                    },
                ),
                (
                    "privacy.fingerprintingProtection",
                    if advanced.canvas_protection {
                        serde_json::json!(true)
                    } else if p
                        .preferences
                        .fingerprint
                        .as_ref()
                        .is_some_and(|f| f.enabled)
                    {
                        serde_json::json!(false)
                    } else {
                        serde_json::Value::Null
                    },
                ),
                (
                    "dom.maxHardwareConcurrency",
                    if advanced.hardware_concurrency > 0 {
                        serde_json::json!(advanced.hardware_concurrency)
                    } else {
                        serde_json::Value::Null
                    },
                ),
            ]);
            let rtc = &advanced.web_rtc;
            fields.push((
                "privacy.fingerprintingProtection.pbmode",
                if p.preferences
                    .fingerprint
                    .as_ref()
                    .is_some_and(|f| f.enabled)
                {
                    serde_json::json!(false)
                } else {
                    serde_json::Value::Null
                },
            ));
            fields.push((
                "media.peerconnection.enabled",
                serde_json::json!(
                    !p.proxy.custom() && (*rtc != WebRtcPolicy::Inherit || !p.preferences.privacy)
                ),
            ));
            for (key, value) in [
                (
                    "media.peerconnection.ice.default_address_only",
                    matches!(rtc, WebRtcPolicy::Public | WebRtcPolicy::Proxy),
                ),
                (
                    "media.peerconnection.ice.no_host",
                    matches!(rtc, WebRtcPolicy::Public | WebRtcPolicy::Proxy),
                ),
                (
                    "media.peerconnection.ice.proxy_only",
                    *rtc == WebRtcPolicy::Proxy,
                ),
            ] {
                fields.push((key, serde_json::json!(value)));
            }
            if force_control || crate::regional::needs_control_for(&p.browser, &p.preferences) {
                fields.push(("remote.prefs.recommended", serde_json::json!(false)));
            }
        }
        if force_control {fields.push(("remote.prefs.recommended", serde_json::json!(false)));}
        fields.push(("dom.file.createInChild", if force_control { serde_json::json!(true) } else { serde_json::Value::Null }));
        // We keep Firefox recommended automation prefs off to preserve profile privacy
        // settings. Apply only its first-run notification override: otherwise the chrome
        // onboarding modal blocks tab activation even though BiDi can read the page.
        // This does not record acceptance and is removed for an ordinary profile launch.
        fields.push(("termsofuse.bypassNotification", if force_control { serde_json::json!(true) } else { serde_json::Value::Null }));
        if p.proxy.custom() && !p.preferences_dirty {
            fields.push(("media.peerconnection.enabled", serde_json::json!(false)));
        }
        for (key, value) in fields {
            changes.push((
                key.into(),
                crate::firefox::snapshot(&dir, key)?,
                serde_json::json!({"user":value,"runtime":value}),
            ));
        }
        crate::firefox::apply(&dir, &changes, false)?;
    } else {
        let file = dir.join("Default/Preferences");
        let mut data: serde_json::Value = if file.exists() {
            serde_json::from_slice(&std::fs::read(&file).map_err(|_| "无法读取副本设置")?)
                .map_err(|_| "副本设置损坏，未覆盖")?
        } else {
            serde_json::json!({})
        };
        if !data.is_object() {
            return Err("副本设置格式无效".into());
        }
        if force_control || crate::regional::needs_control_for(&p.browser, &p.preferences) {
            if data.get("session").is_none() {
                data["session"] = serde_json::json!({});
            }
            if !data["session"].is_object() {
                return Err("浏览器启动字段结构无效，未覆盖".into());
            }
            data["session"]["restore_on_startup"] = serde_json::json!(5);
        }
        if p.preferences_managed && p.preferences_dirty {
            if data.get("intl").is_none() {
                data["intl"] = serde_json::json!({});
            }
            if !data["intl"].is_object() {
                return Err("浏览器语言字段结构无效，未覆盖".into());
            }
            data["intl"]["accept_languages"] = serde_json::json!(p.preferences.language);
            data["intl"]["selected_languages"] = serde_json::json!(p.preferences.language);
            // Preserve per-site exceptions, cookies and unrelated browser preferences.
            if data.get("profile").is_none() {
                data["profile"] = serde_json::json!({});
            }
            if !data["profile"].is_object() {
                return Err("浏览器网站权限字段结构无效，未覆盖".into());
            }
            if data["profile"]
                .get("default_content_setting_values")
                .is_none()
            {
                data["profile"]["default_content_setting_values"] = serde_json::json!({});
            }
            if !data["profile"]["default_content_setting_values"].is_object() {
                return Err("浏览器默认权限字段结构无效，未覆盖".into());
            }
            let advanced = &p.preferences.advanced;
            let values = &mut data["profile"]["default_content_setting_values"];
            for (key, mode) in [
                ("notifications", &advanced.notifications),
                ("geolocation", &advanced.location),
                ("media_stream_camera", &advanced.camera),
                ("media_stream_mic", &advanced.microphone),
            ] {
                values[key] = serde_json::json!(match mode {
                    PermissionMode::Block => 2,
                    PermissionMode::Allow => 1,
                    _ => 3,
                });
            }
            values["images"] = serde_json::json!(if advanced.block_images { 2 } else { 1 });
            if advanced.location == PermissionMode::Block {
                if let Some(exceptions) =
                    data.pointer_mut("/profile/content_settings/exceptions/geolocation")
                {
                    let sites = exceptions
                        .as_object_mut()
                        .ok_or("浏览器定位授权结构无效，未覆盖")?;
                    sites.retain(|_, entry| entry["setting"] != 1);
                }
            }
        }
        if (p.preferences_managed && p.preferences_dirty) || p.proxy.custom() {
            if data.get("webrtc").is_none() {
                data["webrtc"] = serde_json::json!({});
            }
            if !data["webrtc"].is_object() {
                return Err("浏览器隐私字段结构无效，未覆盖".into());
            }
            data["webrtc"]["ip_handling_policy"] = serde_json::json!(if p.proxy.custom() {
                "disable_non_proxied_udp"
            } else {
                match p.preferences.advanced.web_rtc {
                    WebRtcPolicy::Proxy => "disable_non_proxied_udp",
                    WebRtcPolicy::Public => "default_public_interface_only",
                    WebRtcPolicy::Default => "default",
                    WebRtcPolicy::Inherit => {
                        if p.preferences.privacy {
                            "disable_non_proxied_udp"
                        } else {
                            "default"
                        }
                    }
                }
            });
            data["enable_do_not_track"] =
                serde_json::json!(match p.preferences.advanced.do_not_track {
                    DoNotTrack::On => true,
                    DoNotTrack::Off => false,
                    _ => p.preferences.privacy,
                });
        }
        engine::atomic_write(
            &file,
            &serde_json::to_vec_pretty(&data).map_err(|_| "无法保存副本设置")?,
        )?;
    }
    Ok(())
}
pub fn remove(root: &Path, id: &str, purge: bool, runtime: &Runtime) -> Result<(), String> {
    let mut profiles = load(root)?;
    let index = profiles
        .iter()
        .position(|p| p.id == id)
        .ok_or("副本不存在")?;
    let p = &profiles[index];
    platform::ensure_profile_closed(&path(root, p)?)?;
    runtime.0.lock().map_err(|_| "代理管理器不可用")?.remove(id);
    runtime.1.stop(id);
    if purge {
        if p.deleted_at.is_none() {
            return Err("请先将副本移到最近删除，再彻底删除".into());
        }
        crate::cloud::sync::ensure_purge_synced(root, id)?;
        for reference in &p.credential_refs {
            match secret_entry(id, reference)?.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => {}
                Err(_) => return Err("无法清理系统代理凭据，请重试".into()),
            }
        }
        let target = if p.legacy {
            path(root, p)?
        } else {
            base(root, p)?
        };
        guard_tree(&target)?;
        if !target.starts_with(root) || target == root {
            return Err("删除目标超出副本目录".into());
        }
        if p.legacy {
            let mut records = engine::history(root)?;
            records.retain(|r| !local_record(r) || r.browser != p.browser);
            engine::save_history(root, &records)?;
        }
        if target.exists() {
            std::fs::remove_dir_all(&target).map_err(|_| "无法删除副本数据，请检查目录权限")?;
        }
        profiles.remove(index);
    } else {
        profiles[index].deleted_at = Some(stamp());
        profiles[index].active_proxy = None;
        profiles[index].active_proxy_endpoint = None;
    }
    save(root, &profiles)
}
pub fn restore(root: &Path, id: &str) -> Result<(), String> {
    let mut profiles = load(root)?;
    let p = profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("副本不存在")?;
    p.deleted_at = None;
    p.updated_at = stamp();
    save(root, &profiles)
}
pub fn views(root: &Path, runtime: &Runtime) -> Result<Vec<ProfileView>, String> {
    let processes = platform::browser_processes();
    let mut profiles = load(root)?;
    let mut changed = false;
    let mut relays = runtime.0.lock().map_err(|_| "代理管理器不可用")?;
    let mut output = vec![];
    for p in &mut profiles {
        if !crate::cloud::sharing::visible(root, &p.id)? {
            continue;
        }
        let running = platform::profile_ids_from(&processes, &path(root, p)?).len() > 0;
        if !running && (p.active_proxy.is_some() || p.active_proxy_endpoint.is_some()) {
            p.active_proxy = None;
            p.active_proxy_endpoint = None;
            p.active_preferences = None;
            relays.remove(&p.id);
            changed = true;
        }
        let pending = running
            && (p
                .active_proxy
                .as_ref()
                .is_some_and(|active| active != &p.proxy)
                || p.active_preferences
                    .as_ref()
                    .is_some_and(|active| active != &p.preferences));
        output.push(ProfileView {
            profile: p.clone(),
            running,
            pending_restart: pending,
            browser_available: platform::browser_path(&p.browser).is_ok(),
            has_password: p.proxy.credential_ref.is_some(),
            proxy_ready: !p.active_proxy.as_ref().unwrap_or(&p.proxy).custom()
                || relays.get(&p.id).is_some_and(|relay|relay.ready()),
            regional_ready: !crate::regional::manual_needs_control_for(
                &p.browser,
                p.active_preferences.as_ref().unwrap_or(&p.preferences),
            ) || runtime.1.ready(&p.id),
            regional_error: runtime.1.error(&p.id),
            sharing: crate::cloud::sharing::view(root, &p.id)?,
        });
    }
    drop(relays);
    if changed {
        save(root, &profiles)?;
    }
    Ok(output)
}
pub async fn prepare_relay(
    _root: &Path,
    p: &Profile,
    runtime: &Runtime,
    recover: bool,
) -> Result<Option<u16>, String> {
    // Fresh launches must not reuse a relay left by an externally closed browser.
    // Recovering a live browser preserves its active route until an explicit restart.
    if !recover {
        runtime
            .0
            .lock()
            .map_err(|_| "代理管理器不可用")?
            .remove(&p.id);
    }
    let config = if recover {
        p.active_proxy.as_ref().unwrap_or(&p.proxy)
    } else {
        &p.proxy
    };
    if !config.custom() {
        return Ok(None);
    }
    if let Some(relay) = runtime.0.lock().map_err(|_| "代理管理器不可用")?.get(&p.id) {
        return Ok(Some(relay.port));
    }
    let relay = proxy::start(
        upstream(p, recover)?,
        if recover { p.relay_port } else { 0 },
    )
    .await?;
    let port = relay.port;
    runtime
        .0
        .lock()
        .map_err(|_| "代理管理器不可用")?
        .insert(p.id.clone(), relay);
    Ok(Some(port))
}
pub fn mark_started(root: &Path, id: &str, port: Option<u16>, endpoint: Option<proxy::ProxyEndpoint>) -> Result<(), String> {
    let mut profiles = load(root)?;
    let p = profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("副本不存在")?;
    p.active_proxy_endpoint=if p.proxy.api_url.is_some() {
        let endpoint=endpoint.ok_or("API 代理尚未提取有效地址")?;endpoint.validate()?;Some(endpoint)
    }else{None};
    p.active_proxy = Some(p.proxy.clone());
    p.preferences_dirty = false;
    p.active_preferences = Some(p.preferences.clone());
    p.relay_port = port.unwrap_or(0);
    p.updated_at = stamp();
    save(root, &profiles)
}
pub fn remember_region(
    root: &Path,
    id: &str,
    region: crate::regional::ResolvedRegion,
) -> Result<(), String> {
    let mut list = load(root)?;
    let p = list.iter_mut().find(|p| p.id == id).ok_or("副本不存在")?;
    p.last_region = Some(region);
    save(root, &list)
}
pub fn remember_test(root: &Path, id: &str, result: ProxyTest) -> Result<(), String> {
    let mut profiles = load(root)?;
    let p = profiles
        .iter_mut()
        .find(|p| p.id == id)
        .ok_or("副本不存在")?;
    p.last_test = Some(result);
    save(root, &profiles)
}
fn local_record(record: &engine::Record) -> bool {
    !["timezone", "cli"].contains(&record.item_id.as_str())
        && !(record.item_id == "fonts" && record.browser != "firefox")
        && !record.changes.iter().any(|c| c.target == "userFont")
}
pub fn sync_preferences(root: &Path, id: &str) -> Result<(), String> {
    let mut list = load(root)?;
    let p = list
        .iter_mut()
        .find(|p| p.id == id && p.deleted_at.is_none())
        .ok_or("副本不存在")?;
    let detected = existing_preferences(&base(root, p)?, &p.browser);
    p.preferences.language = detected.language;
    p.preferences.privacy = detected.privacy;
    p.preferences.font_restriction = detected.font_restriction;
    p.preferences_dirty = false;
    p.updated_at = stamp();
    save(root, &list)
}
pub fn apply_pending_preferences(root: &Path, id: &str) -> Result<(), String> {
    let p = get(root, id, false)?;
    if p.preferences_dirty {
        apply_preferences(root, &p)?;
        let mut list = load(root)?;
        let p = list.iter_mut().find(|p| p.id == id).ok_or("副本不存在")?;
        p.preferences_dirty = false;
        save(root, &list)?;
    }
    Ok(())
}
pub fn history(root: &Path, id: Option<&str>) -> Result<Vec<engine::Record>, String> {
    let list = load(root)?;
    let mut records = engine::history(root)?;
    records.retain(|r| {
        !local_record(r)
            || list
                .iter()
                .any(|p| p.legacy && p.browser == r.browser && p.deleted_at.is_none())
    });
    for record in &mut records {
        if local_record(record) {
            record.profile_id = Some(format!("legacy-{}", record.browser));
        }
    }
    for p in list.iter().filter(|p| !p.legacy && p.deleted_at.is_none()) {
        let mut local = engine::history(&base(root, p)?)?;
        for record in &mut local {
            record.profile_id = Some(p.id.clone());
        }
        records.extend(local);
    }
    if let Some(id) = id {
        get(root, id, false)?;
        records.retain(|record| record.profile_id.as_deref() == Some(id));
    }
    records.sort_by(|a, b| b.created_at.cmp(&a.created_at));
    Ok(records)
}

#[cfg(test)]
mod tests {
    #[test]
    fn copying_legacy_defaults_keeps_native_login_mode_and_allocates_unique_seeds() {
        let root=test_dir();
        for engine in ["chrome","edge","firefox"] {
            let original=get(root.path(), &format!("legacy-{engine}"), false).unwrap();
            assert!(original.preferences.fingerprint.is_none());
            let draft=|name:&str| Draft {name:name.into(),browser:engine.into(),notes:String::new(),tags:vec![],proxy:ProxyConfig::system(),password:None,clear_password:false,preferences:original.preferences.clone()};
            let first=create(root.path(),draft("Copy one"),Some(&original.id)).unwrap();
            let second=create(root.path(),draft("Copy two"),Some(&original.id)).unwrap();
            assert!(!first.preferences.fingerprint.as_ref().unwrap().enabled, "{engine}: copying enabled automation that the source did not use");
            assert!(!crate::regional::needs_control_for(engine,&first.preferences));
            assert_ne!(first.preferences.fingerprint.as_ref().unwrap().seed, second.preferences.fingerprint.as_ref().unwrap().seed);
        }
    }
    #[test]
    fn new_profile_fingerprints_are_independent_and_survive_reload() {
        let root = test_dir();
        let draft=serde_json::from_value(serde_json::json!({"name":"Fingerprint","browser":"chrome","proxy":{"mode":"direct","host":"","port":0,"username":""}})).unwrap();
        let first = super::create(root.path(), draft, None).unwrap();
        let draft=serde_json::from_value(serde_json::json!({"name":"Fingerprint two","browser":"chrome","proxy":{"mode":"direct","host":"","port":0,"username":""}})).unwrap();
        let second = super::create(root.path(), draft, None).unwrap();
        let seed = |p: &super::Profile| {
            serde_json::to_value(p).unwrap()["preferences"]["fingerprint"]["seed"]
                .as_str()
                .map(str::to_owned)
        };
        assert!(seed(&first).is_some(), "missing persisted fingerprint seed");
        assert_ne!(seed(&first), seed(&second));
        assert_eq!(
            seed(&super::get(root.path(), &first.id, false).unwrap()),
            seed(&first)
        );
    }
    #[test]
    fn copied_fingerprints_are_new_but_cloud_restore_and_regeneration_preserve_login_data() {
        let root = test_dir();
        let draft=serde_json::from_value(serde_json::json!({"name":"Fingerprint","browser":"chrome","proxy":{"mode":"direct","host":"","port":0,"username":""}})).unwrap();
        let first = create(root.path(), draft, None).unwrap();
        let document = crate::cloud::model::Document::export(&first).unwrap();
        assert_eq!(document.schema, 3);
        let mut older = serde_json::to_value(&document).unwrap();
        older["schema"] = serde_json::json!(2);
        assert!(crate::cloud::model::Document::parse(older).is_err());
        let copy = create(root.path(), document.draft(), Some(&first.id)).unwrap();
        assert!(
            copy.preferences.fingerprint.as_ref().unwrap().seed
                != first.preferences.fingerprint.as_ref().unwrap().seed
        );
        let imported = create_synced(root.path(), document.draft(), &identifier()).unwrap();
        assert!(imported.preferences.fingerprint == first.preferences.fingerprint);
        let path = super::path(root.path(), &first).unwrap();
        std::fs::write(path.join("login-sentinel"), b"retained").unwrap();
        let mut edited = document.draft();
        edited.preferences.fingerprint.as_mut().unwrap().seed = crate::fingerprint::seed().unwrap();
        let updated = update(root.path(), &first.id, edited).unwrap();
        assert!(
            updated.preferences.fingerprint.as_ref().unwrap().seed
                != first.preferences.fingerprint.as_ref().unwrap().seed
        );
        assert_eq!(
            std::fs::read(path.join("login-sentinel")).unwrap(),
            b"retained"
        );
        let context = crate::cloud::crypto::Context {
            account: "fixture",
            object: "profile",
            kind: "config",
            revision: 1,
            epoch: 1,
            deleted: false,
            generation: 0,
            writer: "fixture",
        };
        let key = crate::cloud::crypto::random_key().unwrap();
        let bytes = serde_json::to_vec(&document).unwrap();
        let encrypted = crate::cloud::crypto::seal(&key, &context, &bytes).unwrap();
        let plain = crate::cloud::crypto::open(&key, &context, &encrypted).unwrap();
        let decoded =
            crate::cloud::model::Document::parse(serde_json::from_slice(&plain).unwrap()).unwrap();
        assert!(decoded.preferences.fingerprint == first.preferences.fingerprint);
    }
    use super::*;
    #[test]
    fn independent_privacy_controls_are_applied_and_reversible_in_native_profiles() {
        for browser in ["chrome", "edge", "firefox"] {
            let root = test_dir();
            let mut d = draft("Privacy controls");
            d.browser = browser.into();
            d.preferences.privacy = false;
            d.preferences.advanced=serde_json::from_value(serde_json::json!({"doNotTrack":"on","webRtc":"public","blockWebgl":true,"blockWebgpu":true,"blockSpeech":true,"blockBattery":browser=="firefox","canvasProtection":browser=="firefox"})).unwrap();
            let p = create(root.path(), d.clone(), None).unwrap();
            let dir = path(root.path(), &p).unwrap();
            if browser == "firefox" {
                for (key, value) in [
                    ("privacy.donottrackheader.enabled", true),
                    ("media.peerconnection.ice.no_host", true),
                    ("webgl.disabled", true),
                    ("dom.webgpu.enabled", false),
                    ("media.webspeech.synth.enabled", false),
                    ("dom.battery.enabled", false),
                    ("privacy.fingerprintingProtection", true),
                ] {
                    assert_eq!(
                        crate::firefox::snapshot(&dir, key).unwrap()["user"],
                        serde_json::json!(value),
                        "{key}"
                    );
                }
            } else {
                let prefs: serde_json::Value = serde_json::from_slice(
                    &std::fs::read(dir.join("Default/Preferences")).unwrap(),
                )
                .unwrap();
                assert_eq!(prefs["enable_do_not_track"], true);
                assert_eq!(
                    prefs["webrtc"]["ip_handling_policy"],
                    "default_public_interface_only"
                );
            }
            d.preferences.advanced = AdvancedSettings::default();
            update(root.path(), &p.id, d).unwrap();
            apply_pending_preferences(root.path(), &p.id).unwrap();
            if browser == "firefox" {
                assert_eq!(
                    crate::firefox::snapshot(&dir, "dom.webgpu.enabled").unwrap()["user"],
                    serde_json::Value::Null
                );
                assert_eq!(
                    crate::firefox::snapshot(&dir, "privacy.fingerprintingProtection").unwrap()
                        ["user"],
                    serde_json::Value::Null
                );
            }
        }
    }
    #[test]
    fn removes_default_name_prefix_without_renaming_custom_profiles() {
        let root = test_dir();
        let mut profiles = load(root.path()).unwrap();
        assert_eq!(
            profiles.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
            vec!["Chrome", "Edge", "Firefox"]
        );
        profiles[0].name = "默认 Chrome".into();
        profiles[1].name = "My Edge".into();
        save(root.path(), &profiles).unwrap();
        let custom = create(root.path(), draft("默认 Chrome"), None).unwrap();
        let loaded = load(root.path()).unwrap();
        assert_eq!(
            loaded.iter().find(|p| p.id == profiles[0].id).unwrap().name,
            "Chrome"
        );
        assert_eq!(
            loaded.iter().find(|p| p.id == profiles[1].id).unwrap().name,
            "My Edge"
        );
        assert_eq!(
            loaded.iter().find(|p| p.id == custom.id).unwrap().name,
            "默认 Chrome"
        );
    }
    #[test]
    fn removes_generated_legacy_notes_but_keeps_user_notes() {
        let root = test_dir();
        let mut profiles = load(root.path()).unwrap();
        assert!(profiles.iter().all(|p| p.notes.is_empty()));
        profiles[0].notes = "从原专用浏览器接入，原有数据保留".into();
        profiles[1].notes = "Work browser".into();
        save(root.path(), &profiles).unwrap();
        let mut custom = draft("Custom profile");
        custom.notes = "从原专用浏览器接入，原有数据保留".into();
        let custom = create(root.path(), custom, None).unwrap();
        let loaded = load(root.path()).unwrap();
        assert!(loaded
            .iter()
            .find(|p| p.id == profiles[0].id)
            .unwrap()
            .notes
            .is_empty());
        assert_eq!(
            loaded
                .iter()
                .find(|p| p.id == profiles[1].id)
                .unwrap()
                .notes,
            "Work browser"
        );
        assert_eq!(
            loaded.iter().find(|p| p.id == custom.id).unwrap().notes,
            custom.notes
        );
    }
    #[test]
    fn legacy_region_sources_are_disabled_on_load_without_changing_proxy_or_language() {
        let root = test_dir();
        let mut d = draft("Existing regional profile");
        d.preferences.language = "de-DE,de,en".into();
        let created = create(root.path(), d, None).unwrap();
        let mut old = load(root.path()).unwrap();
        let p = old.iter_mut().find(|p| p.id == created.id).unwrap();
        p.preferences.fingerprint = None; // This fixture models an older persisted profile.
        p.preferences.regional.timezone_mode = "custom".into();
        p.preferences.regional.timezone = "America/Los_Angeles".into();
        p.preferences.regional.location_mode = "ip".into();
        p.preferences.advanced.location = PermissionMode::Allow;
        p.active_preferences = Some(p.preferences.clone());
        p.preferences_dirty = false;
        save(root.path(), &old).unwrap();
        let migrated = get(root.path(), &created.id, false).unwrap();
        assert_eq!(migrated.preferences.regional.timezone_mode, "system");
        assert_eq!(migrated.preferences.regional.location_mode, "system");
        assert!(migrated.preferences.advanced.location == PermissionMode::Ask);
        assert_eq!(migrated.preferences.language, "de-DE,de,en");
        assert_eq!(migrated.proxy, created.proxy);
        assert!(migrated.preferences_dirty);
        assert!(!crate::regional::needs_control(&migrated.preferences));
        assert_eq!(
            migrated.active_preferences.unwrap().regional.timezone_mode,
            "custom"
        );
    }
    #[test]
    fn old_preferences_receive_safe_advanced_defaults() {
        let prefs: Preferences = serde_json::from_value(serde_json::json!({
            "language":"en-GB,en", "privacy":false, "fontRestriction":false, "startupUrl":"about:blank"
        })).unwrap();
        assert_eq!(prefs.language, "en-GB,en");
        assert!(prefs.fingerprint.is_none());
        assert!(prefs.advanced == AdvancedSettings::default());
        assert!(serde_json::from_value::<AdvancedSettings>(
            serde_json::json!({"notifications":"invalid"})
        )
        .is_err());
    }
    #[tokio::test]
    #[ignore = "Starts temporary Chrome and Firefox windows to verify browser-observed advanced preferences"]
    async fn advanced_browser_preferences_are_observed_in_real_windows() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        for browser in ["chrome", "firefox"] {
            let root = test_dir();
            let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let address = listener.local_addr().unwrap();
            let server = tokio::spawn(async move {
                loop {
                    let (mut stream, _) = listener.accept().await.unwrap();
                    let mut request = Vec::new();
                    loop {
                        let mut chunk = [0; 2048];
                        let n = match tokio::time::timeout(
                            std::time::Duration::from_secs(2),
                            stream.read(&mut chunk),
                        )
                        .await
                        {
                            Ok(Ok(n)) => n,
                            _ => break,
                        };
                        if n == 0 {
                            break;
                        }
                        request.extend_from_slice(&chunk[..n]);
                        if request.windows(4).any(|w| w == b"\r\n\r\n") || request.len() > 16384 {
                            break;
                        }
                    }
                    let request = String::from_utf8_lossy(&request);
                    let path = request
                        .lines()
                        .next()
                        .unwrap_or("")
                        .split_whitespace()
                        .nth(1)
                        .unwrap_or("/");
                    let report = path.starts_with("/report?").then(|| {
                        let url = reqwest::Url::parse(&format!("http://localhost{path}")).unwrap();
                        let data = url
                            .query_pairs()
                            .find(|(key, _)| key == "data")
                            .unwrap()
                            .1
                            .into_owned();
                        serde_json::from_str::<serde_json::Value>(&data).unwrap()
                    });
                    let (kind, body) = if path.starts_with("/image") {
                        ("image/svg+xml", "<svg xmlns='http://www.w3.org/2000/svg' width='10' height='10'><rect width='10' height='10'/></svg>")
                    } else if report.is_some() {
                        ("text/plain", "ok")
                    } else {
                        (
                            "text/html",
                            r#"<!doctype html><title>NodeCloak advanced settings test</title><p>Temporary browser preference verification</p><script>
                    let reported=false; async function finish(imagesLoaded) { if(reported)return;reported=true;
                      const query=async(name)=>{try{return (await navigator.permissions.query({name})).state}catch{return 'unsupported'}};
                      const data={imagesLoaded,timezone:Intl.DateTimeFormat().resolvedOptions().timeZone,offset:new Date().getTimezoneOffset(),webgl:!!document.createElement('canvas').getContext('webgl'),notifications:await query('notifications'),location:await query('geolocation')};
                      await fetch('/report?data='+encodeURIComponent(JSON.stringify(data)));document.body.textContent='Verification complete. This temporary window will close.';
                    }
                    const img=new Image();img.onload=()=>finish(true);img.onerror=()=>finish(false);img.src='/image';setTimeout(()=>finish(img.complete&&img.naturalWidth>0),1000);
                    </script>"#,
                        )
                    };
                    let response = format!("HTTP/1.1 200 OK\r\nContent-Type: {kind}\r\nContent-Length: {}\r\nConnection: close\r\nCache-Control: no-store\r\n\r\n{body}",body.len());
                    let _ = stream.write_all(response.as_bytes()).await;
                    if let Some(report) = report {
                        return report;
                    }
                }
            });
            let mut d = draft("Advanced settings runtime verification");
            d.browser = browser.into();
            d.proxy.mode = "direct".into();
            d.preferences.advanced.notifications = PermissionMode::Block;
            d.preferences.advanced.location = PermissionMode::Block;
            d.preferences.advanced.block_images = true;
            d.preferences.advanced.block_webgl = browser == "firefox";
            d.preferences.advanced.resist_fingerprinting = browser == "firefox";
            let p = create(root.path(), d, None).unwrap();
            let dir = path(root.path(), &p).unwrap();
            platform::spawn_browser_with_proxy(
                browser,
                &dir,
                &format!("http://{address}/"),
                "direct",
                None,
            )
            .unwrap();
            let result = tokio::time::timeout(std::time::Duration::from_secs(30), server).await;
            // Close our exact temporary profile before assertions, including on failure.
            let closed = platform::close_profile(&dir);
            closed.unwrap();
            let data = result
                .expect("browser did not report preferences in time")
                .unwrap();
            println!("{browser} observed: {data}");
            assert_eq!(data["imagesLoaded"], false);
            assert_eq!(data["notifications"], "denied");
            assert_eq!(data["location"], "denied");
            if browser == "firefox" {
                assert_eq!(data["webgl"], false);
                assert_eq!(data["offset"], 0);
                assert!(["UTC", "Atlantic/Reykjavik"].contains(&data["timezone"].as_str().unwrap()));
            }
        }
    }
    #[test]
    fn advanced_firefox_settings_apply_restore_and_keep_unrelated_data() {
        let root = test_dir();
        let mut d = draft("Firefox protected");
        d.browser = "firefox".into();
        d.preferences.advanced.resist_fingerprinting = true;
        d.preferences.advanced.block_webgl = true;
        d.preferences.advanced.block_images = true;
        d.preferences.advanced.notifications = PermissionMode::Block;
        d.preferences.advanced.location = PermissionMode::Block;
        d.preferences.advanced.camera = PermissionMode::Block;
        d.preferences.advanced.microphone = PermissionMode::Block;
        let p = create(root.path(), d, None).unwrap();
        let dir = path(root.path(), &p).unwrap();
        assert!(
            existing_preferences(&base(root.path(), &p).unwrap(), "firefox").advanced
                == p.preferences.advanced
        );
        for (key, value) in [
            ("privacy.resistFingerprinting", serde_json::json!(true)),
            ("webgl.disabled", serde_json::json!(true)),
            ("permissions.default.image", serde_json::json!(2)),
            ("permissions.default.geo", serde_json::json!(2)),
            ("permissions.default.camera", serde_json::json!(2)),
            ("permissions.default.microphone", serde_json::json!(2)),
            (
                "permissions.default.desktop-notification",
                serde_json::json!(2),
            ),
        ] {
            assert_eq!(crate::firefox::snapshot(&dir, key).unwrap()["user"], value);
        }
        std::fs::write(dir.join("cookies.sqlite"), b"keep cookies").unwrap();
        let mut text = std::fs::read_to_string(dir.join("user.js")).unwrap();
        text.push_str("user_pref(\"unrelated.example\", 42);\n");
        std::fs::write(dir.join("user.js"), text).unwrap();
        let mut reset = draft("Firefox protected");
        reset.browser = "firefox".into();
        let changed = update(root.path(), &p.id, reset).unwrap();
        assert_eq!(
            crate::firefox::snapshot(&dir, "privacy.resistFingerprinting").unwrap()["user"],
            true
        );
        apply_preferences(root.path(), &changed).unwrap();
        assert_eq!(
            crate::firefox::snapshot(&dir, "privacy.resistFingerprinting").unwrap()["user"],
            false
        );
        assert_eq!(
            crate::firefox::snapshot(&dir, "permissions.default.geo").unwrap()["user"],
            0
        );
        assert_eq!(
            crate::firefox::snapshot(&dir, "permissions.default.image").unwrap()["user"],
            1
        );
        assert!(std::fs::read_to_string(dir.join("user.js"))
            .unwrap()
            .contains("unrelated.example"));
        assert_eq!(
            std::fs::read(dir.join("cookies.sqlite")).unwrap(),
            b"keep cookies"
        );
    }
    #[test]
    fn advanced_chromium_defaults_preserve_site_exceptions_and_reject_malformed_structure() {
        let root = test_dir();
        let p = create(root.path(), draft("Chrome permissions"), None).unwrap();
        let file = path(root.path(), &p).unwrap().join("Default/Preferences");
        let original = serde_json::json!({"profile":{"content_settings":{"exceptions":{"notifications":{"https://allowed.example,*":{"setting":1}}}}},"unrelated":42});
        std::fs::write(&file, serde_json::to_vec(&original).unwrap()).unwrap();
        let mut d = draft("Chrome permissions");
        d.preferences.advanced.notifications = PermissionMode::Block;
        d.preferences.advanced.block_images = true;
        let changed = update(root.path(), &p.id, d).unwrap();
        apply_preferences(root.path(), &changed).unwrap();
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
        assert_eq!(
            data["profile"]["default_content_setting_values"]["notifications"],
            2
        );
        assert_eq!(
            data["profile"]["default_content_setting_values"]["geolocation"],
            3
        );
        assert_eq!(
            data["profile"]["default_content_setting_values"]["images"],
            2
        );
        assert_eq!(
            data["profile"]["content_settings"],
            original["profile"]["content_settings"]
        );
        assert_eq!(data["unrelated"], 42);
        assert!(
            existing_preferences(&base(root.path(), &p).unwrap(), "chrome").advanced
                == changed.preferences.advanced
        );
        let invalid = br#"{"profile":{"default_content_setting_values":false}}"#;
        std::fs::write(&file, invalid).unwrap();
        assert!(apply_preferences(root.path(), &changed).is_err());
        assert_eq!(std::fs::read(&file).unwrap(), invalid);
        let mut unsupported = draft("invalid");
        unsupported.preferences.advanced.resist_fingerprinting = true;
        assert!(create(root.path(), unsupported, None).is_err());
    }

    #[test]
    fn location_block_revokes_chromium_grants_without_changing_other_permissions() {
        let root = test_dir();
        let p = create(root.path(), draft("Location permissions"), None).unwrap();
        let file = path(root.path(), &p).unwrap().join("Default/Preferences");
        let original = serde_json::json!({"profile":{"content_settings":{"exceptions":{"geolocation":{"https://allowed.example,*":{"setting":1},"https://blocked.example,*":{"setting":2}},"notifications":{"https://allowed.example,*":{"setting":1}}}}}});
        std::fs::write(&file, serde_json::to_vec(&original).unwrap()).unwrap();
        let mut d = draft("Location permissions");
        d.preferences.advanced.location = PermissionMode::Block;
        let changed = update(root.path(), &p.id, d).unwrap();
        apply_preferences(root.path(), &changed).unwrap();
        let data: serde_json::Value =
            serde_json::from_slice(&std::fs::read(file).unwrap()).unwrap();
        assert!(
            data["profile"]["content_settings"]["exceptions"]["geolocation"]
                .get("https://allowed.example,*")
                .is_none()
        );
        assert_eq!(
            data["profile"]["content_settings"]["exceptions"]["geolocation"]
                ["https://blocked.example,*"]["setting"],
            2
        );
        assert_eq!(
            data["profile"]["content_settings"]["exceptions"]["notifications"],
            original["profile"]["content_settings"]["exceptions"]["notifications"]
        );
    }
    #[test]
    fn firefox_control_suppresses_first_run_dialog_without_changing_privacy_or_consent() {
        let root = test_dir();
        let mut d = draft("Firefox automation");
        d.browser = "firefox".into();
        let profile = create(root.path(), d, None).unwrap();
        let dir = path(root.path(), &profile).unwrap();
        std::fs::write(dir.join("user.js"), "user_pref(\"termsofuse.acceptedVersion\", 0);\nuser_pref(\"privacy.trackingprotection.enabled\", true);\n").unwrap();
        apply_preferences_for_control(root.path(), &profile, true).unwrap();
        assert_eq!(crate::firefox::snapshot(&dir, "termsofuse.bypassNotification").unwrap()["user"], true);
        assert_eq!(crate::firefox::snapshot(&dir, "dom.file.createInChild").unwrap()["user"], true);
        let contents = std::fs::read_to_string(dir.join("user.js")).unwrap();
        assert!(contents.contains("user_pref(\"termsofuse.acceptedVersion\", 0);"));
        assert!(contents.contains("user_pref(\"privacy.trackingprotection.enabled\", true);"));
        apply_preferences_for_control(root.path(), &profile, false).unwrap();
        assert!(crate::firefox::snapshot(&dir, "termsofuse.bypassNotification").unwrap()["user"].is_null());
        assert!(crate::firefox::snapshot(&dir, "dom.file.createInChild").unwrap()["user"].is_null());
    }
    fn test_dir() -> tempfile::TempDir {
        // macOS /var is a system symlink; fixtures use its real path while
        // production profile paths still reject symlinked ancestors.
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }
    #[cfg(unix)]
    #[test]
    fn profile_paths_reject_symlinked_directories() {
        let root = test_dir();
        let real = root.path().join("real");
        std::fs::create_dir(&real).unwrap();
        let alias = root.path().join("alias");
        std::os::unix::fs::symlink(&real, &alias).unwrap();
        assert!(load(&alias).is_err());
        assert!(!real.join("profiles.json").exists());
        std::fs::create_dir(root.path().join("profiles")).unwrap();
        let id = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        std::os::unix::fs::symlink(&real, root.path().join("profiles").join(id)).unwrap();
        let mut p = load(root.path()).unwrap().remove(0);
        p.id = id.into();
        p.legacy = false;
        assert!(base(root.path(), &p).is_err());
    }
    #[tokio::test]
    async fn fresh_launch_replaces_stale_relay_while_recovery_keeps_active_route() {
        use tokio::{
            io::{AsyncReadExt, AsyncWriteExt},
            net::TcpListener,
        };
        let old = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let new = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let mut d = draft("relay lifecycle");
        d.proxy = ProxyConfig {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: old.local_addr().unwrap().port(),
            ..ProxyConfig::default()
        };
        let root = test_dir();
        let p = create(root.path(), d, None).unwrap();
        let runtime = Runtime::default();
        let old_port = prepare_relay(root.path(), &p, &runtime, false)
            .await
            .unwrap();
        mark_started(root.path(), &p.id, old_port, None).unwrap();
        let mut d = draft("relay lifecycle");
        d.proxy = ProxyConfig {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: new.local_addr().unwrap().port(),
            ..ProxyConfig::default()
        };
        let updated = update(root.path(), &p.id, d).unwrap();
        assert_eq!(
            prepare_relay(root.path(), &updated, &runtime, true)
                .await
                .unwrap(),
            old_port
        );
        let port = prepare_relay(root.path(), &updated, &runtime, false)
            .await
            .unwrap()
            .unwrap();
        let server = tokio::spawn(async move {
            let (mut socket, _) = new.accept().await.unwrap();
            let mut request = vec![];
            loop {
                request.push(socket.read_u8().await.unwrap());
                if request.ends_with(b"\r\n\r\n") {
                    break;
                }
            }
            socket
                .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 3\r\nConnection: close\r\n\r\nnew")
                .await
                .unwrap();
        });
        let client = reqwest::Client::builder()
            .no_proxy()
            .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{port}")).unwrap())
            .timeout(std::time::Duration::from_secs(3))
            .build()
            .unwrap();
        assert_eq!(
            client
                .get("http://example.com/test")
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "new"
        );
        server.await.unwrap();
        assert!(
            tokio::time::timeout(std::time::Duration::from_millis(150), old.accept())
                .await
                .is_err()
        );
    }
    #[test]
    fn repairs_and_undo_survive_next_launch_and_histories_do_not_cross_profiles() {
        let root = test_dir();
        let mut d = draft("Firefox one");
        d.browser = "firefox".into();
        let one = create(root.path(), d, None).unwrap();
        let mut d = draft("Firefox two");
        d.browser = "firefox".into();
        let two = create(root.path(), d, None).unwrap();
        let scoped = base(root.path(), &one).unwrap();
        let output = engine::repair_with_font_consent(
            &scoped,
            "firefox",
            &["fonts".into()],
            false,
            "singapore",
            true,
        )
        .unwrap();
        assert!(output[0].success);
        sync_preferences(root.path(), &one.id).unwrap();
        apply_preferences(root.path(), &get(root.path(), &one.id, false).unwrap()).unwrap();
        assert!(crate::firefox::configured(&path(root.path(), &one).unwrap(), "fonts").unwrap());
        assert!(!crate::firefox::configured(&path(root.path(), &two).unwrap(), "fonts").unwrap());
        let records = history(root.path(), Some(&one.id)).unwrap();
        assert_eq!(records.len(), 1);
        assert!(history(root.path(), Some(&two.id)).unwrap().is_empty());
        engine::undo(&scoped, &records[0].id).unwrap();
        sync_preferences(root.path(), &one.id).unwrap();
        apply_preferences(root.path(), &get(root.path(), &one.id, false).unwrap()).unwrap();
        assert!(!crate::firefox::configured(&path(root.path(), &one).unwrap(), "fonts").unwrap());
        let runtime = Runtime::default();
        remove(root.path(), &one.id, false, &runtime).unwrap();
        remove(root.path(), &one.id, true, &runtime).unwrap();
        assert!(!scoped.exists());
        assert!(base(root.path(), &two).unwrap().exists());
    }
    #[test]
    fn migrating_and_purging_legacy_profiles_preserves_computer_history() {
        let root = test_dir();
        load(root.path()).unwrap();
        let records = ["language", "timezone", "cli"]
            .into_iter()
            .map(|id| engine::Record {
                profile_id: None,
                id: id.into(),
                item_id: id.into(),
                browser: "chrome".into(),
                created_at: stamp(),
                status: "applied".into(),
                message: String::new(),
                changes: vec![],
            })
            .collect::<Vec<_>>();
        engine::save_history(root.path(), &records).unwrap();
        let runtime = Runtime::default();
        remove(root.path(), "legacy-chrome", false, &runtime).unwrap();
        assert_eq!(history(root.path(), None).unwrap().len(), 2);
        restore(root.path(), "legacy-chrome").unwrap();
        assert_eq!(
            history(root.path(), Some("legacy-chrome")).unwrap().len(),
            1
        );
        remove(root.path(), "legacy-chrome", false, &runtime).unwrap();
        remove(root.path(), "legacy-chrome", true, &runtime).unwrap();
        assert_eq!(engine::history(root.path()).unwrap().len(), 2);
        assert!(root.path().join("profiles.json").exists());
    }
    #[test]
    fn editing_settings_defers_disk_writes_until_launch() {
        let root = test_dir();
        let p = create(root.path(), draft("one"), None).unwrap();
        let mut d = draft("one");
        d.preferences.language = "en-GB,en".into();
        let updated = update(root.path(), &p.id, d).unwrap();
        assert!(updated.preferences_dirty);
        assert_eq!(
            existing_preferences(&base(root.path(), &p).unwrap(), "chrome").language,
            "en-US,en"
        );
        apply_preferences(root.path(), &updated).unwrap();
        mark_started(root.path(), &p.id, None, None).unwrap();
        assert_eq!(
            existing_preferences(&base(root.path(), &p).unwrap(), "chrome").language,
            "en-GB,en"
        );
        assert!(!get(root.path(), &p.id, false).unwrap().preferences_dirty);
    }
    #[test]
    #[ignore = "Uses the native OS credential store; run explicitly on Windows/macOS"]
    fn native_password_lifecycle_keeps_json_secret_free() {
        let root = test_dir();
        let mut d = draft("credential test");
        d.proxy = ProxyConfig {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            username: "test".into(),
            ..ProxyConfig::default()
        };
        d.password = Some("test-only-credential-060".into());
        let p = create(root.path(), d, None).unwrap();
        assert_eq!(
            upstream(&p, false).unwrap().password,
            "test-only-credential-060"
        );
        assert!(!std::fs::read_to_string(root.path().join("profiles.json"))
            .unwrap()
            .contains("test-only-credential-060"));
        let reference = p.proxy.credential_ref.clone().unwrap();
        let runtime = Runtime::default();
        remove(root.path(), &p.id, false, &runtime).unwrap();
        remove(root.path(), &p.id, true, &runtime).unwrap();
        assert!(matches!(
            secret_entry(&p.id, &reference).unwrap().get_password(),
            Err(keyring::Error::NoEntry)
        ));
    }
    #[test]
    #[ignore = "Uses only synthetic profile UUIDs in the OS credential store"]
    fn native_node_credentials_copy_edit_sync_and_reset_are_scoped() {
        let root=test_dir();let uuid=uuid::Uuid::new_v4().to_string();
        let mut input=draft("Synthetic node credential");
        input.proxy=serde_json::from_value(serde_json::json!({"mode":"vless","host":"node.invalid","port":443,"node":{}})).unwrap();
        input.password=Some(uuid.clone());
        let original=create(root.path(),input.clone(),None).unwrap();
        let _original_cleanup=PendingCredentials{id:original.id.clone(),references:original.credential_refs.clone(),committed:false};
        assert_eq!(upstream(&original,false).unwrap().password,uuid);
        let doc=crate::cloud::model::Document::export(&original).unwrap();assert_eq!(doc.schema,6);
        assert_eq!(crate::cloud::model::Document::parse(serde_json::to_value(doc).unwrap()).unwrap().draft().password.as_deref(),Some(uuid.as_str()));
        input.password=None;
        let copied=create(root.path(),input.clone(),Some(&original.id)).unwrap();
        let _copied_cleanup=PendingCredentials{id:copied.id.clone(),references:copied.credential_refs.clone(),committed:false};
        assert_ne!(copied.proxy.credential_ref,original.proxy.credential_ref);assert_eq!(upstream(&copied,false).unwrap().password,uuid);
        input.proxy.pre_proxy=Some(crate::proxy::LocalProxy{mode:"http".into(),host:"192.168.10.1".into(),port:7890});
        assert_eq!(draft_upstream(root.path(),Some(&original.id),&input).unwrap().password,uuid);
        let changed=update(root.path(),&original.id,input.clone()).unwrap();assert_eq!(changed.proxy.credential_ref,original.proxy.credential_ref);
        assert!(!std::fs::read_to_string(root.path().join("profiles.json")).unwrap().contains(&uuid));
        reset::reset(root.path(),&original.id,false,&Runtime::default()).unwrap();
        assert_eq!(upstream(&get(root.path(),&original.id,false).unwrap(),false).unwrap().password,uuid);
        reset::reset(root.path(),&original.id,true,&Runtime::default()).unwrap();assert_eq!(get(root.path(),&original.id,false).unwrap().proxy,ProxyConfig::system());
    }
    #[test]
    fn new_node_without_uuid_is_not_saved() {
        let root=test_dir();let before=load(root.path()).unwrap().len();let mut input=draft("Missing node credential");
        input.proxy=serde_json::from_value(serde_json::json!({"mode":"vless","host":"node.invalid","port":443,"node":{}})).unwrap();
        assert!(create(root.path(),input,None).is_err());assert_eq!(load(root.path()).unwrap().len(),before);
    }
    fn draft(name: &str) -> Draft {
        Draft {
            name: name.into(),
            browser: "chrome".into(),
            notes: String::new(),
            tags: vec!["工作".into()],
            proxy: ProxyConfig::system(),
            password: None,
            clear_password: false,
            preferences: Preferences::default(),
        }
    }
    #[test]
    fn computer_font_records_stay_global_even_when_firefox_is_selected() {
        let root = test_dir();
        load(root.path()).unwrap();
        let record:engine::Record=serde_json::from_value(serde_json::json!({"id":"font-test","itemId":"fonts","browser":"firefox","createdAt":stamp(),"status":"applied","message":"","changes":[{"target":"userFont","pointer":"","before":null,"after":null}]})).unwrap();
        engine::save_history(root.path(), &[record]).unwrap();
        assert!(history(root.path(), Some("legacy-firefox"))
            .unwrap()
            .is_empty());
        assert!(history(root.path(), None).unwrap()[0].profile_id.is_none());
        let runtime = Runtime::default();
        remove(root.path(), "legacy-firefox", false, &runtime).unwrap();
        remove(root.path(), "legacy-firefox", true, &runtime).unwrap();
        assert_eq!(history(root.path(), None).unwrap().len(), 1);
    }
    #[test]
    fn migration_keeps_existing_login_and_registry_is_idempotent() {
        let root = test_dir();
        let path = root.path().join("browser-chrome/Default");
        std::fs::create_dir_all(&path).unwrap();
        std::fs::write(path.join("Cookies"), b"login").unwrap();
        let first = load(root.path()).unwrap();
        assert_eq!(first.len(), 3);
        assert_eq!(load(root.path()).unwrap()[0].id, first[0].id);
        assert_eq!(std::fs::read(path.join("Cookies")).unwrap(), b"login");
    }
    #[test]
    fn new_profiles_and_copied_settings_do_not_copy_login_or_history() {
        let root = test_dir();
        let p = create(root.path(), draft("one"), None).unwrap();
        std::fs::write(
            path(root.path(), &p).unwrap().join("Default/Cookies"),
            b"private",
        )
        .unwrap();
        let second = create(root.path(), draft("two"), Some(&p.id)).unwrap();
        assert!(!path(root.path(), &second)
            .unwrap()
            .join("Default/Cookies")
            .exists());
        assert_ne!(
            path(root.path(), &p).unwrap(),
            path(root.path(), &second).unwrap()
        );
        assert!(get(root.path(), "../../escape", false).is_err());
    }
    #[test]
    fn trash_restore_and_purge_are_scoped_to_one_profile() {
        let root = test_dir();
        let p = create(root.path(), draft("one"), None).unwrap();
        let other = create(root.path(), draft("two"), None).unwrap();
        let runtime = Runtime::default();
        assert!(remove(root.path(), &p.id, true, &runtime).is_err());
        remove(root.path(), &p.id, false, &runtime).unwrap();
        assert!(get(root.path(), &p.id, false).is_err());
        restore(root.path(), &p.id).unwrap();
        assert!(get(root.path(), &p.id, false).is_ok());
        remove(root.path(), &p.id, false, &runtime).unwrap();
        remove(root.path(), &p.id, true, &runtime).unwrap();
        assert!(path(root.path(), &other).unwrap().exists());
        assert!(get(root.path(), &other.id, false).is_ok());
    }
    #[tokio::test]
    async fn api_proxy_active_endpoint_is_pinned_across_recovery_and_refetched_only_on_fresh_start() {
        use tokio::io::{AsyncReadExt,AsyncWriteExt};
        let listener=tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();let port=listener.local_addr().unwrap().port();
        let calls=std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));let observed=calls.clone();
        let provider=tokio::spawn(async move {loop {
            let (mut stream,_)=listener.accept().await.unwrap();let count=observed.fetch_add(1,std::sync::atomic::Ordering::SeqCst)+1;
            let mut request=[0;4096];let _=stream.read(&mut request).await;
            let body=format!("192.0.2.{count}:{}",8000+count);
            stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",body.len()).as_bytes()).await.unwrap();
        }});
        let root=test_dir();let mut input=draft("API pin");
        input.proxy=serde_json::from_value(serde_json::json!({"mode":"http","apiUrl":format!("http://127.0.0.1:{port}/api")})).unwrap();
        let p=create(root.path(),input.clone(),None).unwrap();let runtime=Runtime::default();
        let relay_port=prepare_relay(root.path(),&p,&runtime,false).await.unwrap();
        mark_started(root.path(),&p.id,relay_port,runtime.proxy_endpoint(&p.id).unwrap()).unwrap();
        let saved=get(root.path(),&p.id,false).unwrap();
        let active=upstream(&saved,true).unwrap();
        assert_eq!(active.config.host,"192.0.2.1","active API endpoint was lost after launch");
        assert!(active.config.api_url.is_none(),"live tests must not refetch API");
        runtime.0.lock().unwrap().remove(&p.id);
        // Model a completed process exit: Relay Drop signals its async accept
        // loop, whereas a real app exit has already released the listener.
        let deadline=std::time::Instant::now()+std::time::Duration::from_secs(2);
        loop {
            if let Ok(listener)=tokio::net::TcpListener::bind(("127.0.0.1",relay_port.unwrap())).await {drop(listener);break;}
            assert!(std::time::Instant::now()<deadline,"old fixture relay did not stop");
            tokio::time::sleep(std::time::Duration::from_millis(5)).await;
        }
        prepare_relay(root.path(),&saved,&runtime,true).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst),1,"recovery changed running IP");
        prepare_relay(root.path(),&saved,&runtime,false).await.unwrap();
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst),2);
        provider.abort();
    }
    #[test]
    fn chain_settings_survive_copy_edit_and_preserve_second_hop_credential_identity() {
        let root=test_dir();let mut input=draft("chain");
        input.proxy=serde_json::from_value(serde_json::json!({"mode":"http","host":"remote.invalid","port":8080,"username":"user",
            "preProxy":{"mode":"http","host":"127.0.0.1","port":7890}})).unwrap();
        let original=create(root.path(),input.clone(),None).unwrap();
        let copy=create(root.path(),input.clone(),Some(&original.id)).unwrap();
        assert_eq!(get(root.path(),&copy.id,false).unwrap().proxy.pre_proxy,input.proxy.pre_proxy);
        let mut all=load(root.path()).unwrap();
        let p=all.iter_mut().find(|p|p.id==original.id).unwrap();
        p.proxy.credential_ref=Some(identifier());let reference=p.proxy.credential_ref.clone();
        p.active_proxy=Some(p.proxy.clone());save(root.path(),&all).unwrap();
        input.proxy.pre_proxy.as_mut().unwrap().port=1081;
        let changed=update(root.path(),&original.id,input.clone()).unwrap();
        assert_eq!(changed.proxy.credential_ref,reference,"changing only the local route lost remote credentials");
        assert_eq!(changed.active_proxy.unwrap().pre_proxy.unwrap().port,7890,"running route changed before restart");
        assert_eq!(changed.proxy.pre_proxy.unwrap().port,1081);
        input.proxy.pre_proxy=None;let off=update(root.path(),&original.id,input).unwrap();
        assert!(off.proxy.pre_proxy.is_none());assert_eq!(off.proxy.credential_ref,reference);
    }
    #[test]
    fn proxy_edits_do_not_reuse_credentials_for_a_different_destination() {
        let root = test_dir();
        let mut initial = draft("credential binding");
        initial.proxy = ProxyConfig {
            mode: "http".into(),
            host: "original.example".into(),
            port: 8080,
            username: "original-user".into(),
            credential_ref: None,
            pre_proxy: None,
            api_url: None,
            node: None,
        };
        let mut original = create(root.path(), initial.clone(), None).unwrap();
        // A missing secret must never be read when the destination changes.
        original.proxy.credential_ref = Some(identifier());
        let mut stored = load(root.path()).unwrap();
        *stored.iter_mut().find(|p| p.id == original.id).unwrap() = original.clone();
        save(root.path(), &stored).unwrap();
        for field in [
            "host", "port", "mode", "username", "direct", "system", "clear", "api",
        ] {
            let mut changed = initial.clone();
            match field {
                "host" => changed.proxy.host = "different.example".into(),
                "port" => changed.proxy.port = 9090,
                "mode" => changed.proxy.mode = "socks5".into(),
                "username" => changed.proxy.username = "different-user".into(),
                "direct" => changed.proxy.mode = "direct".into(),
                "system" => changed.proxy.mode = "system".into(),
                "clear" => changed.clear_password = true,
                "api" => changed.proxy.api_url = Some("https://provider.invalid/extract".into()),
                _ => unreachable!(),
            }
            let outbound = draft_upstream(root.path(), Some(&original.id), &changed)
                .unwrap_or_else(|e| panic!("{field} attempted to read the previous secret: {e}"));
            assert!(outbound.password.is_empty(), "{field}");
            let copied = create(root.path(), changed.clone(), Some(&original.id))
                .unwrap_or_else(|e| panic!("{field} cloned the previous secret: {e}"));
            assert!(copied.proxy.credential_ref.is_none(), "{field}");
            let saved = update(root.path(), &original.id, changed).unwrap();
            assert!(saved.proxy.credential_ref.is_none(), "{field}");
            // Reset the original for the next independently changed identity field.
            let mut stored = load(root.path()).unwrap();
            *stored.iter_mut().find(|p| p.id == original.id).unwrap() = original.clone();
            save(root.path(), &stored).unwrap();
        }
        let same = update(root.path(), &original.id, initial).unwrap();
        assert_eq!(same.proxy.credential_ref, original.proxy.credential_ref);
    }

    #[test]
    #[ignore = "Uses the native OS credential store; run explicitly on Windows/macOS"]
    fn native_proxy_credentials_stay_bound_through_edit_test_and_clone() {
        let root = test_dir();
        let mut cleanup = Vec::new();
        let mut original_draft = draft("credential binding native");
        original_draft.proxy = ProxyConfig {
            mode: "http".into(),
            host: "original.example".into(),
            port: 8080,
            username: "original-user".into(),
            credential_ref: None,
            pre_proxy: None,
            api_url: None,
            node: None,
        };
        original_draft.password = Some("synthetic-original-secret".into());
        let original = create(root.path(), original_draft.clone(), None).unwrap();
        cleanup.push(PendingCredentials {
            id: original.id.clone(),
            references: original.credential_refs.clone(),
            committed: false,
        });
        original_draft.password = None;
        assert_eq!(
            draft_upstream(root.path(), Some(&original.id), &original_draft)
                .unwrap()
                .password,
            "synthetic-original-secret"
        );
        let copied = create(root.path(), original_draft.clone(), Some(&original.id)).unwrap();
        cleanup.push(PendingCredentials {
            id: copied.id.clone(),
            references: copied.credential_refs.clone(),
            committed: false,
        });
        assert_eq!(
            upstream(&copied, false).unwrap().password,
            "synthetic-original-secret"
        );
        assert_ne!(copied.proxy.credential_ref, original.proxy.credential_ref);
        mark_started(root.path(), &original.id, None, None).unwrap();
        let mut changed = original_draft;
        changed.proxy.host = "different.example".into();
        assert!(draft_upstream(root.path(), Some(&original.id), &changed)
            .unwrap()
            .password
            .is_empty());
        let copied = create(root.path(), changed.clone(), Some(&original.id)).unwrap();
        cleanup.push(PendingCredentials {
            id: copied.id.clone(),
            references: copied.credential_refs.clone(),
            committed: false,
        });
        assert!(upstream(&copied, false).unwrap().password.is_empty());
        let saved = update(root.path(), &original.id, changed.clone()).unwrap();
        assert!(upstream(&saved, false).unwrap().password.is_empty());
        assert_eq!(
            upstream(&saved, true).unwrap().password,
            "synthetic-original-secret"
        );
        changed.password = Some("synthetic-replacement-secret".into());
        let replacement = update(root.path(), &original.id, changed.clone()).unwrap();
        cleanup.push(PendingCredentials {
            id: replacement.id.clone(),
            references: replacement.credential_refs.clone(),
            committed: false,
        });
        assert_eq!(
            upstream(&replacement, false).unwrap().password,
            "synthetic-replacement-secret"
        );
        assert_eq!(
            draft_upstream(root.path(), Some(&original.id), &changed)
                .unwrap()
                .password,
            "synthetic-replacement-secret"
        );
        assert_eq!(
            upstream(&replacement, true).unwrap().password,
            "synthetic-original-secret"
        );
        let references: Vec<_> = cleanup
            .iter()
            .flat_map(|c| c.references.iter().map(|r| (c.id.clone(), r.clone())))
            .collect();
        drop(cleanup);
        for (id, reference) in references {
            assert!(matches!(
                secret_entry(&id, &reference).unwrap().get_password(),
                Err(keyring::Error::NoEntry)
            ));
        }
    }

    #[test]
    fn proxy_password_is_not_serialized() {
        let config = ProxyConfig {
            mode: "http".into(),
            host: "localhost".into(),
            port: 8080,
            username: "user".into(),
            credential_ref: Some(identifier()),
            pre_proxy: None,
            api_url: None,
            node: None,
        };
        let bytes = serde_json::to_string(&config).unwrap();
        assert!(!bytes.contains("password"));
        assert!(validate(&draft(" ")).is_err());
    }
    #[test]
    fn unused_credentials_do_not_block_direct_mode_and_registry_rejects_aliasing() {
        let root = test_dir();
        let mut p = create(root.path(), draft("direct"), None).unwrap();
        p.proxy.mode = "direct".into();
        p.proxy.username = "old user".into();
        p.proxy.credential_ref = Some(identifier());
        assert!(upstream(&p, false).unwrap().password.is_empty());
        let mut list = load(root.path()).unwrap();
        list[0].browser = "firefox".into();
        save(root.path(), &list).unwrap();
        assert!(load(root.path()).is_err());
        assert!(!valid_id(&"A".repeat(32)));
        let mut d = draft("large URL");
        d.preferences.startup_url = format!("https://example.com/{}", "x".repeat(4096));
        assert!(validate(&d).is_err());
    }
}
