//! Inspect load commands before authorization. The protected copy must match
//! these exact SHA256 bytes AND the caller's live code identity before execution.
use std::path::{Path, PathBuf};
#[derive(Default)]
pub struct Loads {
    pub libraries: Vec<String>,
    pub rpaths: Vec<String>,
}
fn u32at(b: &[u8], p: usize, little: bool) -> Result<u32, String> {
    let x: [u8; 4] = b
        .get(p..p + 4)
        .ok_or("Mac 程序头部不完整")?
        .try_into()
        .unwrap();
    Ok(if little {
        u32::from_le_bytes(x)
    } else {
        u32::from_be_bytes(x)
    })
}
pub fn loads(bytes: &[u8]) -> Result<Option<Loads>, String> {
    let Some(magic) = bytes.get(..4) else {
        return Ok(None);
    };
    if magic == [0xca, 0xfe, 0xba, 0xbe] || magic == [0xca, 0xfe, 0xba, 0xbf] {
        let wide = magic[3] == 0xbf;
        let count = u32at(bytes, 4, false)? as usize;
        if !(1..=16).contains(&count) {
            return Err("Mac 通用程序结构无效".into());
        }
        let mut result = Loads::default();
        for i in 0..count {
            let p = 8 + i * if wide { 32 } else { 20 };
            let (offset, size) = if wide {
                let a = bytes.get(p + 8..p + 24).ok_or("Mac 通用程序结构不完整")?;
                (
                    u64::from_be_bytes(a[..8].try_into().unwrap()),
                    u64::from_be_bytes(a[8..].try_into().unwrap()),
                )
            } else {
                (
                    u32at(bytes, p + 8, false)? as u64,
                    u32at(bytes, p + 12, false)? as u64,
                )
            };
            let end = offset.checked_add(size).ok_or("Mac 程序范围无效")?;
            if end > bytes.len() as u64 || size < 32 {
                return Err("Mac 程序范围无效".into());
            }
            let slice = &bytes[offset as usize..end as usize];
            if !matches!(
                slice.get(..4),
                Some([0xcf, 0xfa, 0xed, 0xfe]) | Some([0xfe, 0xed, 0xfa, 0xcf])
            ) {
                return Err("Mac 通用程序架构无效".into());
            }
            let part = loads(slice)?.ok_or("Mac 通用程序架构无效")?;
            result.libraries.extend(part.libraries);
            result.rpaths.extend(part.rpaths);
        }
        return Ok(Some(result));
    }
    let little = magic == [0xcf, 0xfa, 0xed, 0xfe];
    if !little && magic != [0xfe, 0xed, 0xfa, 0xcf] {
        return Ok(None);
    }
    let count = u32at(bytes, 16, little)? as usize;
    let end = 32usize
        .checked_add(u32at(bytes, 20, little)? as usize)
        .ok_or("Mac 程序加载表无效")?;
    if count > 4096 || end > bytes.len() {
        return Err("Mac 程序加载表无效".into());
    }
    let mut result = Loads::default();
    let mut p = 32;
    for _ in 0..count {
        let command = u32at(bytes, p, little)?;
        let size = u32at(bytes, p + 4, little)? as usize;
        let next = p.checked_add(size).ok_or("Mac 加载命令无效")?;
        if size < 8 || next > end {
            return Err("Mac 加载命令无效".into());
        }
        if [0x0c, 0x80000018, 0x8000001f, 0x80000023, 0x20, 0x8000001c].contains(&command) {
            let start = p
                .checked_add(u32at(bytes, p + 8, little)? as usize)
                .ok_or("Mac 程序依赖无效")?;
            if start < p + 12 || start >= next {
                return Err("Mac 程序依赖无效".into());
            }
            let b = &bytes[start..next];
            let n = b.iter().position(|b| *b == 0).ok_or("Mac 程序依赖不完整")?;
            let name = std::str::from_utf8(&b[..n]).map_err(|_| "Mac 程序依赖名称无效")?;
            if name.chars().any(char::is_control) {
                return Err("Mac 程序依赖名称无效".into());
            }
            if command == 0x8000001c {
                result.rpaths.push(name.into());
            } else {
                result.libraries.push(name.into());
            }
        }
        p = next;
    }
    Ok(Some(result))
}
fn normalized(path: &str) -> Option<String> {
    if !path.starts_with('/') {
        return None;
    }
    let mut parts = vec![];
    for p in path.split('/') {
        match p {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            _ => parts.push(p),
        }
    }
    Some(format!("/{}", parts.join("/")))
}
fn immutable(path: &str) -> bool {
    path.starts_with("/System/Library/") || path.starts_with("/usr/lib/")
}
pub fn validate(image: &str, bundle: Option<&str>, load: &Loads) -> Result<(), String> {
    let inside = |p: &str| immutable(p) || bundle.is_some_and(|b| p.starts_with(&format!("{b}/")));
    let resolve = |p: &str| -> Option<String> {
        let path = if let Some(tail) = p.strip_prefix("@loader_path") {
            format!("{}{tail}", image.rsplit_once('/')?.0)
        } else if let Some(tail) = p.strip_prefix("@executable_path") {
            format!("{}/Contents/MacOS{tail}", bundle?)
        } else {
            p.to_owned()
        };
        normalized(&path)
    };
    for p in &load.rpaths {
        if !resolve(p).is_some_and(|p| inside(&p)) {
            return Err("应用代理程序引用了应用包外的加载路径，请使用完整的 NodeCloak.app".into());
        }
    }
    for p in &load.libraries {
        if let Some(tail) = p.strip_prefix("@rpath/") {
            if tail.split('/').any(|p| p == "..") || load.rpaths.is_empty() {
                return Err("应用代理程序的动态库路径不受保护".into());
            }
            continue;
        }
        if !resolve(p).is_some_and(|p| inside(&p)) {
            return Err("应用代理程序引用了不受保护的外部动态库".into());
        }
    }
    Ok(())
}
pub fn inspect(executable: &Path) -> Result<Vec<(String, String)>, String> {
    use sha2::{Digest, Sha256};
    let executable = executable
        .canonicalize()
        .map_err(|_| "无法读取应用代理程序")?;
    let bundle = executable
        .ancestors()
        .find(|p| p.extension().is_some_and(|s| s == "app"))
        .map(Path::to_owned);
    let mut stack = vec![bundle.clone().unwrap_or_else(|| executable.clone())];
    let mut files = vec![];
    let mut visited = 0;
    let mut total = 0u64;
    while let Some(file) = stack.pop() {
        visited += 1;
        if visited > 20000 {
            return Err("应用代理程序包含的文件超过限制".into());
        }
        let meta = std::fs::symlink_metadata(&file).map_err(|_| "无法检查应用代理程序")?;
        if meta.file_type().is_symlink() {
            let target = file
                .canonicalize()
                .map_err(|_| "应用包包含无效的符号链接")?;
            if !bundle.as_ref().is_some_and(|b| target.starts_with(b)) {
                return Err("应用包引用了外部文件，请重新安装 NodeCloak".into());
            }
            continue;
        }
        if meta.is_dir() {
            for e in std::fs::read_dir(&file).map_err(|_| "无法检查应用包")? {
                stack.push(e.map_err(|_| "无法检查应用包")?.path());
            }
            continue;
        }
        if !meta.is_file() {
            return Err("应用包包含不受支持的文件类型".into());
        }
        if meta.len() > 268_435_456 {
            return Err("应用代理程序文件超过限制".into());
        }
        total = total.checked_add(meta.len()).ok_or("应用包超过限制")?;
        if total > 1_073_741_824 {
            return Err("应用包超过限制".into());
        }
        let bytes = std::fs::read(&file).map_err(|_| "无法读取应用代理程序")?;
        if let Some(load) = loads(&bytes)? {
            validate(
                file.to_str().ok_or("应用路径无效")?,
                bundle.as_deref().and_then(Path::to_str),
                &load,
            )?;
            let target = if let Some(bundle) = &bundle {
                PathBuf::from("Application.app")
                    .join(
                        file.strip_prefix(bundle)
                            .map_err(|_| "应用程序超出了应用包")?,
                    )
                    .to_string_lossy()
                    .into_owned()
            } else {
                "helper".into()
            };
            files.push((target, format!("{:x}", Sha256::digest(&bytes))));
            if files.len() > 256 {
                return Err("应用包含的加载程序超过限制".into());
            }
        }
    }
    if files.is_empty() {
        return Err("没有找到可核验的应用代理程序".into());
    }
    Ok(files)
}
#[cfg(test)]
mod checks {
    use super::*;
    #[test]
    fn loader_paths_are_protected_and_user_writable_rpaths_are_rejected() {
        let image = "/Applications/NodeCloak.app/Contents/MacOS/nodecloak";
        let root = Some("/Applications/NodeCloak.app");
        assert!(validate(
            image,
            root,
            &Loads {
                libraries: vec![
                    "/usr/lib/libSystem.B.dylib".into(),
                    "@rpath/Safe.dylib".into()
                ],
                rpaths: vec!["@executable_path/../Frameworks".into()]
            }
        )
        .is_ok());
        for path in [
            "/Users/test/lib",
            "/usr/local/lib",
            "@loader_path/../../../../Users/test",
        ] {
            assert!(validate(
                image,
                root,
                &Loads {
                    libraries: vec![],
                    rpaths: vec![path.into()]
                }
            )
            .is_err());
        }
        assert!(validate(
            image,
            root,
            &Loads {
                libraries: vec!["/usr/lib/../../Users/test/evil.dylib".into()],
                rpaths: vec![]
            }
        )
        .is_err());
    }
}
