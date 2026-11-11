pub mod environment;
#[cfg(any(target_os = "linux", test))]
pub(crate) mod linux;
use crate::{engine, platform};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

#[derive(Clone, Copy, Deserialize, Serialize, PartialEq, Debug)]
#[serde(rename_all = "lowercase")]
pub enum Tool {
    Claude,
    Codex,
    Gemini,
    Shell,
}

impl Tool {
    pub fn command(self) -> Option<&'static str> {
        match self {
            Self::Claude => Some("claude"),
            Self::Codex => Some("codex"),
            Self::Gemini => Some("gemini"),
            Self::Shell => None,
        }
    }
    fn name(self) -> &'static str {
        match self {
            Self::Claude => "Claude Code",
            Self::Codex => "Codex CLI",
            Self::Gemini => "Gemini CLI",
            Self::Shell => "终端",
        }
    }
    fn slug(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
            Self::Gemini => "gemini",
            Self::Shell => "shell",
        }
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ToolStatus {
    pub id: Tool,
    pub installed: bool,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub tools: Vec<ToolStatus>,
    pub home_directory: String,
}

fn search_roots() -> Vec<PathBuf> {
    let home = platform::home_dir();
    let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    roots.extend([
        home.join(".local/bin"),
        home.join(".npm-global/bin"),
        home.join(".bun/bin"),
    ]);
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA") {
            roots.push(PathBuf::from(appdata).join("npm"));
        }
    } else {
        roots.extend([
            PathBuf::from("/opt/homebrew/bin"),
            PathBuf::from("/usr/local/bin"),
        ]);
    }
    roots
}

pub(crate) fn executable_file(file: &Path) -> bool {
    if !file.is_file() { return false; }
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        return file.metadata().is_ok_and(|m| m.permissions().mode() & 0o111 != 0);
    }
    #[cfg(not(unix))]
    true
}

fn find_in(tool: Tool, roots: &[PathBuf], windows: bool) -> Option<PathBuf> {
    let command = tool.command()?;
    let names = if windows {
        vec![format!("{command}.exe"), format!("{command}.cmd")]
    } else {
        vec![command.to_owned()]
    };
    roots.iter().find_map(|root| {
        names
            .iter()
            .map(|name| root.join(name))
            .find(|file| if windows { file.is_file() } else { executable_file(file) })
    })
}

pub fn catalog() -> Catalog {
    let roots = search_roots();
    Catalog {
        home_directory: platform::home_dir().display().to_string(),
        tools: [Tool::Claude, Tool::Codex, Tool::Gemini, Tool::Shell]
            .into_iter()
            .map(|id| ToolStatus {
                id,
                installed: id == Tool::Shell || find_in(id, &roots, cfg!(windows)).is_some(),
            })
            .collect(),
    }
}

pub fn open_guide(tool: Tool) -> Result<(), String> {
    let url = match tool {
        Tool::Claude => "https://code.claude.com/docs/en/setup",
        Tool::Codex => "https://learn.chatgpt.com/docs/codex/cli",
        Tool::Gemini => "https://geminicli.com/docs/get-started/installation/",
        Tool::Shell => return Err("普通终端无需安装 CLI。".into()),
    };
    platform::open_external(url)
}

fn legacy_launcher_contents(tool: Tool, windows: bool) -> String {
    if windows {
        let start = if let Some(command) = tool.command() {
            format!("$cli = Get-Command {command} -CommandType Application -ErrorAction SilentlyContinue | Select-Object -First 1\r\nif (-not $cli) {{ throw '{} is not installed or not on PATH.' }}\r\n& $cli.Source\r\n", tool.name())
        } else {
            "Write-Host 'NodeCloak terminal ready. Run claude, codex, gemini, or your preferred CLI.'\r\n".into()
        };
        format!("# NodeCloak managed terminal launcher\r\nparam([string]$WorkingDirectory = $env:USERPROFILE)\r\n$ErrorActionPreference = 'Stop'\r\n$env:TZ = 'Asia/Singapore'\r\n$env:LANG = 'en_US.UTF-8'\r\n$env:LC_ALL = 'en_US.UTF-8'\r\n$env:PATH = (Join-Path $env:USERPROFILE '.local/bin') + ';' + (Join-Path $env:USERPROFILE '.npm-global/bin') + ';' + (Join-Path $env:USERPROFILE '.bun/bin') + ';' + (Join-Path $env:APPDATA 'npm') + ';' + $env:PATH\r\nSet-Location -LiteralPath $WorkingDirectory\r\n{start}")
    } else {
        let start = if let Some(command) = tool.command() {
            format!("if ! command -v {command} >/dev/null 2>&1; then echo '{} is not installed or not on PATH.'; exit 1; fi\nexec {command}\n", tool.name())
        } else {
            "exec /bin/zsh -i\n".into()
        };
        format!("#!/bin/zsh\n# NodeCloak managed terminal launcher\nexport PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:$HOME/.bun/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"\nexport TZ=Asia/Singapore\nexport LANG=en_US.UTF-8\nexport LC_ALL=en_US.UTF-8\ncd -- \"${{1:-$HOME}}\" || exit 1\n{start}")
    }
}

