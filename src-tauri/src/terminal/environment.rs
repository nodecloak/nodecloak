use super::Tool;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, HashSet};

#[derive(Clone, Copy, Default, Deserialize, Serialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum Mode {
    Inherit,
    #[default]
    Custom,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub struct Variable {
    pub name: String,
    pub value: String,
}
#[derive(Clone, Deserialize, Serialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Environment {
    pub mode: Mode,
    pub timezone: String,
    pub lang: String,
    pub lc_all: String,
    pub variables: Vec<Variable>,
}
impl Default for Environment {
    fn default() -> Self {
        Self {
            mode: Mode::Custom,
            timezone: "Asia/Singapore".into(),
            lang: "en_US.UTF-8".into(),
            lc_all: "en_US.UTF-8".into(),
            variables: vec![],
        }
    }
}
impl Environment {
    pub fn overrides(&self) -> Result<BTreeMap<String, String>, String> {
        if self.variables.len() > 32
            || self.timezone.len() > 100
            || self.lang.len() > 100
            || self.lc_all.len() > 100
            || self
                .variables
                .iter()
                .map(|v| v.name.len() + v.value.len())
                .sum::<usize>()
                > 24576
        {
            return Err("终端环境配置过大，最多 32 个变量、共 24 KiB。".into());
        }
        let mut result = BTreeMap::new();
        if self.mode == Mode::Inherit {
            return Ok(result);
        }
        if !self.timezone.is_empty() {
            if self.timezone.parse::<chrono_tz::Tz>().is_err() {
                return Err("请填写有效的 IANA 时区，例如 Asia/Singapore。".into());
            }
            result.insert("TZ".into(), self.timezone.clone());
        }
        for (key, value) in [("LANG", &self.lang), ("LC_ALL", &self.lc_all)] {
            if !value.is_empty() {
                if !value
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || b"_.@-".contains(&c))
                {
                    return Err("语言格式无效，例如 en_US.UTF-8。".into());
                }
                result.insert(key.into(), value.clone());
            }
        }
        let mut names = HashSet::new();
        for variable in &self.variables {
            let name = &variable.name;
            if name.is_empty() && variable.value.is_empty() {
                continue;
            }
            if name.len() > 128
                || !name.bytes().enumerate().all(|(i, c)| {
                    c == b'_' || c.is_ascii_alphabetic() || (i > 0 && c.is_ascii_digit())
                })
                || name.is_empty()
            {
                return Err("变量名需以字母或下划线开头，仅包含字母、数字和下划线。".into());
            }
            let upper = name.to_ascii_uppercase();
            if upper.starts_with("NODECLOAK_") || ["TZ", "LANG", "LC_ALL"].contains(&upper.as_str())
            {
                return Err("TZ、LANG、LC_ALL 请使用上方字段；NODECLOAK_ 为保留前缀。".into());
            }
            if !names.insert(upper) {
                return Err("环境变量名重复，请检查大小写。".into());
            }
            if variable.value.contains('\0') {
                return Err("环境变量值不能包含空字符。".into());
            }
            result.insert(name.clone(), variable.value.clone());
        }
        Ok(result)
    }
}
use base64::{engine::general_purpose::STANDARD as B64, Engine};
use chacha20poly1305::{
    aead::{Aead, KeyInit, Payload},
    ChaCha20Poly1305, Nonce,
};
use std::{
    path::{Path, PathBuf},
    sync::Mutex,
};
static STORE: Mutex<()> = Mutex::new(());
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Encrypted {
    version: u8,
    nonce: String,
    ciphertext: String,
}
fn entry(root: &Path, create: bool) -> Result<keyring::Entry, String> {
    // Keep the reference with the encrypted files so moving the app data folder
    // on this device does not lose access to the OS-protected encryption key.
    let reference = root.join("terminal-environments").join("key-id");
    crate::profiles::guard_tree(&reference)?;
    let id = if reference.exists() {
        if std::fs::metadata(&reference)
            .map_err(|_| "终端环境密钥引用不可用")?
            .len()
            > 64
        {
            return Err("终端环境密钥引用无效".into());
        }
        let id = std::fs::read_to_string(&reference).map_err(|_| "终端环境密钥引用不可用")?;
        uuid::Uuid::parse_str(&id)
            .map_err(|_| "终端环境密钥引用无效")?
            .to_string()
    } else if create {
        let id = uuid::Uuid::new_v4().to_string();
        std::fs::create_dir_all(reference.parent().ok_or("终端配置目录不可用")?)
            .map_err(|_| "无法创建终端配置目录")?;
        crate::engine::atomic_write(&reference, id.as_bytes())?;
        id
    } else {
        return Err("终端环境密钥引用缺失".into());
    };
    keyring::Entry::new("com.nodecloak.terminal", &format!("key-{id}"))
        .map_err(|_| "系统凭据存储不可用".into())
}
fn key(root: &Path, create: bool) -> Result<zeroize::Zeroizing<[u8; 32]>, String> {
    let entry = entry(root, create)?;
    match entry.get_password() {
        Ok(value) => {
            let value = zeroize::Zeroizing::new(value);
            let decoded = zeroize::Zeroizing::new(
                B64.decode(value.as_bytes())
                    .map_err(|_| "终端环境密钥格式无效")?,
            );
            let bytes: [u8; 32] = decoded
                .as_slice()
                .try_into()
                .map_err(|_| "终端环境密钥格式无效")?;
            Ok(zeroize::Zeroizing::new(bytes))
        }
        Err(keyring::Error::NoEntry) if create => {
            let mut bytes = zeroize::Zeroizing::new([0u8; 32]);
            getrandom::fill(bytes.as_mut()).map_err(|_| "系统随机数不可用")?;
            let encoded = zeroize::Zeroizing::new(B64.encode(bytes.as_ref()));
            entry
                .set_password(&encoded)
                .map_err(|_| "无法保存终端环境密钥，请检查系统凭据存储。")?;
            Ok(bytes)
        }
        Err(keyring::Error::NoEntry) => Err("终端环境密钥缺失，无法读取已保存的配置。".into()),
        Err(_) => Err("无法读取终端环境，请检查系统凭据存储。".into()),
    }
}
fn config_file(root: &Path, tool: Tool) -> Result<PathBuf, String> {
    let file = root
        .join("terminal-environments")
        .join(format!("{}.json", tool.slug()));
    crate::profiles::guard_tree(&file)?;
    Ok(file)
}
fn seal(config: &Environment, tool: Tool, key: &[u8; 32]) -> Result<Vec<u8>, String> {
    config.overrides()?;
    let mut nonce = [0u8; 12];
    getrandom::fill(&mut nonce).map_err(|_| "系统随机数不可用")?;
    let plaintext =
        zeroize::Zeroizing::new(serde_json::to_vec(config).map_err(|_| "终端环境格式无效")?);
    let aad = format!("NodeCloak terminal environment v1:{}", tool.slug());
    let ciphertext = ChaCha20Poly1305::new_from_slice(key)
        .map_err(|_| "终端环境密钥格式无效")?
        .encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: &plaintext,
                aad: aad.as_bytes(),
            },
        )
        .map_err(|_| "无法加密终端环境")?;
    serde_json::to_vec(&Encrypted {
        version: 1,
        nonce: B64.encode(nonce),
        ciphertext: B64.encode(ciphertext),
    })
    .map_err(|_| "终端环境格式无效".into())
}
fn unseal(bytes: &[u8], tool: Tool, key: &[u8; 32]) -> Result<Environment, String> {
    let stored: Encrypted =
        serde_json::from_slice(bytes).map_err(|_| "已保存的终端环境格式无效")?;
    if stored.version != 1 {
        return Err("终端环境版本不受支持".into());
    }
    let nonce = B64
        .decode(&stored.nonce)
        .map_err(|_| "已保存的终端环境格式无效")?;
    if nonce.len() != 12 {
        return Err("已保存的终端环境格式无效".into());
    }
    let ciphertext = B64
        .decode(&stored.ciphertext)
        .map_err(|_| "已保存的终端环境格式无效")?;
    let aad = format!("NodeCloak terminal environment v1:{}", tool.slug());
    let plaintext = zeroize::Zeroizing::new(
        ChaCha20Poly1305::new_from_slice(key)
            .map_err(|_| "终端环境密钥格式无效")?
            .decrypt(
                Nonce::from_slice(&nonce),
                Payload {
                    msg: &ciphertext,
                    aad: aad.as_bytes(),
                },
            )
            .map_err(|_| "无法解密终端环境，请检查配置完整性与系统凭据存储。")?,
    );
    let config: Environment =
        serde_json::from_slice(&plaintext).map_err(|_| "已保存的终端环境格式无效")?;
    config.overrides()?;
    Ok(config)
}
pub fn read(root: &Path, tool: Tool) -> Result<Environment, String> {
    let _lock = STORE.lock().map_err(|_| "终端配置存储不可用")?;
    let file = config_file(root, tool)?;
    if !file.exists() {
        // Import the brief pre-encryption development format without deleting it.
        let legacy = keyring::Entry::new("com.nodecloak.terminal", tool.slug())
            .map_err(|_| "系统凭据存储不可用")?;
        return match legacy.get_password() {
            Ok(value) => {
                let value = zeroize::Zeroizing::new(value);
                let config: Environment =
                    serde_json::from_str(&value).map_err(|_| "已保存的终端环境格式无效")?;
                config.overrides()?;
                Ok(config)
            }
            Err(keyring::Error::NoEntry) => Ok(Environment::default()),
            Err(_) => Err("无法读取终端环境，请检查系统凭据存储。".into()),
        };
    }
    if std::fs::metadata(&file)
        .map_err(|_| "无法读取终端环境")?
        .len()
        > 512 * 1024
    {
        return Err("已保存的终端环境格式无效".into());
    }
    let bytes = std::fs::read(file).map_err(|_| "无法读取终端环境")?;
    unseal(&bytes, tool, &*key(root, false)?)
}
pub fn save(root: &Path, tool: Tool, environment: &Environment) -> Result<(), String> {
    let _lock = STORE.lock().map_err(|_| "终端配置存储不可用")?;
    environment.overrides()?;
    let file = config_file(root, tool)?;
    let encrypted = seal(environment, tool, &*key(root, true)?)?;
    std::fs::create_dir_all(file.parent().ok_or("终端配置目录不可用")?)
        .map_err(|_| "无法创建终端配置目录")?;
    crate::engine::atomic_write(&file, &encrypted)
        .map_err(|_| "无法保存终端环境，请检查配置目录。".into())
}
#[cfg(any(target_os = "macos", target_os = "linux", test))]
pub(super) fn mac_exports(values: &BTreeMap<String, String>) -> String {
    values
        .iter()
        .map(|(key, value)| format!("export {key}='{}'\n", value.replace('\'', "'\\''")))
        .collect()
}
#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    #[ignore = "Requires an unlocked Secret Service in an isolated D-Bus session"]
    fn native_linux_credential_roundtrip() {
        let service = format!("com.nodecloak.test.{}", uuid::Uuid::new_v4());
        let account = format!("fixture-{}", uuid::Uuid::new_v4());
        struct Cleanup(keyring::Entry);
        impl Drop for Cleanup {
            fn drop(&mut self) { let _ = self.0.delete_credential(); }
        }
        let cleanup = Cleanup(keyring::Entry::new(&service, &account).unwrap());
        assert!(matches!(cleanup.0.get_password(), Err(keyring::Error::NoEntry)));
        cleanup.0.set_password("synthetic-linux-credential").unwrap();
        // Reconstruct the entry to verify the real persistent backend, rather
        // than an in-memory entry or a mock selected on unsupported platforms.
        let reopened = keyring::Entry::new(&service, &account).unwrap();
        assert_eq!(reopened.get_password().unwrap(), "synthetic-linux-credential");
        reopened.delete_credential().unwrap();
        assert!(matches!(cleanup.0.get_password(), Err(keyring::Error::NoEntry)));
    }
    #[test]
    #[ignore = "Uses a unique synthetic OS credential and removes it afterwards"]
    fn native_terminal_environment_credentials_round_trip() {
        let root = tempfile::tempdir().unwrap();
        struct Credential(keyring::Entry);
        impl Drop for Credential {
            fn drop(&mut self) {
                let _ = self.0.delete_credential();
            }
        }
        let _entry = Credential(entry(root.path(), true).unwrap());
        let mut config = Environment::default();
        config.timezone = "Europe/Paris".into();
        config.variables = vec![Variable {
            name: "SYNTHETIC_TEST_KEY".into(),
            value: "synthetic-only-not-a-real-key".repeat(500),
        }];
        save(root.path(), Tool::Shell, &config).unwrap();
        let loaded = read(root.path(), Tool::Shell).unwrap();
        assert_eq!(
            serde_json::to_value(&loaded).unwrap(),
            serde_json::to_value(&config).unwrap()
        );
        let file = config_file(root.path(), Tool::Shell).unwrap();
        let text = std::fs::read_to_string(&file).unwrap();
        assert!(!text.contains("synthetic-only-not-a-real-key"));
        assert!(!text.contains("SYNTHETIC_TEST_KEY"));
        let mut bad = config.clone();
        bad.variables[0].name = "invalid=name".into();
        assert!(save(root.path(), Tool::Shell, &bad).is_err());
        assert_eq!(
            serde_json::to_value(read(root.path(), Tool::Shell).unwrap()).unwrap(),
            serde_json::to_value(config).unwrap()
        );
    }
    #[test]
    fn encrypted_settings_reject_tampering_wrong_keys_and_tool_substitution() {
        let config = Environment::default();
        let bytes = seal(&config, Tool::Claude, &[7; 32]).unwrap();
        assert!(unseal(&bytes, Tool::Claude, &[7; 32]).is_ok());
        assert!(unseal(&bytes, Tool::Codex, &[7; 32]).is_err());
        assert!(unseal(&bytes, Tool::Claude, &[8; 32]).is_err());
        let mut stored: Encrypted = serde_json::from_slice(&bytes).unwrap();
        let mut ciphertext = B64.decode(&stored.ciphertext).unwrap();
        ciphertext[0] ^= 1;
        stored.ciphertext = B64.encode(ciphertext);
        assert!(unseal(
            &serde_json::to_vec(&stored).unwrap(),
            Tool::Claude,
            &[7; 32]
        )
        .is_err());
    }
    #[test]
    fn inherits_or_overrides_only_explicit_fields_and_keeps_values_literal() {
        let mut config = Environment::default();
        config.mode = Mode::Inherit;
        assert!(config.overrides().unwrap().is_empty());
        config.mode = Mode::Custom;
        config.timezone = "Europe/Paris".into();
        config.lc_all = String::new();
        config.variables = vec![
            Variable {
                name: "HTTPS_PROXY".into(),
                value: "http://name:p'a$()@localhost:8080".into(),
            },
            Variable {
                name: "OPENAI_API_KEY".into(),
                value: "test\nvalue=$()'\"".into(),
            },
        ];
        let values = config.overrides().unwrap();
        assert_eq!(values["TZ"], "Europe/Paris");
        assert!(!values.contains_key("LC_ALL"));
        assert_eq!(values["OPENAI_API_KEY"], config.variables[1].value);
        let exports = mac_exports(&values);
        assert!(exports.contains("'\\''"));
        assert!(!exports.contains("export LC_ALL"));
    }
    #[test]
    fn rejects_invalid_duplicate_and_reserved_names_without_returning_values() {
        for name in [
            "a;b",
            "a=b",
            "1KEY",
            "NODECLOAK_TERMINAL_ENVIRONMENT",
            "TZ",
            "lc_all",
        ] {
            let mut config = Environment::default();
            config.variables = vec![Variable {
                name: name.into(),
                value: "hidden-synthetic-secret".into(),
            }];
            let error = config.overrides().unwrap_err();
            assert!(!error.contains("hidden-synthetic-secret"));
        }
        let mut config = Environment::default();
        config.variables = vec![
            Variable {
                name: "Key".into(),
                value: "one".into(),
            },
            Variable {
                name: "KEY".into(),
                value: "two".into(),
            },
        ];
        assert!(config.overrides().is_err());
        config.variables.clear();
        config.timezone = "wrong/zone".into();
        assert!(config.overrides().is_err());
        config.timezone = "UTC".into();
        config.lang = "en; echo wrong".into();
        assert!(config.overrides().is_err());
    }
}
