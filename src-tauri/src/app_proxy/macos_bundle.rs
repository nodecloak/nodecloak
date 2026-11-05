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
    #[cfg(target_os = "macos")]
    return bundle_product(Path::new(&format!("{before}.app")));
    #[cfg(not(target_os = "macos"))]
    {
        let _ = before;
        None
    }
}
fn publisher(identifier: &str) -> Option<&'static str> {
    match identifier {
        "com.openai.codex" | "com.openai.chat" => Some("2DC432GLL2"),
        "com.anthropic.claudefordesktop" => Some("Q6L2SF6YDW"),
        _ => None,
    }
}
fn product(_bundle: &str, identifier: &str, team: &str) -> Option<&'static str> {
    if publisher(identifier) != Some(team) {
        return None;
    }
    match identifier {
        "com.openai.codex" => Some("codex"),
        "com.openai.chat" => Some("chatgpt"),
        "com.anthropic.claudefordesktop" => Some("claude"),
        _ => None,
    }
}
#[cfg(target_os = "macos")]
pub(super) fn bundle_product(bundle: &Path) -> Option<&'static str> {
    use std::process::{Command, Stdio};
    let output = Command::new("/usr/libexec/PlistBuddy")
        .args(["-c", "Print :CFBundleIdentifier"])
        .arg(bundle.join("Contents/Info.plist"))
        .stderr(Stdio::null())
        .output()
        .ok()?;
    if !output.status.success() || output.stdout.len() > 1024 {
        return None;
    }
    let id = std::str::from_utf8(&output.stdout).ok()?.trim();
    let team = publisher(id)?;
    let product = product(bundle.file_name()?.to_str()?, id, team)?;
    let requirement = format!("=anchor apple generic and identifier \"{id}\" and certificate leaf[subject.OU] = \"{team}\"");
    Command::new("/usr/bin/codesign")
        .args(["--verify", "--strict", "-R", &requirement])
        .arg(bundle)
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .ok()?
        .success()
        .then_some(product)
}
fn macho(path: &Path) -> bool {
    use std::io::{Read, Seek, SeekFrom};
    fn thin(file: &mut std::fs::File, offset: u64) -> bool {
        let mut h = [0; 16];
        if file
            .seek(SeekFrom::Start(offset))
            .and_then(|_| file.read_exact(&mut h))
            .is_err()
        {
            return false;
        }
        match h[..4] {
            [0xcf, 0xfa, 0xed, 0xfe] => u32::from_le_bytes(h[12..16].try_into().unwrap()) == 2,
            [0xfe, 0xed, 0xfa, 0xcf] => u32::from_be_bytes(h[12..16].try_into().unwrap()) == 2,
            _ => false,
        }
    }
    let Ok(mut file) = std::fs::File::open(path) else {
        return false;
    };
    let mut header = [0; 8];
    if file.read_exact(&mut header).is_err() {
        return false;
    }
    let fat64 = header[..4] == [0xca, 0xfe, 0xba, 0xbf];
    if !fat64 && header[..4] != [0xca, 0xfe, 0xba, 0xbe] {
        return thin(&mut file, 0);
    }
    let count = u32::from_be_bytes(header[4..8].try_into().unwrap());
    if !(1..=16).contains(&count) {
        return false;
    }
    let size = if fat64 { 32 } else { 20 };
    let length = file.metadata().map(|m| m.len()).unwrap_or(0);
    for i in 0..count {
        let mut arch = [0; 32];
        if file
            .seek(SeekFrom::Start(8 + (i * size) as u64))
            .and_then(|_| file.read_exact(&mut arch[..size as usize]))
            .is_err()
        {
            return false;
        }
        let offset = if fat64 {
            u64::from_be_bytes(arch[8..16].try_into().unwrap())
        } else {
            u32::from_be_bytes(arch[8..12].try_into().unwrap()) as u64
        };
        if offset.checked_add(16).is_some_and(|end| end <= length) && thin(&mut file, offset) {
            return true;
        }
    }
    false
}
pub fn executables(selection: &Path) -> Result<Vec<String>, String> {
    if std::fs::symlink_metadata(selection).is_ok_and(|m| m.file_type().is_symlink()) {
        return Err("请选择实际的 Mac 应用或程序，不支持符号链接入口".into());
    }
    let lexical_bundle = selection
        .ancestors()
        .find(|p| p.extension().is_some_and(|s| s == "app"))
        .ok_or("请选择 Mac 应用（.app）或应用内的程序")?;
    let selected_bundle = lexical_bundle
        .canonicalize()
        .map_err(|_| "无法解析所选 Mac 应用")?;
    let canonical = selection
        .canonicalize()
        .map_err(|_| "无法读取所选 Mac 应用")?;
    if selection
        .components()
        .any(|c| matches!(c, Component::ParentDir))
    {
        return Err("应用路径不能包含上级目录".into());
    }
    let bundle = selected_bundle;
    if !canonical.starts_with(&bundle) {
        return Err("应用程序路径超出了所选应用".into());
    }
    if bundle
        .file_name()
        .is_some_and(|s| s.to_string_lossy().eq_ignore_ascii_case("NodeCloak.app"))
    {
        return Err("不能选择 NodeCloak 本身".into());
    }
    if canonical.is_file() {
        if !macho(&canonical) {
            return Err("请选择 Mac 可执行程序，不能选择脚本或共享库".into());
        }
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            if canonical
                .metadata()
                .map_err(|_| "无法读取程序权限")?
                .permissions()
                .mode()
                & 0o111
                == 0
            {
                return Err("所选程序没有执行权限".into());
            }
        }
        return Ok(vec![canonical
            .to_str()
            .ok_or("应用路径不是有效文本")?
            .to_owned()]);
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
        // Product identity comes from the bundle, not its display name. Some
        // Codex builds use ChatGPT.app as their on-disk name.
        let mut bundles:Vec<_> = ["Codex","ChatGPT","Claude"].iter().map(|name|std::path::PathBuf::from(format!("/Applications/{name}.app"))).collect();
        if let Some(home) = std::env::var_os("HOME") {
            for name in ["Codex","ChatGPT","Claude"] {
                bundles.push(std::path::PathBuf::from(&home).join("Applications").join(format!("{name}.app")));
            }
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
            if bundle.exists() && bundle_product(&bundle) == Some(app.id.as_str()) {
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

#[cfg(test)]
mod checks {
    use super::*;
    #[test]
    fn renamed_apps_do_not_become_codex_or_chatgpt() {
        assert_eq!(product("Codex.app", "com.other.vendor", "2DC432GLL2"), None);
        assert_eq!(product("Codex.app", "com.openai.codex", "FOREIGN123"), None);
        assert_eq!(
            product("Codex.app", "com.openai.codex", "2DC432GLL2"),
            Some("codex")
        );
        assert_eq!(
            product("ChatGPT.app", "com.openai.chat", "2DC432GLL2"),
            Some("chatgpt")
        );
    }
    fn image(file: &Path, kind: u32) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let mut header = vec![0u8; 32];
        header[..4].copy_from_slice(&[0xcf, 0xfa, 0xed, 0xfe]);
        header[12..16].copy_from_slice(&kind.to_le_bytes());
        std::fs::write(file, header).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            std::fs::set_permissions(file, std::fs::Permissions::from_mode(0o755)).unwrap();
        }
    }
    #[test]
    fn bundles_include_nested_helpers_but_not_executable_libraries() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let app = dir.path().join("Example.app");
        let main = app.join("Contents/MacOS/Example");
        let helper =
            app.join("Contents/Frameworks/Example Helper.app/Contents/MacOS/Example Helper");
        image(&main, 2);
        image(&helper, 2);
        image(&app.join("Contents/Frameworks/Library.dylib"), 6);
        let found = executables(&app).unwrap();
        assert_eq!(
            found.len(),
            2,
            "a shared library cannot own process traffic"
        );
        assert!(found.contains(&main.canonicalize().unwrap().to_string_lossy().into_owned()));
        assert!(found.contains(
            &helper
                .canonicalize()
                .unwrap()
                .to_string_lossy()
                .into_owned()
        ));
    }
    #[cfg(unix)]
    #[test]
    fn selecting_a_symlink_cannot_switch_to_a_foreign_bundle() {
        let dir = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let app = dir.path().join("Example.app");
        let other = dir.path().join("Foreign.app/Contents/MacOS/Foreign");
        image(&app.join("Contents/MacOS/Example"), 2);
        image(&other, 2);
        let link = app.join("Contents/MacOS/Escape");
        std::os::unix::fs::symlink(&other, &link).unwrap();
        assert!(executables(&link).is_err());
        assert_eq!(executables(&app).unwrap().len(), 1);
    }
}
