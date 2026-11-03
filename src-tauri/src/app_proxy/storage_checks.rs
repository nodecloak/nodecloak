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
        clear_password: false,
    }
}
#[test]
fn destinations_persist_without_passwords_and_rules_stay_disabled() {
    let root = tempfile::tempdir().unwrap();
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
    let root = tempfile::tempdir().unwrap();
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
    let root = tempfile::tempdir().unwrap();
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
