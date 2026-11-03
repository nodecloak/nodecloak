use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// QueryFullProcessImageName returns a long image path. Resolve NTFS short names
/// and file aliases before matching so selecting an existing executable cannot
/// silently become a direct connection merely because TEMP used an 8.3 path.
pub fn canonical_executable(path: &str) -> String {
    #[cfg(windows)]
    if let Ok(file) = std::fs::canonicalize(path) {
        let value = file.to_string_lossy();
        return value.strip_prefix(r"\\?\").unwrap_or(&value).to_owned();
    }
    path.to_owned()
}

pub fn normalize(path: &str) -> String {
    path.strip_prefix(r"\\?\")
        .unwrap_or(path)
        .replace('/', "\\")
        .to_lowercase()
}
pub fn identify(path: &str) -> Option<&'static str> {
    let path = normalize(path);
    if path.as_bytes().get(1) == Some(&b':')
        && path
            .get(2..)
            .is_some_and(|p| p.starts_with(r"\program files\windowsapps\"))
    {
        let tail = path.split_once(r"\windowsapps\").unwrap().1;
        let package = tail.split('\\').next().unwrap_or("");
        if package.starts_with("openai.codex_")
            && package.ends_with("__2p2nqsd0c76g0")
            && path.ends_with(r"\chatgpt.exe")
        {
            return Some("codex");
        }
        if (package.starts_with("openai.chatgpt_")
            || package.starts_with("openai.chatgpt-desktop_"))
            && package.ends_with("__2p2nqsd0c76g0")
            && path.ends_with(r"\chatgpt.exe")
        {
            return Some("chatgpt");
        }
        if (package.starts_with("anthropic.claude_") || package.starts_with("claude_"))
            && path.ends_with(r"\claude.exe")
        {
            return Some("claude");
        }
    }
    if path.ends_with(r"\codex.exe")
        && (path.contains(r"\appdata\local\openai\codex\") || path.contains(r"\@openai\codex\"))
    {
        return Some("codex");
    }
    if path.ends_with(r"\claude.exe")
        && (path.contains(r"\appdata\local\anthropicclaude\")
            || path.contains(r"\appdata\local\programs\claude\"))
    {
        return Some("claude");
    }
    None
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct DetectedApp {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub running: u32,
}
pub fn empty_apps() -> Vec<DetectedApp> {
    [
        ("codex", "Codex"),
        ("chatgpt", "ChatGPT"),
        ("claude", "Claude"),
    ]
    .into_iter()
    .map(|(id, name)| DetectedApp {
        id: id.into(),
        name: name.into(),
        paths: vec![],
        running: 0,
    })
    .collect()
}
pub fn discover() -> Result<Vec<DetectedApp>, String> {
    let mut apps = empty_apps();
    #[cfg(windows)]
    {
        use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
        let mut system = System::new();
        system.refresh_processes_specifics(
            ProcessesToUpdate::All,
            true,
            ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
        );
        for process in system.processes().values() {
            if let Some(path) = process.exe().and_then(|p| p.to_str()) {
                add(&mut apps, path, true);
            }
        }
        // Package identities distinguish the two OpenAI products even when their
        // executable names coincide. Only fixed, read-only discovery is executed.
        let script="@(Get-AppxPackage | Where-Object {$_.Name -match '^(OpenAI\\.(Codex|ChatGPT)|Anthropic\\.Claude|Claude)'} | Select-Object Name,InstallLocation,PublisherId) | ConvertTo-Json -Compress";
        if let Ok(output) = crate::platform::command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .output()
        {
            if output.status.success() && output.stdout.len() < 65536 {
                if let Ok(value) = serde_json::from_slice::<serde_json::Value>(&output.stdout) {
                    let packages: Vec<&serde_json::Value> = if value.is_object() {
                        vec![&value]
                    } else {
                        value
                            .as_array()
                            .map(|v| v.iter().collect())
                            .unwrap_or_default()
                    };
                    for package in packages.into_iter().take(32) {
                        let name = package["Name"].as_str().unwrap_or("").to_lowercase();
                        let publisher = package["PublisherId"].as_str().unwrap_or("");
                        let id = if name == "openai.codex" && publisher == "2p2nqsd0c76g0" {
                            Some("codex")
                        } else if ["openai.chatgpt", "openai.chatgpt-desktop"]
                            .contains(&name.as_str())
                            && publisher == "2p2nqsd0c76g0"
                        {
                            Some("chatgpt")
                        } else if ["anthropic.claude", "claude"].contains(&name.as_str()) {
                            Some("claude")
                        } else {
                            None
                        };
                        if let (Some(id), Some(folder)) = (id, package["InstallLocation"].as_str())
                        {
                            let exe = if id == "claude" {
                                "Claude.exe"
                            } else {
                                "ChatGPT.exe"
                            };
                            for suffix in [exe.to_owned(), format!("app/{exe}")] {
                                let file = std::path::Path::new(folder).join(suffix);
                                if file.is_file() {
                                    if let Some(app) = apps.iter_mut().find(|a| a.id == id) {
                                        app.paths.push(file.to_string_lossy().into());
                                    }
                                }
                            }
                        }
                    }
                }
            }
        }
        if let Some(local) = std::env::var_os("LOCALAPPDATA") {
            for suffix in ["OpenAI/Codex/bin", "AnthropicClaude", "Programs/Claude"] {
                scan(&mut apps, &std::path::PathBuf::from(&local).join(suffix), 3);
            }
        }
        for app in &mut apps {
            app.running = system
                .processes()
                .values()
                .filter(|p| {
                    p.exe().and_then(|p| p.to_str()).is_some_and(|path| {
                        app.paths
                            .iter()
                            .any(|known| normalize(known) == normalize(path))
                    })
                })
                .count() as u32;
        }
    }
    for app in &mut apps {
        let mut seen = BTreeSet::new();
        app.paths.retain(|p| seen.insert(normalize(p)));
    }
    Ok(apps)
}
#[cfg(windows)]
fn add(apps: &mut [DetectedApp], path: &str, running: bool) {
    if let Some(app) = identify(path).and_then(|id| apps.iter_mut().find(|a| a.id == id)) {
        if running {
            app.running += 1;
        }
        app.paths.push(path.into());
    }
}
#[cfg(windows)]
fn scan(apps: &mut [DetectedApp], folder: &std::path::Path, depth: u8) {
    if depth == 0 {
        return;
    }
    if let Ok(entries) = std::fs::read_dir(folder) {
        for entry in entries.take(128).flatten() {
            let Ok(kind) = entry.file_type() else {
                continue;
            };
            if kind.is_symlink() {
                continue;
            }
            if kind.is_dir() {
                scan(apps, &entry.path(), depth - 1);
            } else if kind.is_file() {
                if let Some(path) = entry.path().to_str() {
                    add(apps, path, false);
                }
            }
        }
    }
}
