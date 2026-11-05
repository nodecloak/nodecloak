//! Administrator-only, real Wintun / owned-helper checks. The executables are
//! synthetic fixtures, not proof that the Codex/ChatGPT/Claude products signed in.
use super::{
    ipc::{self, Reply},
    model::*,
    windows::{network, security, transport},
};
use std::{
    net::TcpListener,
    process::Command,
    sync::{
        atomic::Ordering,
        Arc,
    },
    time::Duration,
};
const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
use super::test_fixture::Fixture;
#[tokio::test]
async fn synthetic_socks_fixture_carries_a_complete_request_without_tun() {
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    let fixture = Fixture::start();
    let mut stream = tokio::net::TcpStream::connect(("127.0.0.1", fixture.port))
        .await
        .unwrap();
    stream.write_all(&[5, 1, 0]).await.unwrap();
    let mut hello = [0; 2];
    stream.read_exact(&mut hello).await.unwrap();
    assert_eq!(hello, [5, 0]);
    stream
        .write_all(&[5, 1, 0, 1, 198, 18, 0, 10, 0xa8, 0xca])
        .await
        .unwrap();
    let mut connected = [0; 10];
    stream.read_exact(&mut connected).await.unwrap();
    assert_eq!(&connected[..2], &[5, 0]);
    stream.write_all(b"nodecloak-probe").await.unwrap();
    let mut reply = [0u8; 9];
    stream.read_exact(&mut reply).await.unwrap();
    assert_eq!(&reply, b"via-proxy");
    assert_eq!(fixture.hits.load(Ordering::Acquire), 1);
}
fn request(paths: &[(String, String)], port: u16) -> ipc::StartRequest {
    ipc::StartRequest {
        session: uuid::Uuid::new_v4().to_string(),
        controller_port: TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port(),
        controller_secret: "a".repeat(64),
        rules: paths
            .iter()
            .map(|(name, path)| ResolvedRule {
                id: uuid::Uuid::new_v4().to_string(),
                name: name.clone(),
                paths: vec![path.clone()],
                proxy_id: ID.into(),
                exact_paths: true,
            })
            .collect(),
        proxies: vec![ResolvedProxy {
            id: ID.into(),
            config: crate::proxy::ProxyConfig {
                mode: "socks5".into(),
                host: "127.0.0.1".into(),
                port,
                ..Default::default()
            },
            password: String::new(),
        }],
    }
}
async fn begin(
    request: &ipc::StartRequest,
) -> (
    tokio::net::windows::named_pipe::NamedPipeServer,
    security::Handle,
    u64,
) {
    let (mut pipe, helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut pipe, request).await.unwrap();
    let reply: Reply = tokio::time::timeout(Duration::from_secs(75), ipc::read_frame(&mut pipe))
        .await
        .unwrap()
        .unwrap();
    match reply {
        Reply::Ready { interface, luid } => {
            assert_eq!(
                interface,
                super::config::interface_name(&request.session).unwrap()
            );
            (pipe, helper, luid)
        }
        Reply::Failure { message } => panic!("helper failed: {message}"),
        _ => panic!("helper not ready"),
    }
}
async fn exited(helper: security::Handle, name: &str, luid: u64) {
    assert!(
        tokio::task::spawn_blocking(move || helper.wait(15000))
            .await
            .unwrap(),
        "helper remains running"
    );
    for _ in 0..50 {
        if network::released(name, luid).unwrap() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    panic!("owned routes/interface not released");
}
fn stable_network() -> serde_json::Value {
    let state = network::snapshot().unwrap();
    let mut dns: Vec<_> = state
        .adapters
        .iter()
        .filter(|a| !a.name.starts_with("NCApps-"))
        .map(|a| format!("{}:{:?}", a.luid, a.dns))
        .collect();
    dns.sort();
    let mut routes: Vec<_> = state
        .routes
        .iter()
        .map(|r| format!("{}:{}", r.luid, r.prefix))
        .collect();
    routes.sort();
    let key = winreg::RegKey::predef(winreg::enums::HKEY_CURRENT_USER)
        .open_subkey(r"Software\Microsoft\Windows\CurrentVersion\Internet Settings")
        .unwrap();
    serde_json::json!({"dns":dns,"routes":routes,"proxyEnabled":key.get_value::<u32,_>("ProxyEnable").ok(),"proxyServer":key.get_value::<String,_>("ProxyServer").ok(),"pac":key.get_value::<String,_>("AutoConfigURL").ok()})
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Changes routes on a disposable elevated Windows runner; never run in a user's active desktop session"]
async fn native_application_proxy_routes_tcp_udp_and_releases_only_owned_network() {
    assert!(
        security::elevated(),
        "Run only on an elevated disposable Windows runner"
    );
    let root = tempfile::tempdir().unwrap();
    let base = root.path().join("probe.exe");
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/application_proxy_probe.rs");
    assert!(Command::new("rustc")
        .arg(source)
        .args(["--edition=2021", "-o"])
        .arg(&base)
        .status()
        .unwrap()
        .success());
    let mut paths = vec![];
    for (name, file) in [
        ("Codex", "ChatGPT.exe"),
        ("ChatGPT", "ChatGPT.exe"),
        ("Claude", "Claude.exe"),
        ("Unmatched", "ChatGPT.exe"),
    ] {
        let dir = root.path().join(name);
        std::fs::create_dir(&dir).unwrap();
        let exe = dir.join(file);
        std::fs::copy(&base, &exe).unwrap();
        paths.push((name.to_string(), exe.to_string_lossy().into_owned()));
    }
    let fixture = Fixture::start();
    let before = stable_network();
    let req = request(&paths[..3], fixture.port);
    let (mut pipe, helper, luid) = begin(&req).await;
    let name = super::config::interface_name(&req.session).unwrap();
    println!(
        "native rules: {}",
        serde_json::to_string(&req.rules).unwrap()
    );
    println!(
        "native routes: {}",
        serde_json::to_string(&network::snapshot().unwrap()).unwrap()
    );
    let logs = Arc::new(std::sync::Mutex::new(Vec::<String>::new()));
    let captured = logs.clone();
    let port = req.controller_port;
    let secret = req.controller_secret.clone();
    std::thread::spawn(move || {
        use tungstenite::client::IntoClientRequest;
        let mut request = format!("ws://127.0.0.1:{port}/logs?level=info")
            .into_client_request()
            .unwrap();
        request
            .headers_mut()
            .insert("Authorization", format!("Bearer {secret}").parse().unwrap());
        if let Ok((mut socket, _)) = tungstenite::connect(request) {
            for _ in 0..128 {
                match socket.read() {
                    Ok(message) => {
                        if let Ok(text) = message.into_text() {
                            captured.lock().unwrap().push(text.to_string());
                        }
                    }
                    Err(_) => break,
                }
            }
        }
    });
    for (app, path) in &paths[..3] {
        for mode in ["tcp", "udp"] {
            let path = path.clone();
            let probe =
                tokio::task::spawn_blocking(move || Command::new(path).arg(mode).output().unwrap());
            tokio::time::sleep(Duration::from_millis(700)).await;
            let connections = reqwest::Client::builder()
                .no_proxy()
                .build()
                .unwrap()
                .get(format!(
                    "http://127.0.0.1:{}/connections",
                    req.controller_port
                ))
                .bearer_auth(&req.controller_secret)
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            let output = probe.await.unwrap();
            assert!(
                output.status.success(),
                "{app} {mode}: {:?}; connections={connections}; logs={:?}",
                output,
                logs.lock().unwrap()
            );
            println!("synthetic {app} {mode}: assigned proxy observed");
        }
    }
    let count = fixture.hits.load(Ordering::Acquire);
    assert_eq!(count, 6);
    let other = paths[3].1.clone();
    let output =
        tokio::task::spawn_blocking(move || Command::new(other).arg("tcp").output().unwrap())
            .await
            .unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fixture.hits.load(Ordering::Acquire),
        count,
        "same basename from another path was incorrectly proxied"
    );
    ipc::write_frame(&mut pipe, &ipc::Control::Stop)
        .await
        .unwrap();
    drop(pipe);
    exited(helper, &name, luid).await;
    assert_eq!(
        stable_network(),
        before,
        "DNS/proxy/routes changed after stop"
    );
    // Owner EOF is independent of a Stop message and must release the owned Job.
    let req = request(&paths[..3], fixture.port);
    let (pipe, helper, luid) = begin(&req).await;
    let name = super::config::interface_name(&req.session).unwrap();
    drop(pipe);
    exited(helper, &name, luid).await;
    assert_eq!(stable_network(), before, "owner EOF left routing behind");
    // A dead selected proxy must fail; it must not switch to another outbound.
    let dead = TcpListener::bind("127.0.0.1:0").unwrap();
    let port = dead.local_addr().unwrap().port();
    drop(dead);
    let req = request(&paths[..3], port);
    let (mut pipe, helper, luid) = begin(&req).await;
    let name = super::config::interface_name(&req.session).unwrap();
    let path = paths[0].1.clone();
    let output =
        tokio::task::spawn_blocking(move || Command::new(path).arg("tcp").output().unwrap())
            .await
            .unwrap();
    assert!(!output.status.success());
    ipc::write_frame(&mut pipe, &ipc::Control::Stop)
        .await
        .unwrap();
    drop(pipe);
    exited(helper, &name, luid).await;
    assert_eq!(stable_network(), before);
    println!("normal stop, owner EOF, dead proxy: network restored; real applications still require separate traffic validation");
    // Cancellation while preparing a privileged core must not wait for Ready.
    let req = request(&paths[..3], fixture.port);
    let (mut pipe, helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut pipe, &req).await.unwrap();
    drop(pipe);
    assert!(tokio::task::spawn_blocking(move || helper.wait(15000))
        .await
        .unwrap());
    assert_eq!(
        stable_network(),
        before,
        "cancelled startup left routing behind"
    );
    // Occupied diagnostic ports cannot impersonate an owned core's readiness.
    let occupied = TcpListener::bind("127.0.0.1:0").unwrap();
    let mut req = request(&paths[..3], fixture.port);
    req.controller_port = occupied.local_addr().unwrap().port();
    let (mut pipe, helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut pipe, &req).await.unwrap();
    let reply: Reply = tokio::time::timeout(Duration::from_secs(75), ipc::read_frame(&mut pipe))
        .await
        .unwrap()
        .unwrap();
    assert!(
        matches!(reply, Reply::Failure { .. }),
        "occupied controller accepted as ready"
    );
    drop(pipe);
    assert!(tokio::task::spawn_blocking(move || helper.wait(15000))
        .await
        .unwrap());
    assert_eq!(
        stable_network(),
        before,
        "failed startup left routing behind"
    );
}

#[test]
#[ignore = "Read-only discovery on the current Windows desktop; prints no credentials"]
fn installed_application_inventory_distinguishes_products() {
    for app in super::discovery::discover().unwrap() {
        println!(
            "{}: {} executable paths, {} running processes",
            app.name,
            app.paths.len(),
            app.running
        );
        for path in app.paths {
            println!("  {path}");
        }
    }
}
