//! Administrator-only, real Wintun / owned-helper checks. The executables are
//! synthetic fixtures, not proof that the Codex/ChatGPT/Claude products signed in.
use super::{
    ipc::{self, Reply},
    model::*,
    windows::{network, security, transport},
};
use std::{
    io::{Read, Write},
    net::{TcpListener, UdpSocket},
    process::Command,
    sync::{
        atomic::{AtomicBool, AtomicUsize, Ordering},
        Arc,
    },
    time::Duration,
};
const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
struct Fixture {
    port: u16,
    hits: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
}
impl Fixture {
    fn start() -> Self {
        let tcp = TcpListener::bind("127.0.0.1:0").unwrap();
        let port = tcp.local_addr().unwrap().port();
        tcp.set_nonblocking(true).unwrap();
        let udp = UdpSocket::bind("127.0.0.1:0").unwrap();
        let udp_port = udp.local_addr().unwrap().port();
        udp.set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();
        let hits = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let th = hits.clone();
        let ts = stop.clone();
        std::thread::spawn(move || {
            while !ts.load(Ordering::Acquire) {
                match tcp.accept() {
                    Ok((mut stream, _)) => {
                        let hits = th.clone();
                        std::thread::spawn(move || {
                            stream
                                .set_read_timeout(Some(Duration::from_secs(5)))
                                .unwrap();
                            let mut greeting = [0; 2];
                            if stream.read_exact(&mut greeting).is_err() {
                                return;
                            }
                            let mut methods = vec![0; greeting[1] as usize];
                            if stream.read_exact(&mut methods).is_err() {
                                return;
                            }
                            if stream.write_all(&[5, 0]).is_err() {
                                return;
                            }
                            let mut request = [0; 4];
                            if stream.read_exact(&mut request).is_err() {
                                return;
                            }
                            let count = match request[3] {
                                1 => 4,
                                4 => 16,
                                3 => {
                                    let mut n = [0];
                                    if stream.read_exact(&mut n).is_err() {
                                        return;
                                    }
                                    n[0] as usize
                                }
                                _ => return,
                            };
                            let mut target = vec![0; count + 2];
                            if stream.read_exact(&mut target).is_err() {
                                return;
                            }
                            if request[1] == 3 {
                                let mut reply = vec![5, 0, 0, 1, 127, 0, 0, 1];
                                reply.extend(udp_port.to_be_bytes());
                                let _ = stream.write_all(&reply);
                                let mut byte = [0];
                                let _ = stream.read(&mut byte);
                            } else if request[1] == 1 {
                                let _ = stream.write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0]);
                                let mut bytes = [0; 128];
                                if stream
                                    .read(&mut bytes)
                                    .is_ok_and(|n| &bytes[..n] == b"nodecloak-probe")
                                {
                                    hits.fetch_add(1, Ordering::AcqRel);
                                    let _ = stream.write_all(b"via-proxy");
                                    std::thread::sleep(Duration::from_millis(250));
                                }
                            }
                        });
                    }
                    Err(_) => std::thread::sleep(Duration::from_millis(20)),
                }
            }
        });
        let uh = hits.clone();
        let us = stop.clone();
        std::thread::spawn(move || {
            let mut bytes = [0; 2048];
            while !us.load(Ordering::Acquire) {
                if let Ok((n, peer)) = udp.recv_from(&mut bytes) {
                    let header = match bytes.get(3) {
                        Some(1) => 10,
                        Some(4) => 22,
                        Some(3) => 7 + bytes[4] as usize,
                        _ => continue,
                    };
                    if n >= header && &bytes[header..n] == b"nodecloak-probe" {
                        uh.fetch_add(1, Ordering::AcqRel);
                        let mut reply = bytes[..header].to_vec();
                        reply.extend(b"via-proxy");
                        let _ = udp.send_to(&reply, peer);
                    }
                }
            }
        });
        Self { port, hits, stop }
    }
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
    }
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
    for (app, path) in &paths[..3] {
        for mode in ["tcp", "udp"] {
            let path = path.clone();
            let output =
                tokio::task::spawn_blocking(move || Command::new(path).arg(mode).output().unwrap())
                    .await
                    .unwrap();
            assert!(output.status.success(), "{app} {mode}: {:?}", output);
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
}
