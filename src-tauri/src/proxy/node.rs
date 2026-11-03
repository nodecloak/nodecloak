//! Typed connection options only. Credentials live in the profile keyring.
use super::{ProxyConfig, Upstream};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields, rename_all = "camelCase")]
pub struct NodeOptions {
    pub tls: String,
    #[serde(skip_serializing_if = "is_false")]
    pub insecure: bool,
    pub server_name: String,
    pub fingerprint: String,
    pub public_key: String,
    pub short_id: String,
    pub alpn: Vec<String>,
    pub flow: String,
    pub transport: String,
    pub host: String,
    pub path: String,
    pub service_name: String,
    pub security: String,
    pub alter_id: u16,
}
impl Default for NodeOptions {
    fn default() -> Self {
        Self {
            tls: "none".into(),
            insecure: false,
            server_name: String::new(),
            fingerprint: String::new(),
            public_key: String::new(),
            short_id: String::new(),
            alpn: vec![],
            flow: String::new(),
            transport: "tcp".into(),
            host: String::new(),
            path: String::new(),
            service_name: String::new(),
            security: "auto".into(),
            alter_id: 0,
        }
    }
}
impl NodeOptions {
    pub fn validate(&self, mode: &str) -> Result<(), String> {
        let bad = || "节点参数不支持或格式无效，请检查 TLS 和传输设置".to_owned();
        if !["vless", "vmess", "hysteria2"].contains(&mode)
            || !["none", "tls", "reality"].contains(&self.tls.as_str())
            || !["tcp", "ws", "httpupgrade", "grpc"].contains(&self.transport.as_str())
        {
            return Err(bad());
        }
        for value in [
            &self.server_name,
            &self.fingerprint,
            &self.public_key,
            &self.short_id,
            &self.host,
            &self.path,
            &self.service_name,
            &self.flow,
            &self.security,
        ] {
            if value.len() > 2048 || value.chars().any(char::is_control) {
                return Err(bad());
            }
        }
        if self
            .server_name
            .contains(|c: char| c.is_whitespace() || "/@?#\\".contains(c))
            || self.host.contains(|c: char| c.is_whitespace())
        {
            return Err(bad());
        }
        if ![
            "",
            "chrome",
            "firefox",
            "safari",
            "edge",
            "ios",
            "android",
            "random",
            "randomized",
        ]
        .contains(&self.fingerprint.as_str())
        {
            return Err(bad());
        }
        if self.alpn.len() > 8
            || self.alpn.iter().any(|v| {
                v.is_empty()
                    || v.len() > 64
                    || v.chars().any(|c| c.is_whitespace() || c.is_control())
            })
        {
            return Err(bad());
        }
        if !["auto", "aes-128-gcm", "chacha20-poly1305", "none", "zero"]
            .contains(&self.security.as_str())
        {
            return Err(bad());
        }
        if self.tls == "none"
            && (!self.server_name.is_empty()
                || !self.fingerprint.is_empty()
                || !self.alpn.is_empty())
        {
            return Err(bad());
        }
        if self.tls == "reality" {
            use base64::Engine;
            if mode != "vless"
                || self.server_name.is_empty()
                || self.fingerprint.is_empty()
                || base64::engine::general_purpose::URL_SAFE_NO_PAD
                    .decode(&self.public_key)
                    .map(|v| v.len() != 32)
                    .unwrap_or(true)
                || self.short_id.len() > 16
                || self.short_id.len() % 2 != 0
                || !self.short_id.bytes().all(|c| c.is_ascii_hexdigit())
            {
                return Err(bad());
            }
        } else if !self.public_key.is_empty() || !self.short_id.is_empty() {
            return Err(bad());
        }
        if !self.flow.is_empty()
            && (mode != "vless"
                || self.flow != "xtls-rprx-vision"
                || self.tls == "none"
                || self.transport != "tcp")
        {
            return Err(bad());
        }
        if mode == "vless" && (self.alter_id != 0 || self.security != "auto") {
            return Err(bad());
        }
        if (self.insecure && mode != "hysteria2")
            || (mode == "hysteria2" && (self.tls != "tls" || self.transport != "tcp"
                || !self.fingerprint.is_empty() || !self.flow.is_empty()
                || self.alter_id != 0 || self.security != "auto")) {
            return Err(bad());
        }
        if self.transport == "tcp"
            && (!self.path.is_empty() || !self.host.is_empty() || !self.service_name.is_empty())
        {
            return Err(bad());
        }
        if self.transport == "grpc" && (!self.path.is_empty() || !self.host.is_empty()) {
            return Err(bad());
        }
        if self.transport != "grpc" && !self.service_name.is_empty() {
            return Err(bad());
        }
        Ok(())
    }
}
fn is_false(value: &bool) -> bool { !*value }
pub fn outbound(up: &Upstream) -> Result<Value, String> {
    up.validate()?;
    let p = &up.config;
    let options = p.node.as_ref().ok_or("请先导入节点链接")?;
    let mut out = json!({"type":p.mode,"tag":"node","server":p.host,"server_port":p.port,"uuid":up.password,"network":"tcp"});
    if p.mode == "hysteria2" {
        out.as_object_mut().unwrap().remove("uuid");
        out["password"] = json!(up.password);
    }
    if p.mode == "vmess" {
        out["security"] = json!(options.security);
        out["alter_id"] = json!(options.alter_id);
    }
    if !options.flow.is_empty() {
        out["flow"] = json!(options.flow);
    }
    if options.tls != "none" {
        let mut tls = json!({"enabled":true});
        if options.insecure { tls["insecure"] = json!(true); }
        if !options.server_name.is_empty() {
            tls["server_name"] = json!(options.server_name);
        }
        if !options.fingerprint.is_empty() {
            tls["utls"] = json!({"enabled":true,"fingerprint":options.fingerprint});
        }
        if !options.alpn.is_empty() {
            tls["alpn"] = json!(options.alpn);
        }
        if options.tls == "reality" {
            tls["reality"] =
                json!({"enabled":true,"public_key":options.public_key,"short_id":options.short_id});
        }
        out["tls"] = tls;
    }
    if options.transport != "tcp" {
        let mut transport = json!({"type":options.transport});
        if options.transport == "grpc" {
            transport["service_name"] = json!(options.service_name);
        } else {
            transport["path"] = json!(if options.path.is_empty() {
                "/"
            } else {
                &options.path
            });
            if !options.host.is_empty() {
                if options.transport == "ws" {
                    transport["headers"] = json!({"Host":options.host});
                } else {
                    transport["host"] = json!(options.host);
                }
            }
        }
        out["transport"] = transport;
    }
    if p.pre_proxy.is_some() {
        out["detour"] = json!("first-hop");
    }
    Ok(out)
}
pub fn config(up: &Upstream, port: u16, username: &str, password: &str) -> Result<Value, String> {
    let mut outbounds = vec![outbound(up)?];
    if let Some(first) = &up.config.pre_proxy {
        let mut out = json!({"type":if first.mode=="socks5"{"socks"}else{"http"},"tag":"first-hop","server":first.host,"server_port":first.port});
        if first.mode == "socks5" {
            out["version"] = json!("5");
        }
        if first.mode == "https" {
            out["tls"] = json!({"enabled":true,"server_name":first.host});
        }
        outbounds.push(out);
    }
    Ok(
        json!({"log":{"level":"info","timestamp":false,"disabled":false},"dns":{"servers":[{"type":"local","tag":"bootstrap"}]},"inbounds":[{"type":"http","tag":"profile","listen":"127.0.0.1","listen_port":port,"users":[{"username":username,"password":password}],"set_system_proxy":false}],"outbounds":outbounds,"route":{"final":"node","default_domain_resolver":"bootstrap","rules":[{"domain":["nodecloak-core-health.invalid"],"action":"reject"}]}}),
    )
}
pub fn is_node(mode: &str) -> bool {
    matches!(mode, "vless" | "vmess" | "hysteria2")
}
pub fn validate_config(p: &ProxyConfig) -> Result<(), String> {
    if is_node(&p.mode) {
        if p.mode == "hysteria2" && p.pre_proxy.as_ref().is_some_and(|first| first.mode != "socks5") {
            return Err("Hysteria2 需要支持 UDP 的 SOCKS5 前置代理".into());
        }
        if p.api_url.is_some() || !p.username.is_empty() {
            return Err("节点代理不支持 API 提取或代理用户名".into());
        }
        p.node.as_ref().ok_or("请先导入节点链接")?.validate(&p.mode)
    } else if p.node.is_some() {
        Err("普通代理不能附带节点参数".into())
    } else {
        Ok(())
    }
}
