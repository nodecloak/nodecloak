use crate::{engine, profiles, proxy::ProxyConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct PortableProxy {
    pub mode: String,
    pub host: String,
    pub port: u16,
    pub username: String,
    pub password: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub pre_proxy: Option<crate::proxy::LocalProxy>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub api_url: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub node: Option<crate::proxy::node::NodeOptions>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields, rename_all = "camelCase")]
pub struct Document {
    pub schema: u8,
    pub name: String,
    pub browser: String,
    pub notes: String,
    pub tags: Vec<String>,
    pub proxy: PortableProxy,
    pub preferences: profiles::Preferences,
    pub deleted: bool,
}
impl Document {
    pub fn export(p: &profiles::Profile) -> Result<Self, String> {
        if p.received_share.is_some() {
            return Err("收到的共享副本不能导出或关联个人同步".into());
        }
        let upstream = profiles::upstream(p, false)?;
        let mut prefs = p.preferences.clone();
        crate::regional::normalize_sources(&mut prefs);
        let advanced = &prefs.advanced;
        let enhanced = prefs.regional.enabled
            || advanced.hardware_concurrency > 0
            || advanced.viewport_width > 0
            || advanced.block_webgpu
            || advanced.block_speech
            || advanced.block_battery
            || advanced.canvas_protection
            || advanced.do_not_track != profiles::DoNotTrack::Inherit
            || advanced.web_rtc != profiles::WebRtcPolicy::Inherit
            || (advanced.block_webgl && p.browser != "firefox");
        Ok(Self {
            schema: if p.proxy.node.is_some() {
                6
            } else if p.proxy.api_url.is_some() {
                5
            } else if p.proxy.pre_proxy.is_some() {
                4
            } else if prefs.fingerprint.is_some() {
                3
            } else if enhanced {
                2
            } else {
                1
            },
            name: p.name.clone(),
            browser: p.browser.clone(),
            notes: p.notes.clone(),
            tags: p.tags.clone(),
            proxy: PortableProxy {
                mode: p.proxy.mode.clone(),
                host: p.proxy.host.clone(),
                port: p.proxy.port,
                username: p.proxy.username.clone(),
                password: upstream.password.clone(),
                pre_proxy: p.proxy.pre_proxy.clone(),
                api_url: p.proxy.api_url.clone(),
                node: p.proxy.node.clone(),
            },
            preferences: prefs,
            deleted: p.deleted_at.is_some(),
        })
    }
    pub fn draft(&self) -> profiles::Draft {
        profiles::Draft {
            name: self.name.clone(),
            browser: self.browser.clone(),
            notes: self.notes.clone(),
            tags: self.tags.clone(),
            proxy: ProxyConfig {
                mode: self.proxy.mode.clone(),
                host: self.proxy.host.clone(),
                port: self.proxy.port,
                username: self.proxy.username.clone(),
                credential_ref: None,
                pre_proxy: self.proxy.pre_proxy.clone(),
                api_url: self.proxy.api_url.clone(),
                node: self.proxy.node.clone(),
            },
            password: Some(self.proxy.password.clone()),
            clear_password: self.proxy.password.is_empty(),
            preferences: self.preferences.clone(),
        }
    }
    pub fn parse(v: Value) -> Result<Self, String> {
        let mut doc: Self = serde_json::from_value(v).map_err(|_| "云端副本格式无效")?;
        if ![1, 2, 3, 4, 5, 6].contains(&doc.schema) {
            return Err("不支持的云端副本版本".into());
        };
        if doc.schema < 6 && (doc.proxy.node.is_some() || crate::proxy::node::is_node(&doc.proxy.mode)) {return Err("节点代理需要新版副本配置".into());}
        if doc.schema < 5 && doc.proxy.api_url.is_some() {
            return Err("API 代理需要新版副本配置".into());
        }
        if doc.schema < 4 && doc.proxy.pre_proxy.is_some() {
            return Err("链式代理需要新版副本配置".into());
        }
        if doc.schema < 3 && doc.preferences.fingerprint.is_some() {
            return Err("稳定指纹需要新版副本配置".into());
        }
        crate::regional::normalize_sources(&mut doc.preferences);
        profiles::validate(&doc.draft())?;
        if crate::proxy::node::is_node(&doc.proxy.mode) {crate::proxy::Upstream{config:doc.draft().proxy,password:doc.proxy.password.clone()}.validate()?;}
        Ok(doc)
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Account {
    pub id: String,
    pub email: String,
    pub name: String,
    pub avatar: String,
    #[serde(default)]
    pub google_linked: bool,
    #[serde(default)]
    pub has_password: bool,
    #[serde(default)]
    pub two_factor_enabled: bool,
    #[serde(default)]
    pub recent_auth: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Object {
    pub id: String,
    pub kind: String,
    pub revision: u64,
    pub epoch: u32,
    pub deleted: bool,
    #[serde(default)]
    pub payload: String,
    pub writer: String,
    pub generation: u64,
    pub updated_at: String,
}
impl Object {
    pub fn context<'a>(&'a self, account: &'a str) -> super::crypto::Context<'a> {
        super::crypto::Context {
            account,
            object: &self.id,
            kind: &self.kind,
            revision: self.revision,
            epoch: self.epoch,
            deleted: self.deleted,
            generation: self.generation,
            writer: &self.writer,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Binding {
    pub cloud_id: String,
    pub baseline: Option<Object>,
    pub session_baseline: Option<Object>,
    #[serde(default)]
    pub session_checkpoint: Option<Object>,
    pub generation: u64,
    pub session_dirty: bool,
    pub pending_session: Option<Object>,
    pub pending_config: Option<Object>,
    #[serde(default)]
    pub session_hash: Option<String>,
    #[serde(default)]
    pub file_stamp: Option<String>,
}
impl Binding {
    pub fn new(id: &str) -> Self {
        Self {
            cloud_id: id.into(),
            baseline: None,
            session_baseline: None,
            session_checkpoint: None,
            generation: 0,
            session_dirty: false,
            pending_session: None,
            pending_config: None,
            session_hash: None,
            file_stamp: None,
        }
    }
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Conflict {
    pub local_id: String,
    pub cloud_id: String,
    pub name: String,
    pub kind: String,
    pub fields: Vec<String>,
    pub remote_revision: u64,
}
impl Conflict {
    /// Persist the original records for older clients; project ordinary waiting
    /// conditions separately so only incompatible content asks for a choice.
    pub fn waiting(&self) -> bool {
        !self.fields.is_empty() && self.fields.iter().all(|field| matches!(field.as_str(),
            "close_browser" | "concurrent_write" | "config_changed" | "lease_held"
            | "lease_or_revision_changed" | "deletion_waiting_device"))
    }
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct AccountState {
    pub auto_sync: bool,
    pub session_sync: bool,
    pub setup_complete: bool,
    pub recovery_saved: bool,
    pub epoch: u32,
    pub authorized_device: Option<String>,
    pub pull_cloud: bool,
    pub bindings: BTreeMap<String, Binding>,
    pub excluded: BTreeSet<String>,
    pub detached: BTreeSet<String>,
    pub known_local: BTreeSet<String>,
    pub conflicts: Vec<Conflict>,
    pub last_sync: Option<String>,
    pub last_error: Option<String>,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(default, rename_all = "camelCase", deny_unknown_fields)]
pub struct Disk {
    pub account: Option<Account>,
    pub accounts: BTreeMap<String, AccountState>,
}
impl Disk {
    pub fn load(root: &Path) -> Result<Self, String> {
        let file = root.join("cloud/state.json");
        profiles::guard_tree(&file)?;
        match std::fs::read(&file) {
            Ok(b) => {
                if b.len() > 24 << 20 {
                    return Err("云端状态文件过大".into());
                };
                let disk: Self = serde_json::from_slice(&b).map_err(|_| "云端状态文件损坏")?;
                for (a, s) in &disk.accounts {
                    if uuid::Uuid::parse_str(a).is_err()
                        || s.bindings
                            .values()
                            .any(|b| uuid::Uuid::parse_str(&b.cloud_id).is_err())
                    {
                        return Err("云端标识无效".into());
                    }
                }
                Ok(disk)
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Self::default()),
            Err(_) => Err("无法读取云端状态".into()),
        }
    }
    pub fn save(&self, root: &Path) -> Result<(), String> {
        let file = root.join("cloud/state.json");
        profiles::guard_tree(&file)?;
        std::fs::create_dir_all(file.parent().unwrap()).map_err(|_| "无法创建云端状态目录")?;
        engine::atomic_write(
            &file,
            &serde_json::to_vec(self).map_err(|_| "无法序列化云端状态")?,
        )
    }
    pub fn can_attach(&self, account: &str, local_id: &str) -> Result<(), String> {
        if self
            .accounts
            .iter()
            .any(|(id, s)| id != account && s.bindings.contains_key(local_id))
        {
            Err("此副本关联了其他账号，请先复制副本再关联".into())
        } else {
            Ok(())
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn enhanced_settings_round_trip_with_a_new_schema_that_old_clients_reject() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let mut draft:profiles::Draft=serde_json::from_value(serde_json::json!({"name":"Enhanced","browser":"chrome","proxy":{"mode":"direct","host":"","port":0,"username":""},"preferences":{"language":"de-DE,de,en","privacy":true,"fontRestriction":false,"startupUrl":"about:blank","regional":{"enabled":true,"timezoneMode":"custom","timezone":"Europe/Berlin"},"advanced":{"hardwareConcurrency":4,"blockWebgpu":true}}})).unwrap();
        let mut profile = profiles::create(root.path(), draft.clone(), None).unwrap();
        profile.preferences.fingerprint = None; // A pre-fingerprint configuration.
        let exported = Document::export(&profile).unwrap();
        assert_eq!(exported.schema, 2);
        let parsed = Document::parse(serde_json::to_value(exported).unwrap()).unwrap();
        assert_eq!(parsed.preferences.regional.timezone, "Europe/Berlin");
        assert_eq!(parsed.preferences.advanced.hardware_concurrency, 4);
        draft.preferences = profiles::Preferences::default();
        let mut normal = profiles::create(root.path(), draft, None).unwrap();
        normal.preferences.fingerprint = None;
        assert_eq!(Document::export(&normal).unwrap().schema, 1);
    }
    #[test]
    fn cloud_profile_projection_excludes_device_paths_and_runtime() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let p = crate::profiles::load(dir.path()).unwrap().remove(0);
        let doc = Document::export(&p).unwrap();
        let value = serde_json::to_value(doc).unwrap();
        assert_eq!(value["browser"], "chrome");
        assert!(value.get("id").is_none());
        for key in [
            "relayPort",
            "activeProxy",
            "activeProxyEndpoint",
            "lastTest",
            "legacy",
            "credentialRefs",
            "lastRegion",
            "createdAt",
            "updatedAt",
        ] {
            assert!(value.get(key).is_none(), "{key}")
        }
        assert!(value["proxy"].get("credentialRef").is_none());
    }
    #[test]
    fn imported_profile_rejects_unknown_schema_and_platform_state() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let p = crate::profiles::load(dir.path()).unwrap().remove(0);
        let doc = Document::export(&p).unwrap();
        let mut v = serde_json::to_value(doc).unwrap();
        v["schema"] = serde_json::json!(99);
        assert!(Document::parse(v).is_err());
        let doc = Document::export(&p).unwrap();
        let mut v = serde_json::to_value(doc).unwrap();
        v["relayPort"] = serde_json::json!(1234);
        assert!(Document::parse(v).is_err());
    }
    #[test]
    fn new_account_cannot_implicitly_claim_previous_account_profiles() {
        let mut state = Disk::default();
        state
            .accounts
            .entry("first".into())
            .or_default()
            .bindings
            .insert("legacy-chrome".into(), Binding::new("cloud-a"));
        assert!(state.can_attach("second", "legacy-chrome").is_err());
        assert!(state.can_attach("first", "legacy-chrome").is_ok());
    }
    #[test]
    fn chain_proxy_round_trips_through_encrypted_document_schema_and_draft() {
        let dir=tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let mut profile=profiles::load(dir.path()).unwrap().remove(0);
        profile.proxy=serde_json::from_value(serde_json::json!({"mode":"socks5","host":"remote.invalid","port":1080,
            "preProxy":{"mode":"http","host":"127.0.0.1","port":7890}})).unwrap();
        let exported=Document::export(&profile).unwrap();
        assert_eq!(exported.schema,4,"required chain must not be silently dropped by older clients");
        let value=serde_json::to_value(exported).unwrap();
        assert_eq!(value["proxy"]["preProxy"]["port"],7890);
        let parsed=Document::parse(value.clone()).unwrap();
        assert_eq!(parsed.draft().proxy,profile.proxy);
        let mut legacy=value.clone();legacy["schema"]=serde_json::json!(3);
        assert!(Document::parse(legacy).is_err());
        let mut malformed=value;malformed["proxy"]["preProxy"]["port"]=serde_json::json!(0);
        assert!(Document::parse(malformed).is_err());
    }
    #[test]
    fn api_proxy_source_round_trips_without_runtime_endpoint_and_requires_schema5() {
        let root=tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let mut p=profiles::load(root.path()).unwrap().remove(0);
        p.proxy=serde_json::from_value(serde_json::json!({"mode":"http","apiUrl":"https://provider.invalid/extract?token=synthetic"})).unwrap();
        let doc=Document::export(&p).unwrap();assert_eq!(doc.schema,5);
        let value=serde_json::to_value(doc).unwrap();let parsed=Document::parse(value.clone()).unwrap();
        assert_eq!(parsed.draft().proxy,p.proxy);
        let mut old=value;old["schema"]=serde_json::json!(4);assert!(Document::parse(old).is_err());
    }

}
