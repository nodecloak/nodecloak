use super::*;
use serde_json::json;

#[test]
fn hysteria2_uses_opaque_userpass_and_mandatory_tls_without_uuid() {
    let config: ProxyConfig = serde_json::from_value(json!({"mode":"hysteria2","host":"192.0.2.10","port":39114,"node":{"tls":"tls","serverName":"example.com"}})).unwrap();
    let up = Upstream { config, password:"test-user:Secret:AbC".into() };
    let out = node::outbound(&up).unwrap();
    assert_eq!(out["type"],"hysteria2");assert_eq!(out["password"],"test-user:Secret:AbC");
    assert!(out.get("uuid").is_none());assert_eq!(out["tls"]["server_name"],"example.com");
    assert!(out["tls"].get("insecure").is_none());assert!(up.config.has_secret());
    let mut value=serde_json::to_value(&up.config).unwrap();value["node"]["insecure"]=json!(true);
    let mut up=Upstream {config:serde_json::from_value(value).unwrap(),password:up.password};
    assert_eq!(node::outbound(&up).unwrap()["tls"]["insecure"],true);
    for mode in ["http","https"] {
        up.config.pre_proxy=Some(LocalProxy{mode:mode.into(),host:"127.0.0.1".into(),port:7890});
        assert!(up.validate().unwrap_err().contains("UDP"));
    }
    up.config.pre_proxy=Some(LocalProxy{mode:"socks5".into(),host:"127.0.0.1".into(),port:7890});
    assert_eq!(node::outbound(&up).unwrap()["detour"],"first-hop");
    up.password.clear();assert!(up.validate().is_err());
    for patch in [json!({"tls":"none"}),json!({"transport":"ws"}),json!({"fingerprint":"chrome"}),json!({"alterId":1})] {
        let mut value=serde_json::to_value(&up.config).unwrap();
        for (key,item) in patch.as_object().unwrap(){value["node"][key]=item.clone();}
        assert!(serde_json::from_value::<ProxyConfig>(value).unwrap().validate().is_err());
    }
}

#[test]
fn hysteria2_cloud_document_preserves_auth_tls_and_node_schema() {
    let source=json!({"schema":6,"name":"HY2 fixture","browser":"edge","notes":"","tags":[],"proxy":{"mode":"hysteria2","host":"node.invalid","port":443,"username":"","password":"test-user:CaseSensitive","node":{"tls":"tls","insecure":true,"serverName":"example.com"}},"preferences":crate::profiles::Preferences::default(),"deleted":false});
    let draft=crate::cloud::model::Document::parse(source.clone()).unwrap().draft();
    assert_eq!(draft.password.as_deref(),Some("test-user:CaseSensitive"));
    assert!(draft.proxy.has_secret());assert!(draft.proxy.node.as_ref().unwrap().insecure);
    assert!(!serde_json::to_string(&draft.proxy).unwrap().contains("CaseSensitive"));
    let mut old=source;old["schema"]=json!(5);assert!(crate::cloud::model::Document::parse(old).is_err());
}

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