fn launcher_contents(tool: Tool, windows: bool) -> String {
    let legacy = legacy_launcher_contents(tool, windows);
    if windows {
        legacy.replace("$env:TZ = 'Asia/Singapore'\r\n$env:LANG = 'en_US.UTF-8'\r\n$env:LC_ALL = 'en_US.UTF-8'\r\n", "")
            .replace("Set-Location -LiteralPath", "$environment = $env:NODECLOAK_TERMINAL_ENVIRONMENT | ConvertFrom-Json\r\nRemove-Item Env:NODECLOAK_TERMINAL_ENVIRONMENT -ErrorAction SilentlyContinue\r\nforeach ($property in $environment.PSObject.Properties) { [Environment]::SetEnvironmentVariable($property.Name, [string]$property.Value, 'Process') }\r\nSet-Location -LiteralPath")
    } else {
        legacy.replace("export TZ=Asia/Singapore\nexport LANG=en_US.UTF-8\nexport LC_ALL=en_US.UTF-8\n", "typeset -r NODECLOAK_TERMINAL_DIRECTORY=\"${1:-$HOME}\"\ntypeset -r NODECLOAK_TERMINAL_ENVFILE=\"${2:-}\"\nif [[ -n \"$NODECLOAK_TERMINAL_ENVFILE\" ]]; then\n  builtin source -- \"$NODECLOAK_TERMINAL_ENVFILE\" || { /bin/rm -f -- \"$NODECLOAK_TERMINAL_ENVFILE\"; exit 1; }\n  /bin/rm -f -- \"$NODECLOAK_TERMINAL_ENVFILE\"\nfi\n")
            .replace("cd -- \"${1:-$HOME}\"", "builtin cd -- \"$NODECLOAK_TERMINAL_DIRECTORY\"")
    }
}

fn working_directory(input: &str) -> Result<PathBuf, String> {
    let path = if input.trim().is_empty() {
        platform::home_dir()
    } else {
        PathBuf::from(input.trim())
    };
    if !path.is_absolute() || !path.is_dir() {
        return Err(
            "请选择已存在的绝对工作目录，例如 D:\\Projects、/Users/name/Projects 或 /home/name/Projects。".into(),
        );
    }
    Ok(path)
}

#[cfg(windows)]
fn windows_command(contents: &str) -> String {
    // Run our fixed commands inline: local .ps1 files may be blocked by the default policy.
    // The directory is passed as a child-process environment value, never interpolated as code.
    format!("& {{\r\n{contents}\r\n}} -WorkingDirectory $env:NODECLOAK_TERMINAL_DIRECTORY")
}

#[cfg(any(target_os = "macos", test))]
fn mac_command(path: &Path, directory: &Path, environment: Option<&Path>) -> String {
    let quote = |value: &Path| format!("'{}'", value.to_string_lossy().replace('\'', "'\\''"));
    let shell = format!(
        "{} {}{}",
        quote(path),
        quote(directory),
        environment
            .map(|p| format!(" {}", quote(p)))
            .unwrap_or_default()
    );
    format!(
        "tell application \"Terminal\" to do script \"{}\"",
        shell.replace('\\', "\\\\").replace('"', "\\\"")
    )
}

