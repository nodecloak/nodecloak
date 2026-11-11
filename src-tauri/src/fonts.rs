use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::Write,
    path::{Component, Path, PathBuf},
};

#[derive(Clone, Serialize, Deserialize, Debug, PartialEq)]
pub struct Registration {
    pub name: String,
    pub value: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct FontItem {
    pub id: String,
    pub label: String,
    pub file_name: String,
    pub bytes: u64,
}
#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Catalog {
    pub items: Vec<FontItem>,
    pub protected: Vec<String>,
    pub note: String,
}
#[derive(Clone, Serialize, Deserialize, Debug)]
#[serde(rename_all = "camelCase")]
pub struct Snapshot {
    pub file_name: String,
    pub label: String,
    pub registrations: Vec<Registration>,
    pub digest: String,
}
#[derive(Serialize)]
pub struct FontOutcome {
    pub id: String,
    pub label: String,
    pub success: bool,
    pub message: String,
}

fn user_font_dir() -> Result<PathBuf, String> {
    #[cfg(target_os = "windows")]
    {
        return std::env::var_os("LOCALAPPDATA")
            .map(|p| PathBuf::from(p).join("Microsoft/Windows/Fonts"))
            .ok_or("用户字体目录不可用".into());
    }
    #[cfg(target_os = "macos")]
    {
        return Ok(crate::platform::home_dir().join("Library/Fonts"));
    }
    #[cfg(target_os = "linux")]
    {
        return Ok(linux_user_font_dir(&crate::platform::home_dir(), std::env::var_os("XDG_DATA_HOME").as_deref()));
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Err("当前系统不支持字体管理".into())
}
#[cfg(any(target_os = "linux", test))]
fn linux_user_font_dir(home: &Path, data_home: Option<&std::ffi::OsStr>) -> PathBuf {
    data_home.map(Path::new).filter(|path| path.is_absolute())
        .map(Path::to_path_buf).unwrap_or_else(|| home.join(".local/share")).join("fonts")
}
pub(crate) fn candidate(name: &str) -> bool {
    let compact = |value: &str| {
        value
            .to_lowercase()
            .chars()
            .filter(|c| !c.is_whitespace() && *c != '-' && *c != '_')
            .collect::<String>()
    };
    let n = compact(name);
    [
        "microsoft yahei",
        "simsun",
        "nsimsun",
        "simhei",
        "fangsong",
        "kaiti",
        "dengxian",
        "pingfang",
        "songti",
        "heiti",
        "stheiti",
        "noto sans cjk sc",
        "noto serif cjk sc",
        "source han sans sc",
        "source han serif sc",
        "wenquanyi",
        "微软雅黑",
        "宋体",
        "仿宋",
        "楷体",
        "黑体",
        "思源",
        "苹方",
        "文泉驿",
    ]
    .iter()
    .any(|s| n.contains(&compact(s)))
}
fn safe_file_name(name: &str) -> bool {
    let path = Path::new(name);
    !name.is_empty()
        && name.len() <= 240
        && !name.contains(['/', '\\', ':', '\0'])
        && path.components().count() == 1
        && matches!(path.components().next(), Some(Component::Normal(_)))
        && path
            .extension()
            .and_then(|s| s.to_str())
            .is_some_and(|s| ["ttf", "otf", "ttc"].contains(&s.to_ascii_lowercase().as_str()))
}
fn id(name: &str) -> String {
    format!("{:x}", Sha256::digest(name.as_bytes()))
}
fn hash_file(path: &Path) -> Result<String, String> {
    if fs::metadata(path).map_err(|e| e.to_string())?.len() > 128 * 1024 * 1024 {
        return Err("字体超过备份大小限制".into());
    }
    Ok(format!(
        "{:x}",
        Sha256::digest(fs::read(path).map_err(|e| e.to_string())?)
    ))
}
// Deny symlinks / junctions for every component, including the fixed root.
fn no_links(path: &Path) -> Result<(), String> {
    for p in path.ancestors() {
        match fs::symlink_metadata(p) {
            Ok(m) => {
                if m.file_type().is_symlink() {
                    return Err("字体或备份路径包含符号链接，本次未操作".into());
                }
                #[cfg(target_os = "windows")]
                {
                    use std::os::windows::fs::MetadataExt;
                    if m.file_attributes() & 0x400 != 0 {
                        return Err("字体或备份路径包含重解析点，本次未操作".into());
                    }
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    Ok(())
}
#[cfg(target_os = "windows")]
const FONT_KEY: &str = r"Software\Microsoft\Windows NT\CurrentVersion\Fonts";
#[cfg(target_os = "windows")]
fn registrations() -> Result<Vec<Registration>, String> {
    use winreg::{enums::*, RegKey};
    let key = match RegKey::predef(HKEY_CURRENT_USER).open_subkey(FONT_KEY) {
        Ok(k) => k,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(vec![]),
        Err(e) => return Err(e.to_string()),
    };
    let mut out = vec![];
    for pair in key.enum_values() {
        let (name, value) = pair.map_err(|e| e.to_string())?;
        if value.vtype == REG_SZ {
            let value: String = key.get_value(&name).map_err(|e| e.to_string())?;
            out.push(Registration { name, value });
        }
    }
    Ok(out)
}
#[cfg(not(target_os = "windows"))]
fn registrations() -> Result<Vec<Registration>, String> {
    Ok(vec![])
}
fn registration_matches(reg: &Registration, dir: &Path, file: &str) -> bool {
    let value = Path::new(&reg.value);
    let path = if value.is_absolute() {
        value.to_path_buf()
    } else {
        dir.join(value)
    };
    let normalize = |p: &Path| p.to_string_lossy().replace('/', "\\").to_lowercase();
    normalize(&path) == normalize(&dir.join(file))
}
fn entries() -> Result<Vec<(FontItem, Vec<Registration>)>, String> {
    let dir = user_font_dir()?;
    no_links(&dir)?;
    if !dir.exists() {
        return Ok(vec![]);
    }
    let regs = registrations()?;
    let mut out = vec![];
    for e in fs::read_dir(&dir).map_err(|e| e.to_string())? {
        let e = e.map_err(|e| e.to_string())?;
        let name = e.file_name().to_string_lossy().to_string();
        if !safe_file_name(&name)
            || no_links(&e.path()).is_err()
            || !e.file_type().map_err(|e| e.to_string())?.is_file()
        {
            continue;
        }
        let matches: Vec<_> = regs
            .iter()
            .filter(|r| registration_matches(r, &dir, &name))
            .cloned()
            .collect();
        #[cfg(target_os = "windows")]
        if matches.is_empty() || !matches.iter().any(|r| candidate(&r.name)) {
            continue;
        }
        #[cfg(not(target_os = "windows"))]
        if !candidate(&name) {
            continue;
        }
        let label = if matches.is_empty() {
            name.clone()
        } else {
            matches
                .iter()
                .map(|r| r.name.clone())
                .collect::<Vec<_>>()
                .join(" / ")
        };
        out.push((
            FontItem {
                id: id(&name),
                label,
                file_name: name,
                bytes: e.metadata().map_err(|e| e.to_string())?.len(),
            },
            matches,
        ));
    }
    out.sort_by(|a, b| a.0.label.cmp(&b.0.label));
    Ok(out)
}
pub fn catalog() -> Result<Catalog, String> {
    Ok(Catalog{items:entries()?.into_iter().map(|(i,_)|i).collect(),protected:crate::platform::font_evidence(),note:"仅处理当前用户目录中识别到的中文字体，影响该用户的所有应用。系统字体、所有用户字体及字体组件不自动卸载。网页宽度检测可能误判，处理后需完全退出浏览器；Windows 字体缓存可能需要注销或重启。".into()})
}
fn backup_path(root: &Path, token: &str, name: &str) -> Result<PathBuf, String> {
    if token.is_empty()
        || token.len() > 160
        || !token.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        || !safe_file_name(name)
    {
        return Err("字体备份标识无效".into());
    }
    let path = root.join("font-backups").join(token).join(name);
    no_links(&path)?;
    Ok(path)
}
fn copy_verified(source: &Path, target: &Path, digest: &str) -> Result<(), String> {
    no_links(source)?;
    no_links(target)?;
    if hash_file(source)? != digest {
        return Err("字体内容已变化，本次未覆盖".into());
    }
    if target.exists() {
        if hash_file(target)? == digest {
            return Ok(());
        }
        return Err("目标位置已有不同内容，本次未覆盖".into());
    }
    fs::create_dir_all(target.parent().ok_or("字体路径无效")?).map_err(|e| e.to_string())?;
    let bytes = fs::read(source).map_err(|e| e.to_string())?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(target)
        .map_err(|e| e.to_string())?;
    file.write_all(&bytes)
        .and_then(|_| file.sync_all())
        .map_err(|e| e.to_string())?;
    if hash_file(target)? != digest {
        return Err("字体备份核验失败，未卸载原文件".into());
    }
    Ok(())
}
pub fn prepare(root: &Path, font_id: &str, token: &str) -> Result<Snapshot, String> {
    let (item, registrations) = entries()?
        .into_iter()
        .find(|(i, _)| i.id == font_id)
        .ok_or("字体不在当前可处理列表中，请重新扫描")?;
    let source = user_font_dir()?.join(&item.file_name);
    let digest = hash_file(&source)?;
    copy_verified(
        &source,
        &backup_path(root, token, &item.file_name)?,
        &digest,
    )?;
    Ok(Snapshot {
        file_name: item.file_name,
        label: item.label,
        registrations,
        digest,
    })
}
fn validate(snapshot: &Snapshot, dir: &Path) -> Result<(), String> {
    if !safe_file_name(&snapshot.file_name)
        || snapshot.digest.len() != 64
        || !snapshot.digest.chars().all(|c| c.is_ascii_hexdigit())
        || snapshot.registrations.len() > 64
        || snapshot.registrations.iter().any(|r| {
            r.name.len() > 500
                || r.name.contains('\0')
                || r.value.contains('\0')
                || !registration_matches(r, dir, &snapshot.file_name)
        })
    {
        return Err("字体备份内容无效".into());
    }
    #[cfg(target_os = "windows")]
    if snapshot.registrations.is_empty()
        || !snapshot.registrations.iter().any(|r| candidate(&r.name))
    {
        return Err("缺少已识别的用户字体注册备份".into());
    }
    Ok(())
}
#[cfg(target_os = "windows")]
fn change_registrations(s: &Snapshot, remove: bool) -> Result<(), String> {
    use winreg::{enums::*, RegKey};
    let (key, _) = RegKey::predef(HKEY_CURRENT_USER)
        .create_subkey(FONT_KEY)
        .map_err(|e| e.to_string())?;
    for r in &s.registrations {
        match key.get_value::<String, _>(&r.name) {
            Ok(v) if v != r.value => return Err("字体注册已被其他程序修改，本次未覆盖".into()),
            Ok(_) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.to_string()),
        }
    }
    for r in &s.registrations {
        if remove {
            match key.delete_value(&r.name) {
                Ok(()) => {}
                Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
                Err(e) => return Err(e.to_string()),
            }
        } else {
            key.set_value(&r.name, &r.value)
                .map_err(|e| e.to_string())?;
        }
    }
    for r in &s.registrations {
        match key.get_value::<String, _>(&r.name) {
            Err(e) if remove && e.kind() == std::io::ErrorKind::NotFound => {}
            Ok(value) if !remove && value == r.value => {}
            _ => return Err("字体注册变更后核验失败，请从记录恢复备份".into()),
        }
    }
    Ok(())
}
#[cfg(not(target_os = "windows"))]
fn change_registrations(_: &Snapshot, _: bool) -> Result<(), String> {
    Ok(())
}
#[cfg(target_os = "windows")]
fn notify_font(path: &Path, remove: bool) -> bool {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::{
        Graphics::Gdi::{AddFontResourceW, RemoveFontResourceW},
        UI::WindowsAndMessaging::{PostMessageW, HWND_BROADCAST, WM_FONTCHANGE},
    };
    let wide: Vec<u16> = path.as_os_str().encode_wide().chain(Some(0)).collect();
    // System resource notification only; no browser or other application is terminated.
    unsafe {
        let changed = if remove {
            RemoveFontResourceW(wide.as_ptr()) != 0
        } else {
            AddFontResourceW(wide.as_ptr()) != 0
        };
        PostMessageW(HWND_BROADCAST, WM_FONTCHANGE, 0, 0);
        changed
    }
}
#[cfg(not(target_os = "windows"))]
fn notify_font(_: &Path, _: bool) -> bool {
    true
}
pub fn apply(root: &Path, token: &str, s: &Snapshot, undo: bool) -> Result<(), String> {
    let dir = user_font_dir()?;
    validate(s, &dir)?;
    let original = dir.join(&s.file_name);
    let backup = backup_path(root, token, &s.file_name)?;
    no_links(&original)?;
    if hash_file(&backup)? != s.digest {
        return Err("字体备份缺失或内容已变化，未操作原文件".into());
    }
    if undo {
        // Check registration conflicts before copying; never overwrite a new font.
        #[cfg(target_os = "windows")]
        for r in &s.registrations {
            if registrations()?
                .iter()
                .any(|v| v.name == r.name && v.value != r.value)
            {
                return Err("字体注册已变化，本次未覆盖".into());
            }
        }
        copy_verified(&backup, &original, &s.digest)?;
        change_registrations(s, false)?;
        notify_font(&original, false);
    } else {
        if !original.is_file() || hash_file(&original)? != s.digest {
            return Err("字体已被移除或修改，请重新扫描".into());
        }
        let current = entries()?;
        if !current
            .iter()
            .any(|(i, regs)| i.file_name == s.file_name && *regs == s.registrations)
        {
            return Err("字体安装状态已变化，请重新扫描".into());
        }
        change_registrations(s, true)?;
        let unloaded = notify_font(&original, true);
        if let Err(e) = fs::remove_file(&original) {
            let rollback = change_registrations(s, false);
            if unloaded {
                notify_font(&original, false);
            }
            return Err(format!(
                "字体文件仍被使用或无法卸载：{e}。请保存工作并退出使用它的应用后重试。{}",
                if rollback.is_ok() {
                    "注册信息已恢复。"
                } else {
                    "注册恢复未完成，请从修复记录恢复备份。"
                }
            ));
        }
        if original.exists() {
            return Err("字体文件移除后核验失败".into());
        }
    }
    #[cfg(target_os = "linux")]
    {
        let result = crate::platform::command("/usr/bin/fc-cache").arg("--force").arg(&dir)
            .output().map_err(|_| "字体文件已更新，但无法刷新字体缓存，请安装 fontconfig 后重试。")?;
        if !result.status.success() {
            return Err("字体文件已更新，但字体缓存刷新失败，请退出浏览器后重试。".into());
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_fonts_use_absolute_xdg_data_home_and_never_relative_overrides() {
        let dir = tempfile::tempdir().unwrap();
        let home = dir.path().join("home");
        let custom = dir.path().join("custom data");
        assert_eq!(linux_user_font_dir(&home, Some(custom.as_os_str())), custom.join("fonts"));
        for value in [None, Some(std::ffi::OsStr::new("")), Some(std::ffi::OsStr::new("relative/fonts"))] {
            assert_eq!(linux_user_font_dir(&home, value), home.join(".local/share/fonts"));
        }
    }
    #[test]
    fn recognizes_common_cjk_file_names_as_well_as_family_names() {
        assert!(candidate("NotoSansCJKsc-Regular.otf"));
        assert!(candidate("SourceHanSansSC-Regular.otf"));
        assert!(!candidate("Arial.ttf"));
    }
    #[test]
    fn paths_cannot_escape_user_font_or_backup_directories() {
        for name in [
            "../x.ttf",
            "C:\\Windows\\Fonts\\simsun.ttc",
            "x/y.otf",
            "..",
            "x.txt",
        ] {
            assert!(!safe_file_name(name));
        }
        assert!(safe_file_name("Noto Sans CJK SC.otf"));
        let d = tempfile::tempdir().unwrap();
        assert!(backup_path(d.path(), "../bad", "font.ttf").is_err());
        assert!(!registration_matches(
            &Registration {
                name: "SimSun".into(),
                value: "../Windows/simsun.ttc".into()
            },
            Path::new("user-fonts"),
            "simsun.ttc"
        ));
    }
    #[test]
    fn backup_and_restore_keep_exact_bytes_and_refuse_conflicts() {
        let d = tempfile::tempdir().unwrap();
        // macOS's /var is a system symlink; keep the fixture itself in its
        // canonical directory so the production symlink guard stays enabled.
        let directory = fs::canonicalize(d.path()).unwrap();
        let original = directory.join("Noto Sans CJK SC.otf");
        let saved = directory.join("saved.otf");
        fs::write(&original, b"font fixture").unwrap();
        let hash = hash_file(&original).unwrap();
        copy_verified(&original, &saved, &hash).unwrap();
        fs::remove_file(&original).unwrap();
        copy_verified(&saved, &original, &hash).unwrap();
        assert_eq!(fs::read(&original).unwrap(), b"font fixture");
        fs::write(&original, b"new version").unwrap();
        assert!(copy_verified(&saved, &original, &hash).is_err());
        assert_eq!(fs::read(&original).unwrap(), b"new version");
        fs::write(&saved, b"broken backup").unwrap();
        assert!(copy_verified(&saved, &original, &hash).is_err());
    }
}
