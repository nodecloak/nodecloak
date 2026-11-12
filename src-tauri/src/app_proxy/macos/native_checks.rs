//! Disposable-host checks: these programs simulate traffic, not product sign-in.
use super::{network, transport};
use crate::app_proxy::{
    ipc::{self, Reply, StartRequest},
    model::{ResolvedProxy, ResolvedRule},
    test_fixture::Fixture,
};
use std::{process::Command, sync::atomic::Ordering, time::Duration};
const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
#[test]
#[ignore = "Read-only inspection of an official vendor DMG on disposable CI"]
fn native_macos_vendor_bundle_layout() {
    let bundle = std::path::PathBuf::from(std::env::var_os("NODECLOAK_MAC_VENDOR_BUNDLE").unwrap());
    let product = crate::app_proxy::macos_bundle::bundle_product(&bundle)
        .expect("official bundle identity must be recognized");
    let paths = crate::app_proxy::macos_bundle::executables(&bundle).unwrap();
    assert!(!paths.is_empty());
    println!(
        "Official {product}: {} executable paths, including actual helpers",
        paths.len()
    );
    for path in &paths {
        assert!(crate::app_proxy::config::valid_path(path));
        assert!(std::path::Path::new(path).starts_with(&bundle));
    }
}
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
        println!(
            "Starting native Mac session {} on {}",
            request.session,
            crate::app_proxy::config::interface_name(&request.session).unwrap()
        );
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
        stop_only: false,
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
        let dir = if name == "Claude" {
            root.path()
                .join("Claude.app/Contents/Frameworks/Claude Helper.app/Contents/MacOS")
        } else {
            root.path().join(format!("{name}.app/Contents/MacOS"))
        };
        std::fs::create_dir_all(&dir).unwrap();
        let exe = dir.join("SameName");
        std::fs::copy(&probe, &exe).unwrap();
        paths.push((name.into(), exe.to_string_lossy().into_owned()));
    }
    let fixture = Fixture::start();
    let before = stable();
    let req = request(&paths[..3], fixture.port);
    let mut session = Session::begin(&req).await;
    // A second root helper must fail without touching the active session.
    let (mut second, second_helper) = transport::connect_helper().await.unwrap();
    ipc::write_frame(&mut second, &request(&paths[..3], fixture.port))
        .await
        .unwrap();
    let failure = tokio::time::timeout(
        Duration::from_secs(10),
        ipc::read_frame::<_, Reply>(&mut second),
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(failure,Reply::Failure{message} if message.contains("另一应用代理会话")));
    second_helper.close_control();
    assert!(second_helper.wait(15000));
    assert_eq!(
        network::owned_luid(&session.interface).unwrap(),
        Some(session.index)
    );
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
    let children = Command::new("/bin/ps")
        .args(["-axo", "pid=,ppid=,comm="])
        .output()
        .unwrap();
    let children = String::from_utf8(children.stdout).unwrap();
    let core: Vec<u32> = children
        .lines()
        .filter_map(|line| {
            let mut p = line.split_whitespace();
            let pid = p.next()?.parse().ok()?;
            let parent: u32 = p.next()?.parse().ok()?;
            (parent == session.helper.pid && p.collect::<Vec<_>>().join(" ").ends_with("/sing-box"))
                .then_some(pid)
        })
        .collect();
    assert_eq!(core.len(), 1, "must address exactly the owned core");
    sudo_signal(core[0], "-KILL");
    session.released().await;
    assert_eq!(stable(), before, "core crash retained network state");
    drop(session);
    // A dead assigned node must not silently become a direct connection.
    let unavailable = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let port = unavailable.local_addr().unwrap().port();
    drop(unavailable);
    let session = Session::begin(&request(&paths[..3], port)).await;
    let output = Command::new(&paths[0].1).arg("tcp").output().unwrap();
    assert!(
        !output.status.success(),
        "unavailable node unexpectedly carried traffic"
    );
    session.helper.close_control();
    session.released().await;
    assert_eq!(stable(), before, "dead proxy retained network state");
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
    let mut client = tokio::net::UnixStream::connect(&socket).await.unwrap();
    let (mut server, _) = listener.accept().await.unwrap();
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
    assert!(
        super::identity::peer(&server, &hash, None, Some(unsafe { libc::geteuid() } + 1)).is_err()
    );
    let mut token = [0u32; 8];
    let mut length = std::mem::size_of_val(&token) as libc::socklen_t;
    use std::os::fd::AsRawFd;
    assert_eq!(
        unsafe {
            libc::getsockopt(
                server.as_raw_fd(),
                0,
                6,
                token.as_mut_ptr().cast(),
                &mut length,
            )
        },
        0
    );
    token[7] ^= 1;
    assert!(
        super::identity::validate_token(&token, &hash).is_err(),
        "reused or exec-replaced PID must be rejected by the kernel generation check"
    );
    ipc::write_frame(
        &mut server,
        &ipc::Hello {
            protocol: 1,
            token: "a".repeat(64),
        },
    )
    .await
    .unwrap();
    let _: ipc::Hello = ipc::read_frame(&mut client).await.unwrap();
    assert!(
        super::identity::peer(
            &client,
            &hash,
            Some(std::process::id()),
            Some(unsafe { libc::geteuid() })
        )
        .is_ok(),
        "client must authenticate the accepting writer before sending config"
    );
    drop(client);
    let mut foreign = Command::new("/usr/bin/nc")
        .arg("-U")
        .arg(&socket)
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .unwrap();
    let (server, _) = tokio::time::timeout(Duration::from_secs(5), listener.accept())
        .await
        .unwrap()
        .unwrap();
    assert!(
        super::identity::peer(
            &server,
            &hash,
            Some(foreign.id()),
            Some(unsafe { libc::geteuid() })
        )
        .is_err(),
        "foreign executable passed live code validation"
    );
    let _ = foreign.kill();
    let _ = foreign.wait();
}

