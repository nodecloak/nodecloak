//! Disposable-host checks: these programs simulate traffic, not product sign-in.
use super::{network, transport};
use crate::app_proxy::{
    ipc::{self, Reply, StartRequest},
    model::{ResolvedProxy, ResolvedRule},
    test_fixture::Fixture,
};
use std::{process::Command, sync::atomic::Ordering, time::Duration};
const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
#[tokio::test]
#[ignore = "Entry point only for the authenticated root launchd fixture"]
async fn root_entry() {
    assert_eq!(unsafe { libc::geteuid() }, 0);
    let suffix = std::env::var("NC_AP_TEST_SUFFIX").unwrap();
    let parent = std::env::var("NC_AP_TEST_PARENT").unwrap().parse().unwrap();
    crate::app_proxy::helper::run(&suffix, parent)
        .await
        .unwrap();
}
struct Session {
    pipe: transport::Server,
    helper: transport::Handle,
    index: u64,
    interface: String,
}
impl Session {
    async fn begin(request: &StartRequest) -> Self {
        let (mut pipe, helper) = transport::connect_helper().await.unwrap();
        ipc::write_frame(&mut pipe, request).await.unwrap();
        match tokio::time::timeout(
            Duration::from_secs(75),
            ipc::read_frame::<_, Reply>(&mut pipe),
        )
        .await
        .unwrap()
        .unwrap()
        {
            Reply::Ready { interface, luid } => Self {
                pipe,
                helper,
                index: luid,
                interface,
            },
            Reply::Failure { message } => panic!("root helper: {message}"),
            _ => panic!("unexpected root reply"),
        }
    }
    async fn released(&self) {
        assert!(self.helper.wait(15000), "launcher/job did not exit");
        for _ in 0..50 {
            if network::released(&self.interface, self.index).unwrap()
                && !Command::new("/usr/bin/sudo")
                    .args([
                        "-n",
                        "/bin/launchctl",
                        "print",
                        &format!("system/{}", self.helper.label),
                    ])
                    .stdout(std::process::Stdio::null())
                    .stderr(std::process::Stdio::null())
                    .status()
                    .unwrap()
                    .success()
            {
                return;
            }
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        panic!("utun/routes survived teardown")
    }
    async fn stop(&mut self) {
        ipc::write_frame(&mut self.pipe, &ipc::Control::Stop)
            .await
            .unwrap();
        self.helper.close_control();
        self.released().await;
    }
}
impl Drop for Session {
    fn drop(&mut self) {
        self.helper.close_control();
        self.helper.wait(15000);
    }
}
fn request(paths: &[(String, String)], port: u16) -> StartRequest {
    StartRequest {
        session: uuid::Uuid::new_v4().to_string(),
        controller_port: std::net::TcpListener::bind("127.0.0.1:0")
            .unwrap()
            .local_addr()
            .unwrap()
            .port(),
        controller_secret: format!(
            "{}{}",
            uuid::Uuid::new_v4().simple(),
            uuid::Uuid::new_v4().simple()
        ),
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
fn stable() -> serde_json::Value {
    let state = network::snapshot().unwrap();
    let mut routes: Vec<_> = state
        .routes
        .iter()
        .filter(|r| {
            !r.flags.contains('W')
                && !r.flags.contains('L')
                && !r.destination.starts_with("198.18.")
        })
        .map(|r| {
            format!(
                "{}:{}:{}:{}",
                r.destination, r.gateway, r.flags, r.interface
            )
        })
        .collect();
    routes.sort();
    let scutil = |arg: &str| {
        let out = Command::new("/usr/sbin/scutil").arg(arg).output().unwrap();
        assert!(out.status.success());
        String::from_utf8(out.stdout).unwrap()
    };
    let mut roots: Vec<_> = std::fs::read_dir("/private/var/run")
        .unwrap()
        .flatten()
        .filter_map(|e| {
            e.file_name()
                .to_str()
                .filter(|n| n.starts_with("nodecloak-ap."))
                .map(str::to_owned)
        })
        .collect();
    roots.sort();
    serde_json::json!({"interfaces":state.interfaces,"routes":routes,"network":state.signature,"dns":scutil("--dns"),"proxy":scutil("--proxy"),"rootDirectories":roots})
}
fn sudo_signal(pid: u32, signal: &str) {
    assert!(Command::new("/usr/bin/sudo")
        .args(["-n", "/bin/kill", signal, &pid.to_string()])
        .status()
        .unwrap()
        .success());
}
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "Real utun and root launchd jobs; disposable CI hosts only"]
async fn native_macos_application_proxy_routes_and_recovers_owned_network() {
    assert_eq!(
        std::env::var("NODECLOAK_ALLOW_MAC_TUN_TESTS").as_deref(),
        Ok("1")
    );
    assert!(Command::new("/usr/bin/sudo")
        .args(["-n", "/usr/bin/true"])
        .status()
        .unwrap()
        .success());
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let source = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/application_proxy_probe.rs");
    let probe = root.path().join("probe");
    assert!(Command::new("rustc")
        .arg(&source)
        .args(["--edition=2021", "-o"])
        .arg(&probe)
        .status()
        .unwrap()
        .success());
    let mut paths = vec![];
    for name in ["Codex", "ChatGPT", "Claude", "Unmatched"] {
        let dir = root.path().join(format!("{name}.app/Contents/MacOS"));
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("SameName");
        std::fs::copy(&probe, &exe).unwrap();
        paths.push((name.into(), exe.to_string_lossy().into_owned()));
    }
    let fixture = Fixture::start();
    let before = stable();
    let req = request(&paths[..3], fixture.port);
    let mut session = Session::begin(&req).await;
    for (name, path) in &paths[..3] {
        for mode in ["tcp", "udp"] {
            let path = path.clone();
            let output =
                tokio::task::spawn_blocking(move || Command::new(path).arg(mode).output().unwrap())
                    .await
                    .unwrap();
            assert!(output.status.success(), "{name} {mode}: {output:?}");
            println!("synthetic Mac {name} {mode}: proxy reached");
        }
    }
    assert_eq!(fixture.hits.load(Ordering::Acquire), 6);
    let output = Command::new(&paths[3].1).arg("tcp").output().unwrap();
    assert!(!output.status.success());
    assert_eq!(
        fixture.hits.load(Ordering::Acquire),
        6,
        "same-name foreign path was proxied"
    );
    session.stop().await;
    assert_eq!(stable(), before, "Stop changed network preferences/routes");
    drop(session);
    let req = request(&paths[..3], fixture.port);
    let session = Session::begin(&req).await;
    session.helper.close_control();
    session.released().await;
    assert_eq!(stable(), before, "GUI EOF retained network state");
    drop(session);
    let req = request(&paths[..3], fixture.port);
    let session = Session::begin(&req).await;
    sudo_signal(session.helper.launcher_pid(), "-KILL");
    session.released().await;
    assert_eq!(
        stable(),
        before,
        "launcher SIGKILL retained job/files/network"
    );
    drop(session);
    let req = request(&paths[..3], fixture.port);
    let session = Session::begin(&req).await;
    sudo_signal(session.helper.pid, "-KILL");
    session.released().await;
    assert_eq!(
        stable(),
        before,
        "helper SIGKILL retained an orphaned core/utun"
    );
    drop(session);
    let req = request(&paths[..3], fixture.port);
    let (mut pipe, helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut pipe, &req).await.unwrap();
    helper.close_control();
    assert!(helper.wait(15000));
    assert!(network::session_released(
        &crate::app_proxy::config::interface_name(&req.session).unwrap(),
        None
    )
    .unwrap());
    assert_eq!(stable(), before, "startup cancellation changed network");
    let occupied = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let mut req = request(&paths[..3], fixture.port);
    req.controller_port = occupied.local_addr().unwrap().port();
    let (mut pipe, helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut pipe, &req).await.unwrap();
    let result = tokio::time::timeout(
        Duration::from_secs(75),
        ipc::read_frame::<_, Reply>(&mut pipe),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(result, Reply::Failure { .. }));
    helper.close_control();
    assert!(helper.wait(15000));
    assert_eq!(stable(), before, "occupied controller changed network");
    println!("normal Stop, GUI EOF, helper SIGKILL, cancellation and occupied controller: restored Mac network");
}
#[tokio::test]
#[ignore = "Requires an explicitly signed disposable test executable"]
async fn local_audit_tokens_reject_a_different_executable_before_config() {
    let path = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let socket = path.path().join("peer.sock");
    let listener = tokio::net::UnixListener::bind(&socket).unwrap();
    let client = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let (server, _) = listener.accept().await.unwrap();
    let hash = super::identity::live_hash().unwrap();
    assert!(super::identity::peer(
        &server,
        &hash,
        Some(std::process::id()),
        Some(unsafe { libc::geteuid() })
    )
    .is_ok());
    assert!(super::identity::peer(&server, &"0".repeat(40), None, None).is_err());
    assert!(super::identity::peer(&server, &hash, Some(std::process::id() + 1), None).is_err());
    drop(client);
}
