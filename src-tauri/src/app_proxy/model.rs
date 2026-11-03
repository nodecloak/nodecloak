use crate::proxy::ProxyConfig;
use serde::{Deserialize, Serialize};

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Rule {
    pub id: String,
    pub app: String,
    pub executable: String,
    pub proxy_id: Option<String>,
    pub enabled: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Destination {
    pub id: String,
    pub name: String,
    #[serde(deserialize_with = "strict_proxy")]
    pub proxy: ProxyConfig,
    pub has_password: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Settings {
    pub version: u32,
    pub proxies: Vec<Destination>,
    pub rules: Vec<Rule>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedRule {
    pub id: String,
    pub name: String,
    pub paths: Vec<String>,
    pub proxy_id: String,
    #[serde(default)]
    pub exact_paths: bool,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ResolvedProxy {
    pub id: String,
    #[serde(deserialize_with = "strict_proxy")]
    pub config: ProxyConfig,
    pub password: String,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            version: 1,
            proxies: vec![],
            rules: [
                ("codex", "d75f9dd8-9a66-4e26-b21c-4ce4e8f1863d"),
                ("chatgpt", "234c53ed-e319-4698-9897-aae7e056c2f2"),
                ("claude", "e9f6d818-6bd8-4de0-966b-9f3f92a44db3"),
            ]
            .into_iter()
            .map(|(app, id)| Rule {
                id: id.into(),
                app: app.into(),
                executable: String::new(),
                proxy_id: None,
                enabled: false,
            })
            .collect(),
        }
    }
}
#[derive(Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct DestinationDraft {
    pub id: Option<String>,
    pub name: String,
    #[serde(deserialize_with = "strict_proxy")]
    pub proxy: ProxyConfig,
    pub password: Option<String>,
    #[serde(default)]
    pub clear_password: bool,
}

fn strict_proxy<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<ProxyConfig, D::Error> {
    let value = serde_json::Value::deserialize(deserializer)?;
    let allowed = [
        "mode",
        "host",
        "port",
        "username",
        "credentialRef",
        "preProxy",
        "apiUrl",
        "node",
    ];
    let valid = value
        .as_object()
        .is_some_and(|object| object.keys().all(|k| allowed.contains(&k.as_str())))
        && value.get("preProxy").is_none_or(|v| {
            v.is_null()
                || v.as_object().is_some_and(|o| {
                    o.keys()
                        .all(|k| ["mode", "host", "port"].contains(&k.as_str()))
                })
        });
    if !valid {
        return Err(serde::de::Error::custom("应用代理参数包含不支持的字段"));
    }
    serde_json::from_value(value).map_err(serde::de::Error::custom)
}
