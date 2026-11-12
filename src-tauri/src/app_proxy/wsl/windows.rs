use super::Ingress;
use crate::app_proxy::model::Settings;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use std::path::Path;
const AGENT: &str = include_str!("agent.py");

#[derive(Clone, Serialize, Deserialize)]
pub struct Distribution {
    pub name: String,
    pub installed: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Registry {
    names: Vec<String>,
    protected: bool,
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 256
        && !name.starts_with('-')
        && !name.contains(['/', '\\'])
        && !name.chars().any(char::is_control)
}
fn load(root: &Path) -> Result<Registry, String> {
    let path = root.join("application-proxy-wsl.json");
    crate::profiles::guard_tree(&path)?;
    if !path.exists() {
        return Ok(Registry::default());
    }
    if std::fs::metadata(&path)
        .map_err(|_| "无法读取 WSL 接入状态")?
        .len()
        > 65536
    {
        return Err("WSL 接入状态过大".into());
    }
    let state: Registry =
        serde_json::from_slice(&std::fs::read(path).map_err(|_| "无法读取 WSL 接入状态")?)
            .map_err(|_| "WSL 接入状态损坏；请恢复配置后重试，未解除保护")?;
    if state.names.len() > 8 || state.names.iter().any(|n| !valid_name(n)) {
        return Err("WSL 接入记录无效".into());
    }
    Ok(state)
}
fn save(root: &Path, state: &Registry) -> Result<(), String> {
    let path = root.join("application-proxy-wsl.json");
    crate::profiles::guard_tree(&path)?;
    crate::engine::atomic_write(
        &path,
        &serde_json::to_vec(state).map_err(|_| "无法保存 WSL 接入状态")?,
    )
}
pub fn protected(root: &Path) -> Result<bool, String> {
    Ok(load(root)?.protected)
}

pub fn discover(root: &Path) -> Result<Vec<Distribution>, String> {
    let registry = load(root)?;
    let output = crate::platform::command("wsl.exe")
        .args(["--list", "--quiet"])
        .output();
    let mut names = vec![];
    if let Ok(output) = output {
        if output.status.success() && output.stdout.len() <= 65536 {
            let value = String::from_utf16_lossy(
                &output
                    .stdout
                    .chunks_exact(2)
                    .map(|v| u16::from_le_bytes([v[0], v[1]]))
                    .collect::<Vec<_>>(),
            );
            for line in value
                .trim_start_matches('\u{feff}')
                .lines()
                .map(str::trim)
                .filter(|s| valid_name(s))
                .take(64)
            {
                names.push(line.to_owned());
            }
        }
    }
    names.extend(registry.names.iter().cloned());
    names.sort();
    names.dedup();
    Ok(names
        .into_iter()
        .map(|name| Distribution {
            installed: registry.names.contains(&name),
            name,
        })
        .collect())
}

async fn manage(name: &str, action: &str, payload: Value) -> Result<Value, String> {
    use tokio::io::AsyncWriteExt;
    if !valid_name(name) {
        return Err("WSL 发行版名称无效".into());
    }
    let mut command = tokio::process::Command::from(crate::platform::command("wsl.exe"));
    command.args([
        "-d",
        name,
        "--exec",
        "python3",
        "-c",
        "import sys; source=sys.stdin.read(); exec(compile(source, 'nodecloak_wsl.py', 'exec'))",
        "--manage",
        action,
        &payload.to_string(),
    ]);
    command
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let mut child = command
        .spawn()
        .map_err(|_| format!("无法访问 WSL {name}；请先安装并启动该发行版"))?;
    let mut stdin = child.stdin.take().ok_or("无法配置 WSL 接入")?;
    stdin
        .write_all(AGENT.as_bytes())
        .await
        .map_err(|_| "无法发送 WSL 接入程序")?;
    drop(stdin);
    let output = tokio::time::timeout(std::time::Duration::from_secs(25), child.wait_with_output())
        .await
        .map_err(|_| format!("WSL {name} 响应超时，未解除断网保护"))?
        .map_err(|_| "WSL 接入任务中断")?;
    if output.stdout.len() > 65536 || output.stderr.len() > 65536 {
        return Err("WSL 接入输出异常".into());
    }
    if !output.status.success() {
        return Err(format!(
            "WSL {name}: {}",
            String::from_utf8_lossy(&output.stderr)
                .chars()
                .take(1000)
                .collect::<String>()
                .trim()
        ));
    }
    serde_json::from_slice(&output.stdout).map_err(|_| format!("WSL {name} 返回了无效状态"))
}
pub async fn install(root: &Path, name: &str) -> Result<(), String> {
    if !valid_name(name) {
        return Err("WSL 发行版名称无效".into());
    }
    let mut state = load(root)?;
    if state.protected {
        return Err("请先停止应用代理，再修改 WSL 接入".into());
    }
    if !state.names.contains(&name.to_owned()) && state.names.len() >= 8 {
        return Err("最多接入 8 个 WSL 发行版".into());
    }
    let exe = std::env::current_exe().map_err(|_| "无法定位 NodeCloak 程序")?;
    if !state.names.contains(&name.to_owned()) {
        state.names.push(name.to_owned());
    }
    save(root, &state)?;
    manage(name, "install", json!({"windowsExe":exe.to_string_lossy()})).await?;
    Ok(())
}
pub async fn remove(root: &Path, name: &str) -> Result<(), String> {
    let mut state = load(root)?;
    if state.protected {
        return Err("请先停止应用代理，再移除 WSL 接入".into());
    }
    if !state.names.contains(&name.to_owned()) {
        return Err("此 WSL 发行版尚未接入".into());
    }
    manage(name, "remove", json!({})).await?;
    state.names.retain(|n| n != name);
    save(root, &state)
}

pub async fn prepare(root: &Path, settings: &Settings) -> Result<Vec<Ingress>, String> {
    let mut registry = load(root)?;
    if registry.names.is_empty() {
        return Ok(vec![]);
    }
    let mut ingresses = vec![];
    let mut reservations = vec![];
    for rule in settings.rules.iter().filter(|r| {
        r.enabled && ["codex", "claude"].contains(&r.app.as_str()) && r.executable.is_empty()
    }) {
        let proxy_id = rule.proxy_id.clone().ok_or("请为 WSL 应用选择对应节点")?;
        let listener =
            std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配 WSL 应用代理入口")?;
        let port = listener
            .local_addr()
            .map_err(|_| "无法读取 WSL 应用代理入口")?
            .port();
        reservations.push(listener);
        ingresses.push(Ingress {
            app: rule.app.clone(),
            proxy_id,
            port,
            password: format!(
                "{}{}",
                uuid::Uuid::new_v4().simple(),
                uuid::Uuid::new_v4().simple()
            ),
        });
    }
    if ingresses.is_empty() {
        return Ok(ingresses);
    }
    // Persist intent before touching any distro: a partial preparation or GUI
    // crash must still expose a Stop/recovery action on the next app startup.
    registry.protected = true;
    save(root, &registry)?;
    let exe = std::env::current_exe().map_err(|_| "无法定位 NodeCloak 程序")?;
    let apps = ingresses
        .iter()
        .map(|i| (i.app.clone(), json!({"port":i.port,"password":i.password})))
        .collect::<serde_json::Map<_, _>>();
    for name in &registry.names {
        manage(
            name,
            "state",
            json!({"enabled":true,"apps":apps,"windowsExe":exe.to_string_lossy()}),
        )
        .await?;
    }
    Ok(ingresses)
}
pub async fn stop(root: &Path) -> Result<(), String> {
    let mut registry = load(root)?;
    if !registry.protected {
        return Ok(());
    }
    for name in &registry.names {
        manage(name, "state", json!({"enabled":false,"apps":{}})).await?;
    }
    registry.protected = false;
    save(root, &registry)
}

#[cfg(test)]
mod checks {
    #[tokio::test]
    async fn invalid_distribution_names_never_poison_saved_integration_state() {
        let temp = tempfile::tempdir().unwrap();
        for name in ["", "-d", "../Ubuntu", "Ubuntu\n--root"] {
            assert!(super::install(temp.path(), name).await.is_err());
            assert!(super::load(temp.path()).unwrap().names.is_empty());
        }
    }
}
