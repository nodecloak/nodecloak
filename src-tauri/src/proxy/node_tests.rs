use super::*;
use serde_json::json;

#[test]
fn node_proxy_validation_rejects_missing_options_and_incompatible_sources() {
    let mut config: ProxyConfig =
        serde_json::from_value(json!({"mode":"vless","host":"node.invalid","port":443})).unwrap();
    assert!(config.validate().is_err());
    let options =
        json!({"tls":"tls","serverName":"node.invalid","transport":"ws","path":"/socket"});
    config = serde_json::from_value(
        json!({"mode":"vless","host":"node.invalid","port":443,"node":options}),
    )
    .unwrap();
    assert!(config.validate().is_ok());
    let mut value = serde_json::to_value(&config).unwrap();
    value["apiUrl"] = json!("https://provider.invalid");
    assert!(serde_json::from_value::<ProxyConfig>(value)
        .unwrap()
        .validate()
        .is_err());
    for patch in [
        json!({"transport":"xhttp"}),
        json!({"tls":"reality"}),
        json!({"flow":"unrecognized"}),
        json!({"tls":"none","serverName":"node.invalid"}),
    ] {
        let mut value = serde_json::to_value(&config).unwrap();
        for (key, value2) in patch.as_object().unwrap() {
            value["node"][key] = value2.clone();
        }
        assert!(serde_json::from_value::<ProxyConfig>(value)
            .unwrap()
            .validate()
            .is_err());
    }
    let invalid = Upstream {
        config: config.clone(),
        password: "not-a-uuid".into(),
    };
    assert!(invalid.validate().is_err());
    let valid = Upstream {
        config,
        password: "11111111-1111-4111-8111-111111111111".into(),
    };
    assert!(valid.validate().is_ok());
}

#[test]
fn node_options_require_schema6_and_never_enter_profile_credentials() {
    let source = json!({"schema":6,"name":"Node fixture","browser":"chrome","notes":"","tags":[],"proxy":{"mode":"vmess","host":"node.invalid","port":443,"username":"","password":"11111111-1111-4111-8111-111111111111","node":{}},"preferences":crate::profiles::Preferences::default(),"deleted":false});
    let doc = crate::cloud::model::Document::parse(source.clone()).unwrap();
    let draft = doc.draft();
    assert_eq!(
        draft.password.as_deref(),
        Some("11111111-1111-4111-8111-111111111111")
    );
    let public = serde_json::to_string(&draft.proxy).unwrap();
    assert!(!public.contains("11111111"));
    assert!(draft.proxy.has_secret());
    let mut old = source;
    old["schema"] = json!(5);
    assert!(crate::cloud::model::Document::parse(old).is_err());
}
