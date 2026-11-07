use super::{
    model::*,
    store::{self, Credentials},
};
use crate::proxy::ProxyConfig;
use std::{cell::RefCell, collections::BTreeMap};
#[derive(Default)]
struct Vault(RefCell<BTreeMap<String, String>>);
impl Credentials for Vault {
    fn get(&self, id: &str, reference: &str) -> Result<String, String> {
        self.0
            .borrow()
            .get(&format!("{id}/{reference}"))
            .cloned()
            .ok_or("missing".into())
    }
    fn set(&self, id: &str, reference: &str, value: &str) -> Result<(), String> {
        self.0
            .borrow_mut()
            .insert(format!("{id}/{reference}"), value.into());
        Ok(())
    }
    fn remove(&self, id: &str, reference: &str) {
        self.0.borrow_mut().remove(&format!("{id}/{reference}"));
    }
}
fn draft() -> DestinationDraft {
    DestinationDraft {
        id: None,
        name: "Test proxy".into(),
        proxy: ProxyConfig {
            mode: "http".into(),
            host: "127.0.0.1".into(),
            port: 8080,
            username: "fixture".into(),
            ..Default::default()
        },
        password: Some("private-fixture-password".into()),
        pre_proxy_enabled: None,
        clear_password: false,
    }
}
#[test]
fn destinations_persist_without_passwords_and_rules_stay_disabled() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let saved = store::save_node(root.path(), draft(), &vault).unwrap();
    assert_eq!(saved.rules.len(), 3);
    assert!(saved.rules.iter().all(|r| !r.enabled));
    let bytes = std::fs::read_to_string(root.path().join("application-proxy.json")).unwrap();
    assert!(!bytes.contains("private-fixture-password"));
    assert!(bytes.contains("credentialRef"));
    let loaded = store::load(root.path()).unwrap();
    let node = &loaded.proxies[0];
    assert!(node.has_password);
    assert_eq!(
        vault
            .get(&node.id, node.proxy.credential_ref.as_deref().unwrap())
            .unwrap(),
        "private-fixture-password"
    );
}
#[test]
fn changing_destination_cannot_send_previous_credentials_to_new_endpoint() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let first = store::save_node(root.path(), draft(), &vault).unwrap();
    let mut change = draft();
    change.id = Some(first.proxies[0].id.clone());
    change.proxy.host = "new.example".into();
    change.password = None;
    let saved = store::save_node(root.path(), change, &vault).unwrap();
    assert!(!saved.proxies[0].has_password);
    assert!(saved.proxies[0].proxy.credential_ref.is_none());
    assert!(vault.0.borrow().is_empty());
}
#[test]
fn name_only_edit_keeps_secret_but_clear_removes_it() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let first = store::save_node(root.path(), draft(), &vault).unwrap();
    let mut change = draft();
    change.id = Some(first.proxies[0].id.clone());
    change.name = "Renamed".into();
    change.password = None;
    let saved = store::save_node(root.path(), change, &vault).unwrap();
    assert!(saved.proxies[0].has_password);
    let mut clear = draft();
    clear.id = Some(first.proxies[0].id.clone());
    clear.password = None;
    clear.clear_password = true;
    let saved = store::save_node(root.path(), clear, &vault).unwrap();
    assert!(!saved.proxies[0].has_password);
    assert!(vault.0.borrow().is_empty());
}

#[test]
fn first_hop_toggle_preserves_saved_endpoint_and_credentials_across_reloads() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let mut initial = draft();
    initial.proxy.pre_proxy = Some(crate::proxy::LocalProxy {
        mode: "socks5".into(),
        host: "127.0.0.1".into(),
        port: 1080,
    });
    let saved = store::save_node(root.path(), initial, &vault).unwrap();
    let id = saved.proxies[0].id.clone();
    let reference = saved.proxies[0].proxy.credential_ref.clone();
    for enabled in [false, true] {
        let current = store::load(root.path()).unwrap();
        let change: DestinationDraft = serde_json::from_value(serde_json::json!({
            "id": id, "name": "Test proxy", "proxy": current.proxies[0].proxy,
            "preProxyEnabled": enabled,
        }))
        .unwrap();
        store::save_node(root.path(), change, &vault).unwrap();
        let mut loaded = store::load(root.path()).unwrap();
        let node = &loaded.proxies[0];
        assert_eq!(node.proxy.pre_proxy.as_ref().unwrap().port, 1080);
        assert_eq!(node.proxy.credential_ref, reference);
        assert!(node.has_password);
        loaded.rules[0].enabled = true;
        loaded.rules[0].proxy_id = Some(id.clone());
        let resolved = store::resolved_proxies(&loaded, &vault).unwrap();
        assert_eq!(resolved[0].config.pre_proxy.is_some(), enabled);
        assert_eq!(resolved[0].password, "private-fixture-password");
        for (platform, path) in [
            (super::config::Platform::Windows, r"C:\Apps\Codex.exe"),
            (
                super::config::Platform::Macos,
                "/Applications/Codex.app/Contents/MacOS/Codex",
            ),
        ] {
            let rules = [ResolvedRule {
                id: loaded.rules[0].id.clone(),
                name: "Codex".into(),
                paths: vec![path.into()],
                proxy_id: id.clone(),
                exact_paths: true,
            }];
            let compiled = super::config::compile_for(
                platform,
                &rules,
                &resolved,
                51234,
                &"a".repeat(64),
                &id,
            )
            .unwrap();
            let outbounds = compiled["outbounds"].as_array().unwrap();
            assert_eq!(
                outbounds.iter().any(|out| out.get("detour").is_some()),
                enabled
            );
            assert_eq!(
                outbounds.iter().any(|out| out["server_port"] == 1080),
                enabled
            );
        }
        // Old clients that omit the new flag must not re-enable a saved, disabled hop.
        let mut rename = draft();
        rename.id = Some(id.clone());
        rename.password = None;
        rename.proxy = loaded.proxies[0].proxy.clone();
        let renamed = store::save_node(root.path(), rename, &vault).unwrap();
        loaded.proxies = renamed.proxies;
        assert_eq!(
            store::resolved_proxies(&loaded, &vault).unwrap()[0]
                .config
                .pre_proxy
                .is_some(),
            enabled
        );
    }
}