pub fn launch(
    root: &Path,
    tool: Tool,
    directory: &str,
    environment: environment::Environment,
) -> Result<(), String> {
    let overrides = environment.overrides()?;
    let directory = working_directory(directory)?;
    if tool != Tool::Shell
        && !overrides.keys().any(|key| key.eq_ignore_ascii_case("PATH"))
        && find_in(tool, &search_roots(), cfg!(windows)).is_none()
    {
        return Err(format!("未找到 {}，请先安装后点击重新检测。", tool.name()));
    }
    let folder = root.join("terminal-launchers");
    crate::profiles::guard_tree(&folder)?;
    fs::create_dir_all(&folder).map_err(|e| e.to_string())?;
    let path = folder.join(format!(
        "{}.{}",
        tool.slug(),
        if cfg!(windows) { "ps1" } else if cfg!(target_os = "linux") { "sh" } else { "command" }
    ));
    #[cfg(not(target_os = "linux"))]
    let contents = launcher_contents(tool, cfg!(windows));
    #[cfg(target_os = "linux")]
    let contents = linux::launcher_contents(tool);
    crate::profiles::guard_tree(&path)?;
    if path.exists() {
        let previous = fs::read_to_string(&path).map_err(|e| e.to_string())?;
        if previous != contents && previous != legacy_launcher_contents(tool, cfg!(windows)) {
            return Err("这个终端启动器已被修改，未覆盖它。请移走自定义文件后重试。".into());
        }
    }
    engine::atomic_write(&path, contents.as_bytes())?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o700)).map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // The user explicitly opens an interactive terminal from the terminal workspace.
        Command::new("powershell.exe")
            .args(["-NoExit", "-NoProfile", "-Command"])
            .arg(windows_command(&contents))
            .env("NODECLOAK_TERMINAL_DIRECTORY", &directory)
            .env(
                "NODECLOAK_TERMINAL_ENVIRONMENT",
                serde_json::to_string(&overrides).map_err(|_| "终端环境格式无效")?,
            )
            .current_dir(&directory)
            .creation_flags(0x00000010)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        // Terminal.app may already be running, so osascript process env is not
        // inherited. Use an owner-only, per-launch export file and remove it
        // immediately after the shell reads it. Values are single-quoted data.
        use std::io::Write;
        use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
        let sessions = folder.join("sessions");
        crate::profiles::guard_tree(&sessions)?;
        fs::create_dir_all(&sessions).map_err(|_| "无法创建终端会话目录")?;
        fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700))
            .map_err(|_| "无法保护终端会话目录")?;
        let payload = sessions.join(format!("{}.zsh", uuid::Uuid::new_v4()));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o600)
            .open(&payload)
            .map_err(|_| "无法创建终端环境会话")?;
        file.write_all(environment::mac_exports(&overrides).as_bytes())
            .map_err(|_| {
                let _ = fs::remove_file(&payload);
                "无法写入终端环境会话"
            })?;
        drop(file);
        let cleanup = payload.clone();
        std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(60));
            let _ = fs::remove_file(cleanup);
        });
        let result = Command::new("/usr/bin/osascript")
            .args([
                "-e",
                &mac_command(&path, &directory, Some(&payload)),
                "-e",
                "tell application \"Terminal\" to activate",
            ])
            .output()
            .map_err(|e| e.to_string())?;
        if !result.status.success() {
            let _ = fs::remove_file(&payload);
            return Err("无法打开 Terminal，请检查 macOS 自动化权限。".into());
        }
    }
    #[cfg(target_os = "linux")]
    linux::launch(&folder, &path, &directory, &overrides)?;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        return Err("目前终端启动支持 Windows、macOS 和 Ubuntu。".into());
    }
    #[allow(unreachable_code)]
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn launcher_uses_environment_payload_instead_of_fixed_region() {
        let windows = launcher_contents(Tool::Shell, true);
        assert!(windows.contains("NODECLOAK_TERMINAL_ENVIRONMENT"));
        assert!(!windows.contains("$env:TZ = 'Asia/Singapore'"));
        let mac = launcher_contents(Tool::Shell, false);
        assert!(!mac.contains("export TZ=Asia/Singapore"));
        assert!(mac.contains("source"));
    }

    #[cfg(windows)]
    #[test]
    fn powershell_launcher_runs_npm_cli_in_literal_project_directory() {
        let root = tempfile::tempdir().unwrap();
        let bin = root.path().join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let project = root.path().join("Project's $(Write-Error injected)");
        fs::create_dir(&project).unwrap();
        let fixture="@echo off\r\necho TZ=%TZ%\r\necho LANG=%LANG%\r\necho LC_ALL=%LC_ALL%\r\necho PROXY=%HTTPS_PROXY%\r\necho KEY=%OPENAI_API_KEY%\r\necho DIR=\"%CD%\"\r\n";
        fs::write(bin.join("codex.cmd"), fixture).unwrap();
        let result = platform::command("powershell.exe")
            .args(["-NoProfile", "-NonInteractive", "-Command"])
            .arg(windows_command(&launcher_contents(Tool::Codex, true)))
            .env("NODECLOAK_TERMINAL_DIRECTORY", &project)
            .env("USERPROFILE", root.path())
            .env("APPDATA", root.path())
            .env("HTTPS_PROXY", "http://fixture.invalid:8080")
            .env("OPENAI_API_KEY", "fixture-value")
            .env(
                "NODECLOAK_TERMINAL_ENVIRONMENT",
                serde_json::to_string(&environment::Environment::default().overrides().unwrap())
                    .unwrap(),
            )
            .output()
            .unwrap();
        assert!(
            result.status.success(),
            "{}",
            String::from_utf8_lossy(&result.stderr)
        );
        let text = String::from_utf8_lossy(&result.stdout);
        for expected in [
            "TZ=Asia/Singapore",
            "LANG=en_US.UTF-8",
            "LC_ALL=en_US.UTF-8",
            "PROXY=http://fixture.invalid:8080",
            "KEY=fixture-value",
        ] {
            assert!(
                text.contains(expected),
                "Missing {expected} in fixture output"
            );
        }
        let actual_directory = text
            .lines()
            .find_map(|line| {
                line.strip_prefix("DIR=\"")
                    .and_then(|value| value.strip_suffix('"'))
            })
            .expect("The CLI fixture must report its working directory");
        // cmd.exe may expand an 8.3 temporary root (RUNNER~1) to its long spelling.
        assert_eq!(
            fs::canonicalize(actual_directory).unwrap(),
            fs::canonicalize(&project).unwrap()
        );
        assert!(!text.contains("Write-Error :"));
    }
    #[cfg(windows)]
    #[test]
    fn powershell_custom_environment_is_literal_and_inherit_does_not_override() {
        let root = tempfile::tempdir().unwrap();
        let literal = "quote' \" double; $(throw 'injected') `backtick\n第二行";
        for inherit in [false, true] {
            let mut config = environment::Environment::default();
            config.mode = if inherit {
                environment::Mode::Inherit
            } else {
                environment::Mode::Custom
            };
            config.timezone = "Europe/Paris".into();
            config.lang = "zh_CN.UTF-8".into();
            config.lc_all = String::new();
            config.variables = vec![
                environment::Variable {
                    name: "CUSTOM_LITERAL".into(),
                    value: literal.into(),
                },
                environment::Variable {
                    name: "HTTPS_PROXY".into(),
                    value: "http://custom.invalid:9999".into(),
                },
            ];
            let script=format!("{}\r\n@{{tz=$env:TZ;lang=$env:LANG;lc=$env:LC_ALL;literal=$env:CUSTOM_LITERAL;proxy=$env:HTTPS_PROXY;key=$env:OPENAI_API_KEY;payload=[Environment]::GetEnvironmentVariable('NODECLOAK_TERMINAL_ENVIRONMENT')}} | ConvertTo-Json -Compress | ForEach-Object {{ [Convert]::ToBase64String([Text.Encoding]::UTF8.GetBytes($_)) }}",launcher_contents(Tool::Shell,true));
            let result = platform::command("powershell.exe")
                .args(["-NoProfile", "-NonInteractive", "-Command"])
                .arg(windows_command(&script))
                .env("NODECLOAK_TERMINAL_DIRECTORY", root.path())
                .env(
                    "NODECLOAK_TERMINAL_ENVIRONMENT",
                    serde_json::to_string(&config.overrides().unwrap()).unwrap(),
                )
                .env("TZ", "Pacific/Auckland")
                .env("LANG", "C")
                .env("LC_ALL", "C")
                .env("CUSTOM_LITERAL", "inherited")
                .env("HTTPS_PROXY", "http://inherited.invalid:8888")
                .env("OPENAI_API_KEY", "synthetic-inherited-key")
                .output()
                .unwrap();
            assert!(result.status.success(), "terminal fixture failed");
            let text = String::from_utf8_lossy(&result.stdout);
            use base64::Engine;
            let line = text.lines().last().unwrap();
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(line)
                .unwrap();
            let value: serde_json::Value = serde_json::from_slice(&bytes).unwrap();
            assert_eq!(
                value["tz"],
                if inherit {
                    "Pacific/Auckland"
                } else {
                    "Europe/Paris"
                }
            );
            assert_eq!(value["lang"], if inherit { "C" } else { "zh_CN.UTF-8" });
            assert_eq!(value["lc"], "C");
            assert_eq!(
                value["literal"],
                if inherit { "inherited" } else { literal }
            );
            assert_eq!(
                value["proxy"],
                if inherit {
                    "http://inherited.invalid:8888"
                } else {
                    "http://custom.invalid:9999"
                }
            );
            assert_eq!(value["key"], "synthetic-inherited-key");
            assert!(value["payload"].is_null());
        }
    }
    #[test]
    fn only_known_tools_are_accepted_and_npm_shims_are_found() {
        assert!(serde_json::from_str::<Tool>("\"codex; touch injected\"").is_err());
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("codex.cmd"), "fixture").unwrap();
        fs::write(dir.path().join("gemini.ps1"), "fixture").unwrap();
        assert!(find_in(Tool::Codex, &[dir.path().to_owned()], true).is_some());
        assert!(find_in(Tool::Claude, &[dir.path().to_owned()], true).is_none());
        // Do not declare a policy-restricted .ps1-only installation launchable.
        assert!(find_in(Tool::Gemini, &[dir.path().to_owned()], true).is_none());
    }
    #[test]
    fn launchers_preserve_credentials_and_literal_working_directories() {
        for tool in [Tool::Claude, Tool::Codex, Tool::Gemini, Tool::Shell] {
            for windows in [true, false] {
                let script = launcher_contents(tool, windows);
                assert!(!script.contains("Asia/Singapore") && !script.contains("en_US.UTF-8"));
                assert!(
                    !script.contains("API_KEY")
                        && !script.contains("PROXY")
                        && !script.contains("ExecutionPolicy")
                );
                if let Some(command) = tool.command() {
                    assert!(script.contains(command));
                }
                assert!(script.contains(if windows {
                    "-LiteralPath $WorkingDirectory"
                } else {
                    "builtin cd -- \"$NODECLOAK_TERMINAL_DIRECTORY\""
                }));
            }
        }
        let dir = tempfile::tempdir().unwrap();
        assert_eq!(
            working_directory(dir.path().to_str().unwrap()).unwrap(),
            dir.path()
        );
        assert!(working_directory("relative/project").is_err());
        assert!(working_directory(dir.path().join("missing").to_str().unwrap()).is_err());
    }
    #[cfg(windows)]
    #[test]
    fn posix_export_values_are_literal_when_loaded_by_a_shell() {
        let bash = Path::new(r"C:\Program Files\Git\bin\bash.exe");
        if !bash.exists() {
            return;
        }
        let root = tempfile::tempdir().unwrap();
        let file = root.path().join("environment.zsh");
        let literal = "quote' \" double; $(touch injected) `touch injected`\n第二行";
        let values = std::collections::BTreeMap::from([("CUSTOM_LITERAL".into(), literal.into())]);
        fs::write(&file, environment::mac_exports(&values)).unwrap();
        let path = file.to_string_lossy().replace('\\', "/");
        let result = platform::command(bash.to_str().unwrap())
            .args([
                "--noprofile",
                "--norc",
                "-c",
                "source \"$1\"; printf '%s' \"$CUSTOM_LITERAL\"",
                "fixture",
                &path,
            ])
            .current_dir(root.path())
            .output()
            .unwrap();
        assert!(result.status.success());
        assert_eq!(String::from_utf8_lossy(&result.stdout), literal);
        assert!(!root.path().join("injected").exists());
    }
    #[test]
    fn mac_terminal_quotes_paths_for_both_shell_and_applescript() {
        let line = mac_command(
            Path::new("/Users/a'b/launcher.command"),
            Path::new("/Users/a/Project \"$()\""),
            None,
        );
        assert!(line.contains("a'\\\\''b"));
        assert!(line.contains("\\\"$()\\\""));
        assert!(line.starts_with("tell application \"Terminal\" to do script \"'"));
    }
}
