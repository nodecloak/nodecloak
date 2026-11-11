use super::{environment, executable_file, Tool};
use std::{collections::BTreeMap, ffi::OsString, fs, path::{Path, PathBuf}};

pub(super) fn launcher_contents(tool: Tool) -> String {
    let start = tool.command().map(|name| format!(
        "if ! command -v {name} >/dev/null 2>&1; then echo '{} is not installed or not on PATH.'; exit 1; fi\nexec {name}\n", tool.name()
    )).unwrap_or_else(|| "exec /bin/bash -i\n".into());
    format!("#!/bin/bash\n# NodeCloak managed terminal launcher\nexport PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:$HOME/.bun/bin:/usr/local/bin:/usr/bin:/bin:$PATH\"\nreadonly NODECLOAK_TERMINAL_DIRECTORY=\"${{1:-$HOME}}\"\nreadonly NODECLOAK_TERMINAL_ENVFILE=\"${{2:-}}\"\nif [[ -n \"$NODECLOAK_TERMINAL_ENVFILE\" ]]; then\n  builtin source -- \"$NODECLOAK_TERMINAL_ENVFILE\" || {{ /bin/rm -f -- \"$NODECLOAK_TERMINAL_ENVFILE\"; exit 1; }}\n  /bin/rm -f -- \"$NODECLOAK_TERMINAL_ENVFILE\"\nfi\nbuiltin cd -- \"$NODECLOAK_TERMINAL_DIRECTORY\" || exit 1\n{start}")
}

fn terminal_in(roots: &[PathBuf]) -> Option<(PathBuf, &'static str)> {
    // Ubuntu's default terminal accepts literal argv after `--`. The Debian
    // alternative and xterm accept argv after `-e`; no command-string parsing.
    [ ("gnome-terminal", "--"), ("kgx", "--"), ("x-terminal-emulator", "-e"), ("konsole", "-e"), ("xterm", "-e") ]
        .into_iter().find_map(|(name, separator)| roots.iter().map(|root| root.join(name))
            .find(|file| executable_file(file)).map(|file| (file, separator)))
}

#[cfg(target_os = "linux")]
pub(crate) fn open(path: &Path, directory: &Path, payload: Option<&Path>) -> Result<(), String> {
    let mut roots: Vec<_> = std::env::var_os("PATH").map(|p| std::env::split_paths(&p).collect()).unwrap_or_default();
    roots.extend([PathBuf::from("/usr/bin"), PathBuf::from("/bin")]);
    let (terminal, separator) = terminal_in(&roots).ok_or("未找到可用终端，请安装 GNOME Terminal 后重试。")?;
    crate::platform::command(terminal.to_str().ok_or("终端路径无效")?)
        .args(arguments(separator, path, directory, payload))
        .current_dir(directory)
        .spawn().map_err(|_| "无法打开终端，请检查桌面会话和终端安装。")?;
    Ok(())
}

fn arguments(separator: &str, path: &Path, directory: &Path, payload: Option<&Path>) -> Vec<OsString> {
    let mut args: Vec<OsString> = [separator, "/bin/bash", "--noprofile", "--norc"].into_iter().map(Into::into).collect();
    args.extend([path.as_os_str().into(), directory.as_os_str().into()]);
    if let Some(payload) = payload { args.push(payload.as_os_str().into()); }
    args
}

