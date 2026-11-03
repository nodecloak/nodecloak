use super::discovery::normalize;
use super::model::*;
use crate::proxy::{self, Upstream};
use serde_json::{json, Value};
use std::collections::HashSet;

pub const INTERFACE_PREFIX: &str = "NCApps-";
pub fn valid_id(id: &str) -> bool {
    uuid::Uuid::parse_str(id).is_ok()
}
pub fn interface_name(session: &str) -> Result<String, String> {
    let id = uuid::Uuid::parse_str(session).map_err(|_| "应用代理会话标识无效")?;
    Ok(format!(
        "{INTERFACE_PREFIX}{}",
        &id.simple().to_string()[..8]
    ))
}
pub fn path_pattern(path: &str) -> String {
    fn escaped(value: &str) -> String {
        let mut out = String::new();
        for c in value.chars() {
            if "\\.+*?()[]{}^$|".contains(c) {
                out.push('\\');
            }
            out.push(c);
        }
        out
    }
    let normalized = normalize(path);
    if super::discovery::identify(&normalized).is_some() {
        if let Some((root, tail)) = normalized.split_once(r"\windowsapps\") {
            if let Some((package, suffix)) = tail.split_once('\\') {
                let parts: Vec<_> = package.split('_').collect();
                if parts.len() == 5
                    && parts[1].split('.').count() == 4
                    && parts[1]
                        .split('.')
                        .all(|v| !v.is_empty() && v.bytes().all(|b| b.is_ascii_digit()))
                {
                    return format!(
                        "(?i)^{}[0-9]+\\.[0-9]+\\.[0-9]+\\.[0-9]+{}$",
                        escaped(&format!("{root}\\windowsapps\\{}_", parts[0])),
                        escaped(&format!("_{}__{}\\{suffix}", parts[2], parts[4]))
                    );
                }
            }
        }
        if let Some((root, tail)) = normalized.split_once(r"\openai\codex\bin\") {
            if let Some((revision, exe)) = tail.split_once('\\') {
                if (4..=64).contains(&revision.len())
                    && revision.bytes().all(|b| b.is_ascii_hexdigit())
                    && exe == "codex.exe"
                {
                    return format!(
                        "(?i)^{}[0-9a-f]{{4,64}}{}$",
                        escaped(&format!("{root}\\openai\\codex\\bin\\")),
                        escaped(r"\codex.exe")
                    );
                }
            }
        }
    }
    exact_pattern(path)
}
pub fn exact_pattern(path: &str) -> String {
    let normalized = normalize(path);
    let mut pattern = String::from("(?i)^");
    for c in normalized.chars() {
        if "\\.+*?()[]{}^$|".contains(c) {
            pattern.push('\\');
        }
        pattern.push(c);
    }
    pattern.push('$');
    pattern
}
pub fn valid_path(path: &str) -> bool {
    let path = normalize(path);
    path.as_bytes().get(1) == Some(&b':')
        && path.as_bytes().get(2) == Some(&b'\\')
        && path.as_bytes().first().is_some_and(u8::is_ascii_alphabetic)
        && path.len() <= 1024
        && path.ends_with(".exe")
        && !path[2..].contains([':', '*', '?', '"', '<', '>', '|'])
        && !path.chars().any(char::is_control)
        && !path.split('\\').any(|p| p == ".." || p == ".")
        && ![
            "nodecloak.exe",
            "sing-box.exe",
            "msedgewebview2.exe",
            "svchost.exe",
            "system",
        ]
        .contains(&path.rsplit('\\').next().unwrap_or(""))
        && !std::env::current_exe()
            .ok()
            .and_then(|p| p.to_str().map(normalize))
            .is_some_and(|current| current == path)
}
pub fn compile(
    rules: &[ResolvedRule],
    proxies: &[ResolvedProxy],
    port: u16,
    secret: &str,
    session: &str,
) -> Result<Value, String> {
    if rules.is_empty()
        || rules.len() > 64
        || proxies.is_empty()
        || proxies.len() > 32
        || port == 0
        || secret.len() != 64
        || !secret.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("请至少为一个已安装应用配置代理".into());
    }
    let mut ids = HashSet::new();
    let mut outbounds = vec![json!({"type":"direct","tag":"direct"})];
    for proxy in proxies {
        if !valid_id(&proxy.id) || !ids.insert(proxy.id.clone()) {
            return Err("应用代理节点标识重复或无效".into());
        }
        let config = &proxy.config;
        if config.api_url.is_some() || !config.custom() {
            return Err("应用代理需要固定代理地址或节点链接".into());
        }
        let up = Upstream {
            config: config.clone(),
            password: proxy.password.clone(),
        };
        up.validate()?;
        let tag = format!("proxy-{}", proxy.id);
        let hop = format!("hop-{}", proxy.id);
        let mut out = if proxy::node::is_node(&config.mode) {
            let mut value = proxy::node::outbound(&up)?;
            // Browser relays deliberately carry TCP only. This independent TUN
            // also supports UDP when the chosen protocol/upstream supports it.
            value.as_object_mut().unwrap().remove("network");
            value
        } else {
            let mut value = json!({"type":if config.mode=="socks5"{"socks"}else{"http"},"server":config.host,"server_port":config.port});
            if config.mode == "socks5" {
                value["version"] = json!("5");
            }
            if config.mode == "https" {
                value["tls"] = json!({"enabled":true,"server_name":config.host});
            }
            if !config.username.is_empty() {
                value["username"] = json!(config.username);
            }
            if !proxy.password.is_empty() {
                value["password"] = json!(proxy.password);
            }
            value
        };
        out["tag"] = json!(tag);
        if let Some(first) = &config.pre_proxy {
            out["detour"] = json!(hop);
            let mut first_out = json!({"type":if first.mode=="socks5"{"socks"}else{"http"},"tag":hop,"server":first.host,"server_port":first.port});
            if first.mode == "socks5" {
                first_out["version"] = json!("5");
            }
            if first.mode == "https" {
                first_out["tls"] = json!({"enabled":true,"server_name":first.host});
            }
            outbounds.push(first_out);
        }
        outbounds.push(out);
    }
    let mut seen: Vec<(String, String, bool)> = vec![];
    let mut rule_ids = HashSet::new();
    let current = std::env::current_exe().map_err(|_| "无法定位应用代理控制程序")?;
    let mut routing = vec![
        json!({"process_path_regex":[path_pattern(&current.to_string_lossy())],"action":"route","outbound":"direct"}),
    ];
    for rule in rules {
        if !valid_id(&rule.id)
            || !rule_ids.insert(&rule.id)
            || !ids.contains(&rule.proxy_id)
            || rule.paths.is_empty()
            || rule.paths.len() > 32
        {
            return Err("应用代理规则无效或没有找到可执行程序".into());
        }
        let mut paths = vec![];
        let mut local = HashSet::new();
        for path in &rule.paths {
            if !valid_path(path) {
                return Err("请选择本机应用程序；系统及共享网络进程不能作为代理目标".into());
            }
            let pattern = if rule.exact_paths {
                exact_pattern(path)
            } else {
                path_pattern(path)
            };
            if local.insert(pattern.clone()) {
                let family = path_pattern(path);
                if seen.iter().any(|(other, other_family, exact)| {
                    other == &pattern || other_family == &family && (!exact || !rule.exact_paths)
                }) {
                    return Err("同一个程序不能同时使用两条代理规则".into());
                }
                seen.push((pattern.clone(), family, rule.exact_paths));
                paths.push(pattern);
            }
        }
        routing.push(json!({"process_path_regex":paths,"action":"route","outbound":format!("proxy-{}",rule.proxy_id)}));
    }
    Ok(json!({"log":{"level":"info","timestamp":false},
        "dns":{"servers":[{"type":"local","tag":"bootstrap"}]},
        "inbounds":[{"type":"tun","tag":"applications","interface_name":interface_name(session)?,"address":["172.29.255.1/30","fd64:6e6f:6465::1/126"],"mtu":1500,"auto_route":true,"strict_route":false,"dns_mode":"disabled","stack":"mixed"}],
        "outbounds":outbounds,"route":{"auto_detect_interface":true,"find_process":true,"default_domain_resolver":"bootstrap","rules":routing,"final":"direct"},
        "experimental":{"clash_api":{"external_controller":format!("127.0.0.1:{port}"),"secret":secret,"access_control_allow_origin":["http://nodecloak.invalid"],"access_control_allow_private_network":false}}
    }))
}
