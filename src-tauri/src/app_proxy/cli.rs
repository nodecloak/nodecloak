//! Discover native CLI executables, never a shared Node/Python/shell runtime.
pub(super) fn identify_windows(path: &str) -> Option<&'static str> {
    let path = super::discovery::normalize(path);
    for channel in [r"\.vscode\extensions\", r"\.vscode-insiders\extensions\"] {
        if let Some((_,tail))=path.rsplit_once(channel) {
            if let Some((package,suffix))=tail.split_once('\\') {
                if suffix==r"resources\native-binary\claude.exe" && vscode_package(package) {return Some("claude");}
            }
        }
    }
    for (name, id) in [("claude.exe", "claude"), ("codex.exe", "codex")] {
        if path.ends_with(&format!(r"\.local\bin\{name}")) {
            return Some(id);
        }
    }
    let (_, tail) = path.rsplit_once(r"\node_modules\@openai\")?;
    for (package, target) in [
        ("codex", "x86_64-pc-windows-msvc"),
        ("codex", "aarch64-pc-windows-msvc"),
        ("codex-win32-x64", "x86_64-pc-windows-msvc"),
        ("codex-win32-arm64", "aarch64-pc-windows-msvc"),
    ] {
        for bin in ["bin", "codex"] {
            if tail == format!(r"{package}\vendor\{target}\{bin}\codex.exe") {
                return Some("codex");
            }
        }
    }
    None
}

fn vscode_package(package:&str)->bool {
    let Some(version)=package.strip_prefix("anthropic.claude-code-").and_then(|s|s.strip_suffix("-win32-x64").or_else(||s.strip_suffix("-win32-arm64"))) else{return false;};
    version.split('.').count()==3 && version.split('.').all(|part|!part.is_empty()&&part.bytes().all(|b|b.is_ascii_digit()))
}

#[cfg(windows)]
pub(super) fn discover_in(
    apps: &mut [super::discovery::DetectedApp],
    home: &std::path::Path,
    shim_roots: &[std::path::PathBuf],
) {
    use std::{collections::BTreeSet, path::PathBuf};
    let mut candidates = vec![
        ("claude", home.join(".local/bin/claude.exe")),
        ("codex", home.join(".local/bin/codex.exe")),
    ];
    for channel in [".vscode", ".vscode-insiders"] {
        if let Ok(entries)=std::fs::read_dir(home.join(channel).join("extensions")) {
            for entry in entries.take(512).flatten() {
                if vscode_package(&entry.file_name().to_string_lossy().to_lowercase()) {
                    candidates.push(("claude",entry.path().join("resources/native-binary/claude.exe")));
                }
            }
        }
    }
    let mut roots: Vec<PathBuf> = vec![home.join(".npm-global"), home.join(".bun/install/global")];
    roots.extend(shim_roots.iter().take(126).cloned());
    let mut seen = BTreeSet::new();
    for root in roots.into_iter().take(128) {
        if !seen.insert(super::discovery::normalize(&root.to_string_lossy())) {
            continue;
        }
        let package = root.join("node_modules/@openai/codex");
        // pnpm dependencies sit beside the canonical wrapper package in its
        // virtual store, rather than necessarily under wrapper/node_modules.
        let sibling = std::fs::canonicalize(&package)
            .ok()
            .and_then(|p| p.parent().map(std::path::Path::to_path_buf));
        for (platform, target) in [
            ("win32-x64", "x86_64-pc-windows-msvc"),
            ("win32-arm64", "aarch64-pc-windows-msvc"),
        ] {
            for bin in ["bin", "codex"] {
                if let Some(sibling) = &sibling {
                    candidates.push((
                        "codex",
                        sibling.join(format!("codex-{platform}/vendor/{target}/{bin}/codex.exe")),
                    ));
                }
                candidates.push((
                    "codex",
                    package.join(format!("vendor/{target}/{bin}/codex.exe")),
                ));
                candidates.push((
                    "codex",
                    package.join(format!(
                        "node_modules/@openai/codex-{platform}/vendor/{target}/{bin}/codex.exe"
                    )),
                ));
                candidates.push((
                    "codex",
                    root.join(format!(
                        "node_modules/@openai/codex-{platform}/vendor/{target}/{bin}/codex.exe"
                    )),
                ));
            }
        }
    }
    for (id, file) in candidates {
        if file.is_file() {
            if let Some(app) = apps.iter_mut().find(|a| a.id == id) {
                app.paths.push(super::discovery::canonical_executable(
                    &file.to_string_lossy(),
                ));
            }
        }
    }
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn idle_vscode_extension_binary_is_discovered_in_both_channels() {
        let temp=tempfile::tempdir().unwrap();
        let mut expected=vec![];
        for channel in [".vscode", ".vscode-insiders"] {
            let file=temp.path().join(format!("{channel}/extensions/anthropic.claude-code-2.1.322-win32-x64/resources/native-binary/claude.exe"));
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();std::fs::write(&file,b"fixture").unwrap();expected.push(file.canonicalize().unwrap());
        }
        let mut apps=super::super::discovery::empty_apps();super::discover_in(&mut apps,temp.path(),&[]);
        let paths=&apps.iter().find(|a|a.id=="claude").unwrap().paths;
        for file in expected {assert!(paths.iter().any(|p|std::path::Path::new(p).canonicalize().unwrap()==file));}
    }
    #[test]
    fn idle_native_clis_are_discovered_without_launching_commands() {
        let temp = tempfile::tempdir().unwrap();
        let home = temp.path().join("user");
        let npm = home.join("npm");
        let claude = home.join(".local/bin/claude.exe");
        let codex=npm.join("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
        for file in [&claude, &codex] {
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(file, b"fixture path only").unwrap();
        }
        let mut apps = super::super::discovery::empty_apps();
        super::discover_in(&mut apps, &home, &[npm]);
        assert!(apps
            .iter()
            .find(|a| a.id == "claude")
            .unwrap()
            .paths
            .iter()
            .any(|p| std::path::Path::new(p).canonicalize().unwrap()
                == claude.canonicalize().unwrap()));
        assert!(apps
            .iter()
            .find(|a| a.id == "codex")
            .unwrap()
            .paths
            .iter()
            .any(|p| std::path::Path::new(p).canonicalize().unwrap()
                == codex.canonicalize().unwrap()));
        assert!(apps
            .iter()
            .find(|a| a.id == "chatgpt")
            .unwrap()
            .paths
            .is_empty());
    }
}
