//! Authenticated loopback ingress shared by Windows and WSL launchers.
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub(super) use windows::*;

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Ingress {
    pub app: String,
    pub proxy_id: String,
    pub port: u16,
    pub password: String,
}
impl Ingress {
    pub fn tag(&self) -> String {
        format!("wsl-{}", self.app)
    }
}

pub fn add_ingresses(
    config: &mut Value,
    ingresses: &[Ingress],
    controller: u16,
) -> Result<(), String> {
    let mut apps = std::collections::HashSet::new();
    let mut ports = std::collections::HashSet::new();
    if ingresses.len() > 2 {
        return Err("WSL 应用代理入口过多".into());
    }
    for ingress in ingresses {
        let outbound = format!("proxy-{}", ingress.proxy_id);
        if !["codex", "claude"].contains(&ingress.app.as_str())
            || !apps.insert(&ingress.app)
            || ingress.port < 1024
            || ingress.port == controller
            || !ports.insert(ingress.port)
            || ingress.password.len() != 64
            || !ingress.password.bytes().all(|b| b.is_ascii_hexdigit())
            || !super::config::valid_id(&ingress.proxy_id)
            || !config["outbounds"]
                .as_array()
                .is_some_and(|out| out.iter().any(|v| v["tag"] == outbound))
        {
            return Err("WSL 应用代理入口或节点无效".into());
        }
        config["inbounds"]
            .as_array_mut()
            .ok_or("应用代理配置无效")?
            .push(json!({
                "type":"http","tag":ingress.tag(),"listen":"127.0.0.1","listen_port":ingress.port,
                "users":[{"username":"nodecloak","password":ingress.password}]
            }));
        config["route"]["rules"]
            .as_array_mut()
            .ok_or("应用代理路由无效")?
            .insert(
                0,
                json!({"inbound":[ingress.tag()],"action":"route","outbound":outbound}),
            );
    }
    Ok(())
}

/// Stdio transport used only by the host-side WSL bridge. It is deliberately
/// unprivileged and cannot choose a LAN/remote address or install network rules.
#[cfg(windows)]
pub fn relay(port: u16) -> Result<(), String> {
    use std::io::{Read, Write};
    if port < 1024 {
        return Err("WSL 代理端口无效".into());
    }
    let mut socket = std::net::TcpStream::connect_timeout(
        &std::net::SocketAddr::from(([127, 0, 0, 1], port)),
        std::time::Duration::from_secs(3),
    )
    .map_err(|_| "WSL 代理不可用，已阻止直连")?;
    let mut output = socket.try_clone().map_err(|_| "无法创建 WSL 代理通道")?;
    std::thread::spawn(move || {
        let _ = std::io::copy(&mut std::io::stdin().lock(), &mut output);
        let _ = output.shutdown(std::net::Shutdown::Write);
    });
    let mut stdout = std::io::stdout().lock();
    let mut bytes = [0u8; 65536];
    loop {
        let count = socket.read(&mut bytes).map_err(|_| "WSL 代理连接中断")?;
        if count == 0 {
            break;
        }
        stdout
            .write_all(&bytes[..count])
            .and_then(|_| stdout.flush())
            .map_err(|_| "WSL 代理输出已关闭")?;
    }
    Ok(())
}

#[cfg(test)]
mod checks {
    use super::*;
    fn config() -> Value {
        json!({"inbounds":[],"route":{"rules":[{"action":"route","outbound":"direct"}]},"outbounds":[{"tag":"proxy-0b4fcb93-ec83-4f9b-996a-f89f7aa96025"}]})
    }
    fn ingress() -> Ingress {
        Ingress {
            app: "claude".into(),
            proxy_id: "0b4fcb93-ec83-4f9b-996a-f89f7aa96025".into(),
            port: 23333,
            password: "a".repeat(64),
        }
    }
    #[test]
    fn wsl_ingress_authenticates_on_loopback_and_precedes_every_direct_rule() {
        let mut config = config();
        add_ingresses(&mut config, &[ingress()], 24444).unwrap();
        assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
        assert_eq!(
            config["inbounds"][0]["users"][0]["password"],
            "a".repeat(64)
        );
        assert_eq!(
            config["route"]["rules"][0],
            json!({"inbound":["wsl-claude"],"action":"route","outbound":"proxy-0b4fcb93-ec83-4f9b-996a-f89f7aa96025"})
        );
    }
    #[test]
    fn invalid_or_duplicate_wsl_ingress_cannot_reach_a_direct_fallback() {
        for change in ["app", "node", "password", "port", "controller"] {
            let mut item = ingress();
            match change {
                "app" => item.app = "node".into(),
                "node" => item.proxy_id = uuid::Uuid::new_v4().to_string(),
                "password" => item.password.clear(),
                "port" => item.port = 0,
                _ => item.port = 24444,
            }
            assert!(
                add_ingresses(&mut config(), &[item], 24444).is_err(),
                "{change}"
            );
        }
        assert!(add_ingresses(&mut config(), &[ingress(), ingress()], 24444).is_err());
    }
}
