//! Discover native CLI executables, never a shared Node/Python/shell runtime.
pub(super) fn identify_windows(path: &str) -> Option<&'static str> {
    let path = super::discovery::normalize(path);
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
