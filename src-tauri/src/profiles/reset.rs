use super::*;
use std::collections::BTreeSet;
use std::io::Read;

const JOURNAL: &str = "profile-reset.json";
// The sharing watchdog also reads profiles outside the application operation
// lock. Serialize the registry read together with recovery, not just the rename.
static RESET_LOCK: Mutex<()> = Mutex::new(());
pub(super) fn lock() -> Result<std::sync::MutexGuard<'static, ()>, String> {
    RESET_LOCK.lock().map_err(|_| "环境重置锁不可用".into())
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    schema: u8,
    id: String,
    browser: String,
    token: String,
    seed: String,
    reset_proxy: bool,
    updated_at: String,
    accounts: BTreeSet<String>,
    staged: bool,
}
#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct SessionBarrier {
    accounts: BTreeSet<String>,
}

fn read<T: serde::de::DeserializeOwned>(file: &Path) -> Result<Option<T>, String> {
    guard_tree(file)?;
    let source = match std::fs::File::open(file) {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err("无法读取环境重置记录".into()),
    };
    let mut bytes = Vec::new();
    source
        .take(32769)
        .read_to_end(&mut bytes)
        .map_err(|_| "无法读取环境重置记录")?;
    if bytes.len() > 32768 {
        return Err("环境重置记录无效，已停止操作".into());
    }
    serde_json::from_slice(&bytes)
        .map(Some)
        .map_err(|_| "环境重置记录无效，已停止操作".into())
}
fn write(file: &Path, value: &impl Serialize) -> Result<(), String> {
    guard_tree(file)?;
    std::fs::create_dir_all(file.parent().ok_or("环境重置路径无效")?)
        .map_err(|_| "无法创建环境重置目录")?;
    let bytes = serde_json::to_vec(value).map_err(|_| "无法保存环境重置记录")?;
    if bytes.len() > 32768 {
        return Err("环境重置记录无效，已停止操作".into());
    }
    engine::atomic_write(file, &bytes)
}
fn barrier(root: &Path, id: &str) -> Result<PathBuf, String> {
    if !valid_id(id) {
        return Err("副本 ID 无效".into());
    }
    let file = root.join("cloud").join(format!("profile-reset-{id}.json"));
    guard_tree(&file)?;
    Ok(file)
}
pub(crate) fn session_pending(root: &Path, id: &str, account: &str) -> Result<bool, String> {
    Ok(read::<SessionBarrier>(&barrier(root, id)?)?.is_some_and(|b| b.accounts.contains(account)))
}
pub(crate) fn acknowledge_session(root: &Path, id: &str, account: &str) -> Result<(), String> {
    let file = barrier(root, id)?;
    if let Some(mut state) = read::<SessionBarrier>(&file)? {
        if state.accounts.remove(account) {
            write(&file, &state)?;
        }
    }
    Ok(())
}

pub fn reset(root: &Path, id: &str, reset_proxy: bool, runtime: &Runtime) -> Result<(), String> {
    crate::cloud::sharing::editable(root, id)?;
    let _reset = lock()?;
    let mut all = load_unlocked(root)?;
    let p = all
        .iter()
        .find(|p| p.id == id && p.deleted_at.is_none() && p.received_share.is_none())
        .ok_or("副本不存在或已删除")?;
    platform::ensure_profile_closed(&path(root, p)?).map_err(|_| "请先关闭副本再重置环境")?;
    let cloud = crate::cloud::model::Disk::load(root)?;
    let accounts = cloud
        .accounts
        .into_iter()
        .filter_map(|(account, state)| state.bindings.contains_key(id).then_some(account))
        .collect();
    let journal = Journal {
        schema: 1,
        id: id.into(),
        browser: p.browser.clone(),
        token: identifier(),
        seed: crate::fingerprint::seed()?,
        reset_proxy,
        updated_at: stamp(),
        accounts,
        staged: false,
    };
    runtime.1.stop(id);
    runtime.0.lock().map_err(|_| "代理管理器不可用")?.remove(id);
    write(&root.join(JOURNAL), &journal)?;
    recover(root, &mut all)
}

