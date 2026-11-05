use super::model::*;
use std::path::Path;
pub trait Credentials {
    fn get(&self, id: &str, reference: &str) -> Result<String, String>;
    fn set(&self, id: &str, reference: &str, value: &str) -> Result<(), String>;
    fn remove(&self, id: &str, reference: &str);
}
pub struct NativeCredentials;
impl NativeCredentials {
    fn entry(id: &str, reference: &str) -> Result<keyring::Entry, String> {
        if !super::config::valid_id(id)
            || reference.len() != 32
            || !reference.bytes().all(|b| b.is_ascii_hexdigit())
        {
            return Err("应用代理凭据标识无效".into());
        }
        keyring::Entry::new(
            "com.nodecloak.application-proxy",
            &format!("{id}:{reference}"),
        )
        .map_err(|_| "无法访问应用代理凭据存储".into())
    }
}
impl Credentials for NativeCredentials {
    fn get(&self, id: &str, reference: &str) -> Result<String, String> {
        Self::entry(id, reference)?
            .get_password()
            .map_err(|_| "无法读取此应用代理的凭据，请重新填写代理链接".into())
    }
    fn set(&self, id: &str, reference: &str, value: &str) -> Result<(), String> {
        Self::entry(id, reference)?
            .set_password(value)
            .map_err(|_| "无法保存应用代理凭据".into())
    }
    fn remove(&self, id: &str, reference: &str) {
        if let Ok(entry) = Self::entry(id, reference) {
            let _ = entry.delete_credential();
        }
    }
}
pub fn validate(settings: &Settings) -> Result<(), String> {
    use std::collections::HashSet;
    if settings.version != 1 || settings.proxies.len() > 32 || settings.rules.len() > 64 {
        return Err("应用代理配置版本或数量无效".into());
    }
    let mut nodes = HashSet::new();
    let mut rules = HashSet::new();
    let mut targets = HashSet::new();
    for node in &settings.proxies {
        if !super::config::valid_id(&node.id)
            || !nodes.insert(&node.id)
            || node.name.trim().is_empty()
            || node.name.len() > 180
            || node.name.chars().any(char::is_control)
            || !node.proxy.custom()
            || node.proxy.api_url.is_some()
        {
            return Err("应用代理节点配置无效".into());
        }
        node.proxy.validate()?;
        if let Some(reference) = &node.proxy.credential_ref {
            if reference.len() != 32 || !reference.bytes().all(|b| b.is_ascii_hexdigit()) {
                return Err("应用代理凭据标识无效".into());
            }
        }
    }
    for rule in &settings.rules {
        if !super::config::valid_id(&rule.id)
            || !rules.insert(&rule.id)
            || !["codex", "chatgpt", "claude", "custom"].contains(&rule.app.as_str())
            || ((rule.app == "custom" || !rule.executable.is_empty())
                && !super::config::valid_path(&rule.executable)
                && !super::macos_bundle::valid_selection(&rule.executable))
            || (rule.enabled && rule.proxy_id.is_none())
            || rule.proxy_id.as_ref().is_some_and(|id| !nodes.contains(id))
        {
            return Err("应用代理规则或指定节点无效".into());
        }
        let target = if rule.app == "custom" {
            super::discovery::normalize(&rule.executable)
        } else {
            rule.app.clone()
        };
        if !targets.insert(target) {
            return Err("同一个应用只能配置一条代理规则".into());
        }
    }
    Ok(())
}
pub fn load(root: &Path) -> Result<Settings, String> {
    let path = root.join("application-proxy.json");
    crate::profiles::guard_tree(&path)?;
    if !path.exists() {
        return Ok(Settings::default());
    }
    let meta = std::fs::metadata(&path).map_err(|_| "无法读取应用代理配置")?;
    if !meta.is_file() || meta.len() > 1_048_576 {
        return Err("应用代理配置文件无效或过大".into());
    }
    let bytes = std::fs::read(&path).map_err(|_| "无法读取应用代理配置")?;
    let mut settings: Settings =
        serde_json::from_slice(&bytes).map_err(|_| "应用代理配置损坏，已保留原文件")?;
    validate(&settings)?;
    for node in &mut settings.proxies {
        node.has_password = node.proxy.credential_ref.is_some();
    }
    Ok(settings)
}
fn persist(root: &Path, settings: &Settings) -> Result<(), String> {
    validate(settings)?;
    let path = root.join("application-proxy.json");
    crate::profiles::guard_tree(&path)?;
    crate::engine::atomic_write(
        &path,
        &serde_json::to_vec_pretty(settings).map_err(|_| "无法保存应用代理配置")?,
    )
    .map_err(|_| "无法保存应用代理配置，原配置已保留".into())
}
pub fn save_node(
    root: &Path,
    mut draft: DestinationDraft,
    credentials: &impl Credentials,
) -> Result<Settings, String> {
    let mut settings = load(root)?;
    let old = if let Some(id) = &draft.id {
        Some(
            settings
                .proxies
                .iter()
                .find(|p| &p.id == id)
                .cloned()
                .ok_or("此应用代理节点已不存在")?,
        )
    } else {
        None
    };
    let id = old
        .as_ref()
        .map(|p| p.id.clone())
        .unwrap_or_else(|| uuid::Uuid::new_v4().to_string());
    draft.proxy.credential_ref = None;
    let identity_unchanged = old.as_ref().is_some_and(|p| {
        p.proxy.mode == draft.proxy.mode
            && p.proxy.host.eq_ignore_ascii_case(&draft.proxy.host)
            && p.proxy.port == draft.proxy.port
            && p.proxy.username == draft.proxy.username
    });
    let supplied = draft.password.take().filter(|s| !s.is_empty());
    let old_ref = old.as_ref().and_then(|p| p.proxy.credential_ref.clone());
    let retained = if supplied.is_none() && !draft.clear_password && identity_unchanged {
        old_ref.clone()
    } else {
        None
    };
    let password = if let Some(value) = &supplied {
        value.clone()
    } else if let Some(reference) = &retained {
        credentials.get(&id, reference)?
    } else {
        String::new()
    };
    crate::proxy::Upstream {
        config: draft.proxy.clone(),
        password,
    }
    .validate()?;
    let reference = supplied
        .as_ref()
        .map(|_| uuid::Uuid::new_v4().simple().to_string())
        .or(retained);
    draft.proxy.credential_ref = reference.clone();
    let node = Destination {
        id: id.clone(),
        name: draft.name.trim().into(),
        proxy: draft.proxy,
        has_password: reference.is_some(),
    };
    if let Some(index) = settings.proxies.iter().position(|p| p.id == id) {
        settings.proxies[index] = node;
    } else {
        settings.proxies.push(node);
    }
    validate(&settings)?;
    if let (Some(secret), Some(reference)) = (&supplied, &reference) {
        credentials.set(&id, reference, secret)?;
    }
    if let Err(error) = persist(root, &settings) {
        if supplied.is_some() {
            if let Some(reference) = &reference {
                credentials.remove(&id, reference);
            }
        }
        return Err(error);
    }
    if old_ref != reference {
        if let Some(reference) = old_ref {
            credentials.remove(&id, &reference);
        }
    }
    Ok(settings)
}
pub fn save_rules(root: &Path, rules: Vec<Rule>) -> Result<Settings, String> {
    let mut settings = load(root)?;
    settings.rules = rules;
    persist(root, &settings)?;
    Ok(settings)
}
pub fn delete_node(
    root: &Path,
    id: &str,
    credentials: &impl Credentials,
) -> Result<Settings, String> {
    let mut settings = load(root)?;
    let old = settings
        .proxies
        .iter()
        .find(|p| p.id == id)
        .cloned()
        .ok_or("此应用代理节点已不存在")?;
    settings.proxies.retain(|p| p.id != id);
    for rule in &mut settings.rules {
        if rule.proxy_id.as_deref() == Some(id) {
            rule.proxy_id = None;
            rule.enabled = false;
        }
    }
    persist(root, &settings)?;
    if let Some(reference) = &old.proxy.credential_ref {
        credentials.remove(id, reference);
    }
    Ok(settings)
}
pub fn resolved_proxies(
    settings: &Settings,
    credentials: &impl Credentials,
) -> Result<Vec<ResolvedProxy>, String> {
    let ids: std::collections::HashSet<_> = settings
        .rules
        .iter()
        .filter(|r| r.enabled)
        .filter_map(|r| r.proxy_id.as_deref())
        .collect();
    settings
        .proxies
        .iter()
        .filter(|p| ids.contains(p.id.as_str()))
        .map(|p| {
            let password = if let Some(reference) = &p.proxy.credential_ref {
                credentials.get(&p.id, reference)?
            } else {
                String::new()
            };
            let mut config = p.proxy.clone();
            config.credential_ref = None;
            Ok(ResolvedProxy {
                id: p.id.clone(),
                config,
                password,
            })
        })
        .collect()
}
