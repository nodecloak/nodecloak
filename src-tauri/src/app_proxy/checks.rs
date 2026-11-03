use super::{config, discovery, model::*};
use crate::proxy::ProxyConfig;
use serde_json::json;

const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
fn proxy() -> ResolvedProxy {
    ResolvedProxy {
        id: ID.into(),
        config: ProxyConfig {
            mode: "socks5".into(),
            host: "127.0.0.1".into(),
            port: 1080,
            username: "fixture".into(),
            ..Default::default()
        },
        password: "fixture-password".into(),
    }
}
fn rule(path: &str) -> ResolvedRule {
    ResolvedRule {
        id: "0de524a5-6bf0-4f92-82d4-1aad86517b68".into(),
        name: "Codex".into(),
        paths: vec![path.into()],
        proxy_id: ID.into(),
        exact_paths: false,
    }
}
#[test]
fn same_executable_name_does_not_conflate_codex_with_chatgpt() {
    assert_eq!(
        discovery::identify(
            r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        ),
        Some("codex")
    );
    assert_eq!(
        discovery::identify(
            r"C:\Program Files\WindowsApps\OpenAI.ChatGPT-Desktop_1.2.3.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe"
        ),
        Some("chatgpt")
    );
    assert_eq!(
        discovery::identify(r"C:\Users\test\AppData\Local\OpenAI\Codex\bin\aabb\codex.exe"),
        Some("codex")
    );
    assert_eq!(discovery::identify(r"C:\Downloads\ChatGPT.exe"), None);
    assert_eq!(discovery::identify(r"C:\Unrelated\codex.exe"), None);
    assert_eq!(
        discovery::identify(
            r"C:\Downloads\WindowsApps\OpenAI.Codex_1_x64__2p2nqsd0c76g0\ChatGPT.exe"
        ),
        None
    );
}

