//! Exact bundle membership; never match all processes merely by their basename.
use std::path::{Component, Path};
pub fn valid_selection(path: &str) -> bool {
    path.starts_with('/')
        && path.len() <= 2048
        && !path.chars().any(char::is_control)
        && !path.contains(['\\', '\0'])
        && !path.split('/').any(|p| p == "." || p == "..")
        && (valid_executable_path(path)
            || path.ends_with(".app")
                && !path
                    .rsplit('/')
                    .next()
                    .is_some_and(|p| p.eq_ignore_ascii_case("NodeCloak.app")))
}
pub fn valid_executable_path(path: &str) -> bool {
    let Some((bundle, tail)) = path.split_once(".app/Contents/") else {
        return false;
    };
    path.starts_with('/')
        && path.len() <= 2048
        && !path.chars().any(char::is_control)
        && !path.contains('\\')
        && !path.split('/').any(|p| p == "." || p == "..")
        && !bundle
            .rsplit('/')
            .next()
            .is_some_and(|p| p.eq_ignore_ascii_case("NodeCloak"))
        && !tail.is_empty()
        && !path.ends_with('/')
        && ![
            "nodecloak",
            "sing-box",
            "sh",
            "bash",
            "zsh",
            "python",
            "python3",
            "node",
        ]
        .contains(
            &path
                .rsplit('/')
                .next()
                .unwrap_or("")
                .to_ascii_lowercase()
                .as_str(),
        )
}
pub fn identify(path: &str) -> Option<&'static str> {
    if !valid_executable_path(path) {
        return None;
    }
    let before = path.split_once(".app/Contents/")?.0;
    match before.rsplit('/').next()? {
        "Codex" => Some("codex"),
        "ChatGPT" => Some("chatgpt"),
        "Claude" => Some("claude"),
        _ => None,
    }
}
fn macho(path: &Path) -> bool {
    use std::io::Read;
    let mut bytes = [0; 4];
    std::fs::File::open(path)
        .and_then(|mut f| f.read_exact(&mut bytes))
        .is_ok()
        && matches!(
            bytes,
            [0xcf, 0xfa, 0xed, 0xfe]
                | [0xfe, 0xed, 0xfa, 0xcf]
                | [0xca, 0xfe, 0xba, 0xbe]
                | [0xbe, 0xba, 0xfe, 0xca]
                | [0xca, 0xfe, 0xba, 0xbf]
                | [0xbf, 0xba, 0xfe, 0xca]
        )
}
pub fn executables(selection: &Path) -> Result<Vec<String>, String> {
    let canonical = selection
        .canonicalize()
        .map_err(|_| "无法读取所选 Mac 应用")?;
    if selection
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("应用路径不能包含上级目录".into());
    }
    let bundle = if canonical.is_dir() && canonical.extension().is_some_and(|s| s == "app") {
        canonical.clone()
    } else {
        canonical
            .ancestors()
            .find(|p| p.extension().is_some_and(|s| s == "app"))
            .ok_or("请选择 Mac 应用（.app）或应用内的程序")?
            .to_owned()
    };
    if bundle
        .file_name()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("NodeCloak.app"))
    {
        return Err("不能选择 NodeCloak 本身".into());
    }
    let mut pending = vec![(bundle.join("Contents"), 0usize)];
    let mut result = vec![];
    let mut visited = 0;
    while let Some((dir, depth)) = pending.pop() {
        if depth > 16 {
            return Err("应用目录层级超过限制".into());
        }
        for entry in std::fs::read_dir(&dir).map_err(|_| "无法读取应用内容")? {
            let entry = entry.map_err(|_| "无法读取应用内容")?;
            visited += 1;
            if visited > 20_000 {
                return Err("应用包含的文件过多，请选择实际程序文件".into());
            }
            let kind = entry.file_type().map_err(|_| "无法检查应用文件")?;
            if kind.is_symlink() {
                continue;
            }
            let path = entry.path();
            if kind.is_dir() {
                pending.push((path, depth + 1));
                continue;
            }
            if !kind.is_file() || !macho(&path) {
                continue;
            }
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                if entry
                    .metadata()
                    .map_err(|_| "无法检查应用权限")?
                    .permissions()
                    .mode()
                    & 0o111
                    == 0
                {
                    continue;
                }
            }
            let path = path.canonicalize().map_err(|_| "无法解析应用程序路径")?;
            if !path.starts_with(&bundle) {
                return Err("应用程序路径超出了所选应用".into());
            }
            let value = path.to_str().ok_or("应用程序路径不是有效文本")?;
            #[cfg(target_os = "macos")]
            if !valid_executable_path(value) {
                continue;
            }
            result.push(value.to_owned());
            if result.len() > 32 {
                return Err("应用包含的可执行程序超过 32 个，请选择实际程序文件".into());
            }
        }
    }
    if !canonical.is_dir() {
        result.retain(|p| Path::new(p) == canonical)
    }
    result.sort();
    result.dedup();
    if result.is_empty() {
        return Err("没有找到可代理的 Mac 可执行程序".into());
    }
    Ok(result)
}
#[cfg(target_os = "macos")]
pub fn discover(apps: &mut [super::discovery::DetectedApp]) -> Result<(), String> {
    use sysinfo::{ProcessRefreshKind, ProcessesToUpdate, System, UpdateKind};
    let mut system = System::new();
    system.refresh_processes_specifics(
        ProcessesToUpdate::All,
        true,
        ProcessRefreshKind::nothing().with_exe(UpdateKind::Always),
    );
    for app in apps {
        let mut bundles = vec![std::path::PathBuf::from(format!(
            "/Applications/{}.app",
            app.name
        ))];
        if let Some(home) = std::env::var_os("HOME") {
            bundles.push(
                std::path::PathBuf::from(home)
                    .join("Applications")
                    .join(format!("{}.app", app.name)),
            );
        }
        for process in system.processes().values() {
            if let Some(path) = process
                .exe()
                .and_then(|p| p.to_str())
                .filter(|p| identify(p) == Some(app.id.as_str()))
            {
                if let Some(bundle) = Path::new(path)
                    .ancestors()
                    .filter(|p| p.extension().is_some_and(|s| s == "app"))
                    .last()
                {
                    bundles.push(bundle.to_owned());
                }
            }
        }
        bundles.sort();
        bundles.dedup();
        for bundle in bundles {
            if bundle.exists() {
                app.paths.extend(executables(&bundle)?);
            }
        }
        app.paths.sort();
        app.paths.dedup();
        app.running = system
            .processes()
            .values()
            .filter(|p| {
                p.exe()
                    .and_then(|p| p.to_str())
                    .is_some_and(|p| app.paths.iter().any(|known| known == p))
            })
            .count() as u32;
    }
    Ok(())
}