#[test]
fn legacy_first_hop_stays_enabled_and_enabled_empty_hop_is_rejected() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let mut initial = draft();
    initial.proxy.pre_proxy = Some(crate::proxy::LocalProxy {
        mode: "http".into(),
        host: "192.168.10.1".into(),
        port: 7890,
    });
    let saved = store::save_node(root.path(), initial, &vault).unwrap();
    let mut legacy = serde_json::to_value(saved).unwrap();
    legacy["proxies"][0]
        .as_object_mut()
        .unwrap()
        .remove("preProxyEnabled");
    std::fs::write(
        root.path().join("application-proxy.json"),
        serde_json::to_vec(&legacy).unwrap(),
    )
    .unwrap();
    let loaded = store::load(root.path()).unwrap();
    assert!(loaded.proxies[0].connection_config().pre_proxy.is_some());
    let mut empty = draft();
    empty.pre_proxy_enabled = Some(true);
    assert_eq!(
        store::save_node(root.path(), empty, &vault).err().unwrap(),
        "请先填写前置代理地址"
    );
    assert_eq!(store::load(root.path()).unwrap().proxies.len(), 1);
}

#[tokio::test]
async fn disabled_first_hop_never_receives_detection_traffic() {
    use tokio::{
        io::{AsyncReadExt, AsyncWriteExt},
        net::TcpListener,
        time::{timeout, Duration},
    };
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let destination = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let first_hop = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let mut initial = draft();
    initial.proxy.port = destination.local_addr().unwrap().port();
    initial.proxy.pre_proxy = Some(crate::proxy::LocalProxy {
        mode: "http".into(),
        host: "127.0.0.1".into(),
        port: first_hop.local_addr().unwrap().port(),
    });
    initial.pre_proxy_enabled = Some(false);
    let saved = store::save_node(root.path(), initial, &vault).unwrap();
    let node = &saved.proxies[0];
    let password = vault
        .get(&node.id, node.proxy.credential_ref.as_deref().unwrap())
        .unwrap();
    let server = tokio::spawn(async move {
        let (mut socket, _) = destination.accept().await.unwrap();
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            header.push(socket.read_u8().await.unwrap());
        }
        let header = String::from_utf8(header).unwrap();
        assert!(header.starts_with("GET http://target.invalid/check HTTP/1.1"));
        use base64::Engine;
        let auth =
            base64::engine::general_purpose::STANDARD.encode("fixture:private-fixture-password");
        assert!(header.to_ascii_lowercase().contains(&format!(
            "proxy-authorization: basic {}",
            auth.to_ascii_lowercase()
        )));
        socket
            .write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok")
            .await
            .unwrap();
    });
    // This is the same effective configuration used by the testNode command.
    let (client, _relay) = crate::proxy::http_client(crate::proxy::Upstream {
        config: node.connection_config(),
        password,
    })
    .await
    .unwrap();
    let response = timeout(
        Duration::from_secs(3),
        client.get("http://target.invalid/check").send(),
    )
    .await
    .unwrap()
    .unwrap();
    assert_eq!(response.text().await.unwrap(), "ok");
    server.await.unwrap();
    assert!(timeout(Duration::from_millis(50), first_hop.accept())
        .await
        .is_err());
}

#[test]
fn disabled_cached_http_hop_does_not_restrict_hy2_but_cannot_be_enabled() {
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let vault = Vault::default();
    let mut initial = draft();
    initial.proxy.mode = "hysteria2".into();
    initial.proxy.username.clear();
    initial.proxy.node = Some(crate::proxy::node::NodeOptions {
        tls: "tls".into(),
        ..Default::default()
    });
    initial.proxy.pre_proxy = Some(crate::proxy::LocalProxy {
        mode: "http".into(),
        host: "192.168.10.1".into(),
        port: 7890,
    });
    initial.pre_proxy_enabled = Some(false);
    let saved = store::save_node(root.path(), initial, &vault).unwrap();
    let loaded = store::load(root.path()).unwrap();
    assert_eq!(
        loaded.proxies[0].proxy.pre_proxy.as_ref().unwrap().mode,
        "http"
    );
    let enable: DestinationDraft = serde_json::from_value(serde_json::json!({
        "id": saved.proxies[0].id, "name": "HY2", "proxy": loaded.proxies[0].proxy,
        "preProxyEnabled": true,
    }))
    .unwrap();
    assert!(store::save_node(root.path(), enable, &vault)
        .err()
        .unwrap()
        .contains("SOCKS5"));
    assert!(store::load(root.path()).unwrap().proxies[0]
        .connection_config()
        .pre_proxy
        .is_none());
}