/// An accepted reset is resumed before any profile can be loaded or launched.
/// The old browser directory is staged on the same volume, never merged back.
pub(super) fn recover(root: &Path, all: &mut [Profile]) -> Result<(), String> {
    let journal_file = root.join(JOURNAL);
    let Some(mut journal) = read::<Journal>(&journal_file)? else {
        return Ok(());
    };
    if journal.schema != 1
        || journal.token.len() != 32
        || !journal.token.bytes().all(|b| b.is_ascii_hexdigit())
        || !valid_id(&journal.id)
        || journal
            .accounts
            .iter()
            .any(|id| uuid::Uuid::parse_str(id).is_err())
    {
        return Err("环境重置记录无效，已停止操作".into());
    }
    let p = all
        .iter_mut()
        .find(|p| {
            p.id == journal.id
                && p.browser == journal.browser
                && p.deleted_at.is_none()
                && p.received_share.is_none()
        })
        .ok_or("无法继续重置此副本")?;
    let settings = crate::fingerprint::Settings {
        version: 1,
        seed: journal.seed.clone(),
        enabled: true,
        canvas_noise: true,
        font_noise: true,
        webgl_noise: true,
        audio_noise: true,
        media_noise: true,
    };
    crate::fingerprint::validate(&settings)?;
    let directory = path(root, p)?;
    let backup = root.join(format!("profile-reset-data-{}", journal.token));
    guard_tree(&backup)?;
    platform::ensure_profile_closed(&directory).map_err(|_| "请先关闭副本再重置环境")?;
    if !journal.staged {
        if backup.exists() && directory.exists() {
            return Err("环境重置目录状态异常，已停止操作".into());
        }
        if !backup.exists() && directory.exists() {
            std::fs::rename(&directory, &backup)
                .map_err(|_| "无法暂存浏览器数据，请关闭副本后重试")?;
        }
        journal.staged = true;
        write(&journal_file, &journal)?;
    }
    std::fs::create_dir_all(&directory).map_err(|_| "无法创建重置后的浏览器目录")?;
    // Keep this outside the browser directory and outside cached cloud state.
    // Even an already-loaded cloud connection sees it before importing sessions.
    write(
        &barrier(root, &p.id)?,
        &SessionBarrier {
            accounts: journal.accounts.clone(),
        },
    )?;
    p.preferences = Preferences {
        fingerprint: Some(settings),
        ..Preferences::default()
    };
    if journal.reset_proxy {
        p.proxy = ProxyConfig::system();
    }
    p.preferences_managed = true;
    p.preferences_dirty = true;
    p.updated_at = journal.updated_at;
    p.last_test = None;
    p.last_region = None;
    p.active_preferences = None;
    p.active_proxy = None;
    p.active_proxy_endpoint = None;
    p.relay_port = 0;
    save(root, all)?;
    if backup.exists() {
        std::fs::remove_dir_all(&backup).map_err(|_| "无法清理旧浏览器数据，请重试完成重置")?;
    }
    std::fs::remove_file(&journal_file).map_err(|_| "无法完成环境重置，请重试")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn root() -> tempfile::TempDir {
        tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap()
    }
    fn draft(browser: &str) -> Draft {
        let mut prefs = Preferences::default();
        prefs.language = "ja-JP,ja".into();
        prefs.startup_url = "https://example.invalid/".into();
        prefs.advanced.hardware_concurrency = 4;
        Draft {
            name: "Keep name".into(),
            browser: browser.into(),
            notes: "Keep notes".into(),
            tags: vec!["Keep tag".into()],
            proxy: ProxyConfig {
                mode: "http".into(),
                host: "proxy.example.invalid".into(),
                port: 8080,
                username: "synthetic-user".into(),
                pre_proxy: Some(proxy::LocalProxy {
                    mode: "socks5".into(),
                    host: "127.0.0.1".into(),
                    port: 7890,
                }),
                api_url: Some("https://api.example.invalid/proxy".into()),
                node: None,
                credential_ref: None,
            },
            password: None,
            clear_password: false,
            preferences: prefs,
        }
    }
    #[test]
    fn resets_three_browsers_but_keeps_identity_full_proxy_and_other_profile_data() {
        for browser in ["chrome", "edge", "firefox"] {
            let root = root();
            let p = create(root.path(), draft(browser), None).unwrap();
            let other = create(root.path(), draft(browser), None).unwrap();
            let untouched = path(root.path(), &other).unwrap().join("keep");
            let path = path(root.path(), &p).unwrap();
            std::fs::write(&untouched, b"other-login").unwrap();
            std::fs::create_dir_all(path.join("cache/nested")).unwrap();
            std::fs::write(path.join("cookies.sqlite"), b"synthetic-login").unwrap();
            std::fs::write(path.join("cache/nested/entry"), b"cache").unwrap();
            let mut all = load(root.path()).unwrap();
            let original = all.iter_mut().find(|x| x.id == p.id).unwrap();
            original.proxy.credential_ref = Some("synthetic-secret-ref".into());
            original.credential_refs = vec!["synthetic-secret-ref".into()];
            let kept = original.clone();
            save(root.path(), &all).unwrap();
            reset(root.path(), &p.id, false, &Runtime::default()).unwrap();
            let after = get(root.path(), &p.id, false).unwrap();
            assert_eq!(after.name, kept.name);
            assert_eq!(after.notes, kept.notes);
            assert_eq!(after.tags, kept.tags);
            assert_eq!(after.browser, browser);
            assert_eq!(after.created_at, kept.created_at);
            assert_eq!(after.proxy, kept.proxy);
            assert_eq!(after.credential_refs, kept.credential_refs);
            assert_eq!(after.preferences.language, Preferences::default().language);
            assert_eq!(
                after.preferences.startup_url,
                Preferences::default().startup_url
            );
            assert_eq!(after.preferences.advanced.hardware_concurrency, 0);
            let seed = after.preferences.fingerprint.as_ref().unwrap().seed.clone();
            assert_ne!(seed, kept.preferences.fingerprint.unwrap().seed);
            assert!(after.preferences.fingerprint.as_ref().unwrap().enabled);
            assert!(!path.join("cookies.sqlite").exists());
            assert!(!path.join("cache").exists());
            assert_eq!(std::fs::read(&untouched).unwrap(), b"other-login");
            assert_eq!(
                get(root.path(), &p.id, false)
                    .unwrap()
                    .preferences
                    .fingerprint
                    .unwrap()
                    .seed,
                seed
            );
        }
    }
    #[test]
    fn optional_proxy_reset_removes_chain_api_and_active_credentials() {
        let root = root();
        let p = create(root.path(), draft("edge"), None).unwrap();
        reset(root.path(), &p.id, true, &Runtime::default()).unwrap();
        let after = get(root.path(), &p.id, false).unwrap();
        assert_eq!(after.proxy, ProxyConfig::system());
        assert!(after.last_test.is_none());
        assert!(after.last_region.is_none());
        assert!(after.active_proxy.is_none());
        assert!(after.active_preferences.is_none());
    }
    #[test]
    fn live_lock_and_received_shared_profile_are_rejected_without_mutation() {
        let root = root();
        let p = create(root.path(), draft("edge"), None).unwrap();
        let path = path(root.path(), &p).unwrap();
        let lock = std::fs::OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(path.join("SingletonLock"))
            .unwrap();
        fs2::FileExt::lock_exclusive(&lock).unwrap();
        assert!(reset(root.path(), &p.id, false, &Runtime::default()).is_err());
        assert_eq!(
            get(root.path(), &p.id, false).unwrap().preferences.language,
            "ja-JP,ja"
        );
        fs2::FileExt::unlock(&lock).unwrap();
        drop(lock);
        std::fs::remove_file(path.join("SingletonLock")).unwrap();
        mark_shared(root.path(), &p.id, "synthetic-shared").unwrap();
        assert!(reset(root.path(), &p.id, false, &Runtime::default()).is_err());
    }
    fn journal(p: &Profile) -> Journal {
        Journal {
            schema: 1,
            id: p.id.clone(),
            browser: p.browser.clone(),
            token: identifier(),
            seed: "99".repeat(32),
            reset_proxy: false,
            updated_at: stamp(),
            accounts: BTreeSet::new(),
            staged: false,
        }
    }
    #[test]
    fn resumes_each_staging_boundary_without_changing_identity_or_regenerating_seed() {
        for (renamed, staged) in [(false, false), (true, false), (true, true)] {
            let root = root();
            let p = create(root.path(), draft("firefox"), None).unwrap();
            let directory = path(root.path(), &p).unwrap();
            std::fs::write(directory.join("cookies.sqlite"), b"old-login").unwrap();
            let mut j = journal(&p);
            j.staged = staged;
            let backup = root.path().join(format!("profile-reset-data-{}", j.token));
            if renamed {
                std::fs::rename(&directory, &backup).unwrap();
            }
            write(&root.path().join(JOURNAL), &j).unwrap();
            let after = get(root.path(), &p.id, false).unwrap();
            assert_eq!(after.preferences.fingerprint.unwrap().seed, j.seed);
            assert_eq!(after.proxy, p.proxy);
            assert!(!directory.join("cookies.sqlite").exists());
            assert!(!backup.exists());
            assert!(!root.path().join(JOURNAL).exists());
            assert_eq!(
                get(root.path(), &p.id, false)
                    .unwrap()
                    .preferences
                    .fingerprint
                    .unwrap()
                    .seed,
                j.seed
            );
        }
    }
    #[test]
    fn legacy_reset_preserves_the_other_legacy_browser_and_root_files() {
        let root = root();
        let p = load(root.path())
            .unwrap()
            .into_iter()
            .find(|p| p.browser == "edge")
            .unwrap();
        let target = path(root.path(), &p).unwrap();
        std::fs::create_dir_all(&target).unwrap();
        std::fs::write(target.join("old"), b"old").unwrap();
        let other = root.path().join("browser-chrome");
        std::fs::create_dir_all(&other).unwrap();
        std::fs::write(other.join("keep"), b"keep").unwrap();
        std::fs::write(root.path().join("history.json"), b"[]").unwrap();
        reset(root.path(), &p.id, false, &Runtime::default()).unwrap();
        assert!(!target.join("old").exists());
        assert_eq!(std::fs::read(other.join("keep")).unwrap(), b"keep");
        assert_eq!(
            std::fs::read(root.path().join("history.json")).unwrap(),
            b"[]"
        );
        assert!(get(root.path(), &p.id, false).unwrap().legacy);
    }
    #[test]
    fn corrupt_or_ambiguous_journals_preserve_original_data() {
        let root = root();
        let p = create(root.path(), draft("chrome"), None).unwrap();
        let directory = path(root.path(), &p).unwrap();
        std::fs::write(directory.join("keep"), b"original").unwrap();
        let mut j = journal(&p);
        j.token = "../outside".into();
        write(&root.path().join(JOURNAL), &j).unwrap();
        assert!(load(root.path()).is_err());
        assert_eq!(std::fs::read(directory.join("keep")).unwrap(), b"original");
        j.token = identifier();
        let backup = root.path().join(format!("profile-reset-data-{}", j.token));
        std::fs::create_dir(&backup).unwrap();
        std::fs::write(backup.join("keep"), b"staged").unwrap();
        write(&root.path().join(JOURNAL), &j).unwrap();
        assert!(load(root.path()).is_err());
        assert_eq!(std::fs::read(directory.join("keep")).unwrap(), b"original");
        assert_eq!(std::fs::read(backup.join("keep")).unwrap(), b"staged");
    }
    #[test]
    fn session_acknowledgements_are_account_specific_and_survive_profile_reads() {
        let root = root();
        let p = create(root.path(), draft("chrome"), None).unwrap();
        let a = uuid::Uuid::new_v4().to_string();
        let b = uuid::Uuid::new_v4().to_string();
        let mut disk = crate::cloud::model::Disk::default();
        for account in [&a, &b] {
            let mut state = crate::cloud::model::AccountState::default();
            state.bindings.insert(
                p.id.clone(),
                crate::cloud::model::Binding::new(&uuid::Uuid::new_v4().to_string()),
            );
            disk.accounts.insert(account.clone(), state);
        }
        disk.save(root.path()).unwrap();
        reset(root.path(), &p.id, false, &Runtime::default()).unwrap();
        assert!(session_pending(root.path(), &p.id, &a).unwrap());
        assert!(session_pending(root.path(), &p.id, &b).unwrap());
        acknowledge_session(root.path(), &p.id, &a).unwrap();
        load(root.path()).unwrap();
        assert!(!session_pending(root.path(), &p.id, &a).unwrap());
        assert!(session_pending(root.path(), &p.id, &b).unwrap());
        acknowledge_session(root.path(), &p.id, &b).unwrap();
        assert!(!session_pending(root.path(), &p.id, &b).unwrap());
    }
    #[test]
    fn concurrent_profile_reads_cannot_race_a_reset_or_restore_old_registry_values() {
        let root = root();
        let p = create(root.path(), draft("chrome"), None).unwrap();
        let shared = std::sync::Arc::new(std::sync::Barrier::new(5));
        std::thread::scope(|scope| {
            for _ in 0..4 {
                let gate = shared.clone();
                let root = root.path();
                scope.spawn(move || {
                    gate.wait();
                    for _ in 0..8 {
                        load(root).unwrap();
                    }
                });
            }
            shared.wait();
            reset(root.path(), &p.id, false, &Runtime::default()).unwrap();
        });
        assert_eq!(
            get(root.path(), &p.id, false).unwrap().preferences.language,
            Preferences::default().language
        );
        assert!(!root.path().join(JOURNAL).exists());
    }
    #[test]
    fn sharing_watchdog_snapshot_does_not_wait_for_reset_cleanup_or_resume_a_journal() {
        let root = root();
        let p = create(root.path(), draft("edge"), None).unwrap();
        let directory = path(root.path(), &p).unwrap();
        std::fs::write(directory.join("keep"), b"pending-data").unwrap();
        write(&root.path().join(JOURNAL), &journal(&p)).unwrap();
        let guard = lock().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let snapshot = std::thread::scope(|scope| {
            let path = root.path();
            let worker = scope.spawn(move || {
                let result = super::super::load_snapshot(path);
                let _ = tx.send(result);
            });
            let snapshot = rx.recv_timeout(std::time::Duration::from_millis(500));
            drop(guard);
            worker.join().unwrap();
            snapshot
        });
        assert!(
            snapshot.is_ok(),
            "time-critical snapshot waited on environment cleanup"
        );
        assert_eq!(
            snapshot
                .unwrap()
                .unwrap()
                .into_iter()
                .find(|v| v.id == p.id)
                .unwrap()
                .preferences
                .language,
            "ja-JP,ja"
        );
        assert_eq!(
            std::fs::read(directory.join("keep")).unwrap(),
            b"pending-data"
        );
        assert!(root.path().join(JOURNAL).exists());
    }
    #[cfg(windows)]
    #[test]
    fn interrupted_old_data_cleanup_remains_pending_until_the_locked_file_is_released() {
        use std::os::windows::fs::OpenOptionsExt;
        let root = root();
        let p = create(root.path(), draft("edge"), None).unwrap();
        let directory = path(root.path(), &p).unwrap();
        std::fs::write(directory.join("held"), b"old-login").unwrap();
        let mut j = journal(&p);
        j.staged = true;
        let backup = root.path().join(format!("profile-reset-data-{}", j.token));
        std::fs::rename(&directory, &backup).unwrap();
        write(&root.path().join(JOURNAL), &j).unwrap();
        let held = std::fs::OpenOptions::new()
            .read(true)
            .share_mode(0)
            .open(backup.join("held"))
            .unwrap();
        assert!(load(root.path()).is_err());
        assert!(root.path().join(JOURNAL).exists());
        assert!(backup.join("held").exists());
        drop(held);
        let after = get(root.path(), &p.id, false).unwrap();
        assert_eq!(after.preferences.fingerprint.unwrap().seed, j.seed);
        assert!(!backup.exists());
        assert!(!root.path().join(JOURNAL).exists());
    }
}
