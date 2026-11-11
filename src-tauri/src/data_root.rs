use std::path::{Path, PathBuf};

// Keep existing browser profiles, font backups and journals in place. Some
// journal entries contain paths, so renaming or copying the tree is unsafe.
const LEGACY_IDENTIFIER: &str = "com.claudeready.desktop";

pub struct DataRoot(pub PathBuf);

fn has_environment_data(root: &Path) -> bool {
    root.join("history.json").is_file()
        || root.join("profiles.json").is_file()
        || root.join("cloud/state.json").is_file()
        || root.join("terminal-environments/key-id").is_file()
        || root.join("font-backups").is_dir()
        || root.join("profiles").is_dir()
        || ["browser-chrome", "browser-edge", "browser-firefox"]
            .iter()
            .any(|name| root.join(name).is_dir())
}

pub fn resolve(current: PathBuf) -> PathBuf {
    if !has_environment_data(&current) {
        if let Some(parent) = current.parent() {
            for identifier in ["com.claudedone", LEGACY_IDENTIFIER] {
                let legacy = parent.join(identifier);
                if has_environment_data(&legacy) {
                    return legacy;
                }
            }
        }
    }
    current
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn account_and_terminal_data_keep_the_current_xdg_directory() {
        for marker in ["cloud/state.json", "terminal-environments/key-id"] {
            let directory = tempfile::tempdir().unwrap();
            let data_home = directory.path().join("custom data home");
            let current = data_home.join("com.nodecloak");
            let legacy = data_home.join("com.claudedone");
            std::fs::create_dir_all(legacy.join("profiles")).unwrap();
            let file = current.join(marker);
            std::fs::create_dir_all(file.parent().unwrap()).unwrap();
            std::fs::write(&file, "fixture").unwrap();
            assert_eq!(resolve(current.clone()), current);
            assert_eq!(std::fs::read_to_string(file).unwrap(), "fixture");
        }
    }

    #[test]
    fn nodecloak_reuses_claudedone_before_older_legacy_data() {
        let directory = tempfile::tempdir().unwrap();
        let previous = directory.path().join("com.claudedone");
        let oldest = directory.path().join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(previous.join("profiles")).unwrap();
        std::fs::create_dir_all(oldest.join("browser-edge")).unwrap();
        let current = directory.path().join("com.nodecloak");
        assert_eq!(resolve(current.clone()), previous);
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(current.join("profiles.json"), "{}").unwrap();
        assert_eq!(resolve(current.clone()), current);
    }
    #[test]
    fn existing_profiles_are_reused_without_moving_them() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = directory.path().join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(legacy.join("browser-edge")).unwrap();
        std::fs::write(legacy.join("history.json"), "[]").unwrap();
        let current = directory.path().join("com.claudedone");
        std::fs::create_dir_all(current.join("EBWebView")).unwrap();
        assert_eq!(resolve(current), legacy);
        assert!(legacy.join("history.json").exists());
    }
    #[test]
    fn fresh_install_and_existing_new_data_use_the_new_identifier() {
        let directory = tempfile::tempdir().unwrap();
        let current = directory.path().join("com.claudedone");
        assert_eq!(resolve(current.clone()), current);
        let legacy = directory.path().join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(legacy.join("browser-edge")).unwrap();
        std::fs::create_dir_all(current.join("browser-chrome")).unwrap();
        assert_eq!(resolve(current.clone()), current);
    }
    #[test]
    fn profile_registry_prevents_reopening_legacy_data_after_default_profiles_are_deleted() {
        let directory = tempfile::tempdir().unwrap();
        let legacy = directory.path().join(LEGACY_IDENTIFIER);
        std::fs::create_dir_all(legacy.join("browser-edge")).unwrap();
        let current = directory.path().join("com.claudedone");
        std::fs::create_dir_all(&current).unwrap();
        std::fs::write(
            current.join("profiles.json"),
            "{\"schema\":1,\"profiles\":[]}",
        )
        .unwrap();
        assert_eq!(resolve(current.clone()), current);
    }
}