#[test]
#[ignore = "Root bootstrap negative checks; disposable Mac CI only"]
fn native_macos_bootstrap_rejects_changed_and_external_files() {
    assert_eq!(
        std::env::var("NODECLOAK_ALLOW_MAC_TUN_TESTS").as_deref(),
        Ok("1")
    );
    let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
    let before = stable();
    let app = root.path().join("Fixture.app");
    let executable = app.join("Contents/MacOS/fixture");
    std::fs::create_dir_all(executable.parent().unwrap()).unwrap();
    std::fs::copy("/usr/bin/true", &executable).unwrap();
    let info = r#"<?xml version="1.0"?><plist version="1.0"><dict><key>CFBundleIdentifier</key><string>com.nodecloak.bootstrap-negative</string><key>CFBundleExecutable</key><string>fixture</string><key>CFBundlePackageType</key><string>APPL</string></dict></plist>"#;
    std::fs::write(app.join("Contents/Info.plist"), info).unwrap();
    assert!(Command::new("/usr/bin/codesign")
        .args(["--force", "--sign", "-"])
        .arg(&app)
        .status()
        .unwrap()
        .success());
    let manifest = crate::app_proxy::macos_loader::inspect(&executable).unwrap();
    let suffix = uuid::Uuid::new_v4().to_string();
    let script = crate::app_proxy::macos_support::bootstrap(
        executable.to_str().unwrap(),
        &suffix,
        std::process::id(),
        &super::identity::live_hash().unwrap(),
        true,
        &manifest,
    )
    .unwrap();
    let source = root.path().join("authorize.applescript");
    std::fs::write(
        &source,
        crate::app_proxy::macos_support::applescript(&script),
    )
    .unwrap();
    assert!(Command::new("/usr/bin/osacompile")
        .arg("-o")
        .arg(root.path().join("authorize.scpt"))
        .arg(source)
        .status()
        .unwrap()
        .success());
    let sentinel = root.path().join("external.txt");
    std::fs::write(&sentinel, "must survive").unwrap();
    let link = app.join("Contents/external");
    std::os::unix::fs::symlink(&sentinel, &link).unwrap();
    let run = || {
        Command::new("/usr/bin/sudo")
            .args(["-n", "/bin/sh", "-c", &script])
            .output()
            .unwrap()
    };
    let result = run();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("external or broken symlink"),
        "{result:?}"
    );
    assert_eq!(std::fs::read_to_string(&sentinel).unwrap(), "must survive");
    assert_eq!(
        stable(),
        before,
        "rejected bundle changed network or left root files"
    );
    std::fs::remove_file(link).unwrap();
    // Simulate a source replacement between inspection and root copy.
    let mut bytes = std::fs::read(&executable).unwrap();
    bytes.push(0);
    std::fs::write(&executable, bytes).unwrap();
    let result = run();
    assert!(!result.status.success());
    assert!(
        String::from_utf8_lossy(&result.stderr).contains("program changed"),
        "{result:?}"
    );
    assert_eq!(
        stable(),
        before,
        "replaced source left root files or network changes"
    );
}