#[cfg(target_os = "linux")]
pub(super) fn launch(folder: &Path, path: &Path, directory: &Path, overrides: &BTreeMap<String, String>) -> Result<(), String> {
    use std::{io::Write, os::unix::fs::{OpenOptionsExt, PermissionsExt}};
    // A running terminal server does not necessarily inherit the caller's
    // environment. Pass overrides in a private file, never terminal argv.
    let sessions = folder.join("sessions");
    crate::profiles::guard_tree(&sessions)?;
    fs::create_dir_all(&sessions).map_err(|_| "无法创建终端会话目录")?;
    fs::set_permissions(&sessions, fs::Permissions::from_mode(0o700)).map_err(|_| "无法保护终端会话目录")?;
    let payload = sessions.join(format!("{}.sh", uuid::Uuid::new_v4()));
    let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&payload).map_err(|_| "无法创建终端环境会话")?;
    if file.write_all(environment::mac_exports(overrides).as_bytes()).is_err() {
        let _ = fs::remove_file(&payload);
        return Err("无法写入终端环境会话".into());
    }
    drop(file);
    if let Err(error) = open(path, directory, Some(&payload)) {
        let _ = fs::remove_file(&payload);
        return Err(error);
    }
    std::thread::spawn(move || {
        std::thread::sleep(std::time::Duration::from_secs(60));
        let _ = fs::remove_file(payload);
    });
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn terminal_arguments_keep_paths_literal_and_exclude_secret_values() {
        let script = Path::new("/tmp/Node Cloak's $(touch injected)/codex.sh");
        let directory = Path::new("/home/test/Project's `touch injected`");
        let payload = Path::new("/tmp/private-session.sh");
        let args = arguments("--", script, directory, Some(payload));
        assert_eq!(args[4], script.as_os_str());
        assert_eq!(args[5], directory.as_os_str());
        assert_eq!(args[6], payload.as_os_str());
        assert_eq!(args.len(), 7);
        let script = launcher_contents(Tool::Codex);
        assert!(script.starts_with("#!/bin/bash\n"));
        assert!(!script.contains("zsh") && !script.contains("Asia/Singapore"));
    }
    #[cfg(target_os = "linux")]
    #[test]
    fn bash_launcher_preserves_literals_and_removes_private_payload() {
        use std::os::unix::fs::PermissionsExt;
        let temp = tempfile::tempdir().unwrap();
        let root = temp.path();
        let project = root.join("Project's $(touch injected)");
        fs::create_dir(&project).unwrap();
        let bin = root.join(".local/bin");
        fs::create_dir_all(&bin).unwrap();
        let cli = bin.join("codex");
        fs::write(&cli, "#!/bin/bash\nprintf '%s\\n' \"$PWD\" \"$CUSTOM_LITERAL\" \"$TZ\"\n").unwrap();
        fs::set_permissions(&cli, fs::Permissions::from_mode(0o700)).unwrap();
        let script = root.join("codex.sh");
        fs::write(&script, launcher_contents(Tool::Codex)).unwrap();
        let payload = root.join("private.sh");
        let literal = "quote' \"; $(touch injected) `touch injected`\n第二行";
        fs::write(&payload, environment::mac_exports(&BTreeMap::from([("CUSTOM_LITERAL".into(), literal.into()), ("TZ".into(), "Europe/Paris".into())]))).unwrap();
        let output = std::process::Command::new("/bin/bash").args(["--noprofile", "--norc"]).arg(&script).arg(&project).arg(&payload).env("HOME", root).output().unwrap();
        assert!(output.status.success(), "{}", String::from_utf8_lossy(&output.stderr));
        assert_eq!(String::from_utf8_lossy(&output.stdout), format!("{}\n{literal}\nEurope/Paris\n", project.display()));
        assert!(!payload.exists());
        assert!(!project.join("injected").exists());
    }
    #[cfg(unix)]
    #[test]
    fn terminal_detection_rejects_non_executable_files() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let terminal = dir.path().join("gnome-terminal");
        fs::write(&terminal, "fixture").unwrap();
        fs::set_permissions(&terminal, fs::Permissions::from_mode(0o600)).unwrap();
        assert!(terminal_in(&[dir.path().into()]).is_none());
        fs::set_permissions(&terminal, fs::Permissions::from_mode(0o700)).unwrap();
        assert_eq!(terminal_in(&[dir.path().into()]).unwrap().0, terminal);
    }
}