#[test]
fn windows_path_case_and_extended_prefix_cannot_silently_fall_through_to_direct() {
    for path in [r"C:\Apps\Codex.exe", r"\\?\c:\APPS\CODEX.exe"] {
        let v = config::compile(&[rule(path)], &[proxy()], 51234, &"a".repeat(64), ID).unwrap();
        let expected = json!([r"(?i)^c:\\apps\\codex\.exe$"]);
        assert!(
            v["route"]["rules"]
                .as_array()
                .unwrap()
                .iter()
                .any(|r| r["process_path_regex"] == expected
                    && r["outbound"] == format!("proxy-{ID}")),
            "case-sensitive or overbroad path rule: {path}"
        );
    }
}
#[test]
fn package_updates_keep_same_publisher_and_app_while_other_products_stay_out() {
    for path in [
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
        r"C:\Program Files\WindowsApps\OpenAI.Codex_27.1.1.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
    ] {
        assert_eq!(
            config::path_pattern(path),
            r"(?i)^c:\\program files\\windowsapps\\openai\.codex_[0-9]+\.[0-9]+\.[0-9]+\.[0-9]+_x64__2p2nqsd0c76g0\\app\\chatgpt\.exe$"
        );
    }
    assert_eq!(
        config::path_pattern(r"C:\Users\test\AppData\Local\OpenAI\Codex\bin\aabbccdd\codex.exe"),
        r"(?i)^c:\\users\\test\\appdata\\local\\openai\\codex\\bin\\[0-9a-f]{4,64}\\codex\.exe$"
    );
}
#[test]
fn helper_proxy_dto_rejects_nested_arbitrary_options() {
    let value = json!({"id":ID,"password":"fixture","config":{"mode":"http","host":"127.0.0.1","port":1080,"username":"","experimental":{"clash_api":{"external_controller":"0.0.0.0:9090"}}}});
    assert!(serde_json::from_value::<ResolvedProxy>(value).is_err());
}
#[test]
fn overlapping_package_version_patterns_cannot_choose_different_nodes() {
    let mut second = rule(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_27.1.1.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
    );
    second.id = "25a62d5a-dde0-4932-a008-5277d7343efa".into();
    let first = rule(
        r"C:\Program Files\WindowsApps\OpenAI.Codex_26.1027.3384.0_x64__2p2nqsd0c76g0\app\ChatGPT.exe",
    );
    assert!(config::compile(&[first, second], &[proxy()], 51234, &"a".repeat(64), ID).is_err());
}
#[test]
fn app_tun_routes_only_assigned_paths_and_preserves_system_dns_and_browser_relays() {
    let v = config::compile(
        &[rule(r"C:\Apps\Codex.exe")],
        &[proxy()],
        51234,
        &"a".repeat(64),
        ID,
    )
    .unwrap();
    assert_eq!(v["route"]["final"], "direct");
    assert_eq!(v["inbounds"][0]["type"], "tun");
    assert_eq!(v["inbounds"][0]["auto_route"], true);
    assert_eq!(v["inbounds"][0]["dns_mode"], "disabled");
    assert_eq!(v["route"]["auto_detect_interface"], true);
    let rules = v["route"]["rules"].as_array().unwrap();
    assert!(rules.iter().all(|r| r.get("process_name").is_none()));
    assert_eq!(rules[0]["outbound"], "direct");
    let app = rules
        .iter()
        .find(|r| r["outbound"] == format!("proxy-{ID}"))
        .unwrap();
    assert_eq!(
        app["process_path_regex"],
        json!([r"(?i)^c:\\apps\\codex\.exe$"])
    );
    assert_eq!(app["outbound"], format!("proxy-{ID}"));
    assert_eq!(
        v["experimental"]["clash_api"]["external_controller"],
        "127.0.0.1:51234"
    );
    assert!(!v.to_string().contains("set_system_proxy"));
    assert_eq!(v["dns"]["servers"][0]["type"], "local");
}
#[test]
fn no_rules_unsafe_paths_and_conflicting_assignments_cannot_start_routing() {
    let compile = |r: &[ResolvedRule]| config::compile(r, &[proxy()], 51234, &"a".repeat(64), ID);
    assert!(compile(&[]).is_err());
    for path in [
        "codex.exe",
        r"C:\Apps\..\Other.exe",
        r"C:\Apps\test.exe:evil",
        r"\\server\share\app.exe",
        r"C:\Apps\nodecloak.exe",
    ] {
        assert!(compile(&[rule(path)]).is_err(), "unsafe target {path}");
    }
    let first = rule(r"C:\Apps\Codex.exe");
    let mut second = rule(r"c:/apps/CODEX.exe");
    second.id = "25a62d5a-dde0-4932-a008-5277d7343efa".into();
    assert!(compile(&[first, second]).is_err());
}
#[test]
fn proxy_failure_has_no_direct_fallback_or_selector() {
    let v = config::compile(
        &[rule(r"C:\Apps\Codex.exe")],
        &[proxy()],
        51234,
        &"a".repeat(64),
        ID,
    )
    .unwrap();
    let out = v["outbounds"].as_array().unwrap();
    assert!(out
        .iter()
        .all(|v| !matches!(v["type"].as_str(), Some("selector" | "urltest"))));
    let assigned = out
        .iter()
        .find(|v| v["tag"] == format!("proxy-{ID}"))
        .unwrap();
    assert_eq!(assigned["type"], "socks");
    assert_eq!(assigned["password"], "fixture-password");
}

#[test]
#[ignore = "Checks generated configs with the pinned core; never starts a TUN"]
fn installed_core_accepts_application_configs_without_network_changes() {
    use std::{
        io::Write,
        process::{Command, Stdio},
    };
    let binary = std::env::var_os("NODECLOAK_TEST_SINGBOX").expect("Set pinned core path");
    for mode in ["http", "https", "socks5", "vless", "vmess", "hysteria2"] {
        let mut p = proxy();
        p.config.mode = mode.into();
        p.config.username = String::new();
        if crate::proxy::node::is_node(mode) {
            p.config.node = Some(crate::proxy::node::NodeOptions {
                tls: if mode == "hysteria2" { "tls" } else { "none" }.into(),
                ..Default::default()
            });
            p.password = if mode == "hysteria2" {
                "synthetic-secret"
            } else {
                "0b4fcb93-ec83-4f9b-996a-f89f7aa96025"
            }
            .into();
        }
        let config = config::compile(
            &[rule(r"C:\Apps\Codex.exe")],
            &[p],
            51234,
            &"a".repeat(64),
            ID,
        )
        .unwrap();
        let mut child = Command::new(&binary)
            .args(["check", "-c", "stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&config).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{mode}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        println!("{mode}: typed TUN configuration accepted by pinned core");
    }
}
