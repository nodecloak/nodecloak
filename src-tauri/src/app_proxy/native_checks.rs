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
    sync::{atomic::Ordering, Arc},
    time::Duration,
};
const ID: &str = "0b4fcb93-ec83-4f9b-996a-f89f7aa96025";
#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="Pinned core and installed Ubuntu WSL required; loopback only, no TUN or host changes"]
async fn native_wsl_ingresses_reuse_assigned_nodes_with_authenticated_windows_interop(){
    use std::io::{Read,Write};
    let host=std::env::var_os("NODECLOAK_TEST_APP_PROXY_HOST").expect("compiled NodeCloak path");
    let binary=std::env::var_os("NODECLOAK_TEST_SINGBOX").expect("pinned core path");
    let first=Fixture::start();let second=Fixture::start();
    let mut request=request(&[],first.port);
    let second_id=uuid::Uuid::new_v4().to_string();
    let mut proxy=request.proxies[0].clone();proxy.id=second_id.clone();proxy.config.port=second.port;request.proxies.push(proxy);
    let ports:Vec<_>=(0..2).map(|_|TcpListener::bind("127.0.0.1:0").unwrap()).collect();
    request.wsl=vec![("codex",ID),("claude",second_id.as_str())].into_iter().zip(ports.iter()).map(|((app,id),listener)|super::wsl::Ingress{app:app.into(),proxy_id:id.into(),port:listener.local_addr().unwrap().port(),password:"b".repeat(64)}).collect();
    let mut config=super::config::compile_for_wsl(super::config::Platform::Windows,&[],&request.proxies,request.controller_port,&request.controller_secret,&request.session,&request.wsl).unwrap();
    config["inbounds"].as_array_mut().unwrap().retain(|v|v["type"]!="tun");
    drop(ports);
    struct Core(std::process::Child);
    impl Drop for Core {fn drop(&mut self){let _=self.0.kill();let _=self.0.wait();}}
    let mut core=Core(Command::new(binary).args(["run","--disable-color","-c","stdin"]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::null()).stderr(std::process::Stdio::null()).spawn().unwrap());
    core.0.stdin.take().unwrap().write_all(&serde_json::to_vec(&config).unwrap()).unwrap();
    for ingress in &request.wsl {
        let deadline=std::time::Instant::now()+Duration::from_secs(8);
        loop{if std::net::TcpStream::connect(("127.0.0.1",ingress.port)).is_ok(){break;}assert!(std::time::Instant::now()<deadline,"pinned core did not start");std::thread::sleep(Duration::from_millis(40));}
        let mut stream=std::net::TcpStream::connect(("127.0.0.1",ingress.port)).unwrap();stream.set_read_timeout(Some(Duration::from_secs(3))).unwrap();
        stream.write_all(b"CONNECT 198.18.0.10:43210 HTTP/1.1\r\nHost: 198.18.0.10:43210\r\n\r\n").unwrap();
        let mut response=[0;256];let count=stream.read(&mut response).unwrap();assert!(String::from_utf8_lossy(&response[..count]).contains("407"));
    }
    assert_eq!(first.hits.load(Ordering::Acquire),0);assert_eq!(second.hits.load(Ordering::Acquire),0);
    // Observe real Clash metadata while a CONNECT is pending, rather than
    // inventing an inbound field name for the desktop diagnostics.
    let mut held=std::net::TcpStream::connect(("127.0.0.1",request.wsl[0].port)).unwrap();
    use base64::Engine;
    let auth=base64::engine::general_purpose::STANDARD.encode(format!("nodecloak:{}",request.wsl[0].password));
    held.write_all(format!("CONNECT 198.18.0.10:43210 HTTP/1.1\r\nHost: 198.18.0.10:43210\r\nProxy-Authorization: Basic {auth}\r\n\r\n").as_bytes()).unwrap();
    held.set_read_timeout(Some(Duration::from_secs(3))).unwrap();let mut byte=[0];let mut header=vec![];while !header.ends_with(b"\r\n\r\n"){held.read_exact(&mut byte).unwrap();header.push(byte[0]);}
    let client=reqwest::Client::builder().no_proxy().build().unwrap();
    let deadline=tokio::time::Instant::now()+Duration::from_secs(3);
    let evidence=loop {
        let value:serde_json::Value=client.get(format!("http://127.0.0.1:{}/connections",request.controller_port)).bearer_auth(&request.controller_secret).send().await.unwrap().json().await.unwrap();
        let evidence=super::runtime::connections(&value,&Settings::default(),&[],&request.wsl);
        if !evidence.is_empty(){break evidence;}
        assert!(tokio::time::Instant::now()<deadline,"core did not register accepted CONNECT: {value}");
        tokio::time::sleep(Duration::from_millis(25)).await;
    };
    assert_eq!(evidence.len(),1);assert_eq!(evidence[0].app,"Codex (WSL)");assert_eq!(evidence[0].proxy_id,ID);
    drop(held);
    let script=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).parent().unwrap().join("scripts/check-wsl-proxy-interop.py");
    let output=Command::new("wsl.exe").args(["-d","Ubuntu","--exec","wslpath","-u"]).arg(script).output().unwrap();assert!(output.status.success());
    let script=String::from_utf8(output.stdout).unwrap().trim().to_owned();
    let apps=request.wsl.iter().map(|i|(i.app.clone(),serde_json::json!({"port":i.port,"password":i.password}))).collect::<serde_json::Map<_,_>>();
    let mut payload=serde_json::json!({"enabled":true,"windowsExe":host.to_string_lossy(),"apps":apps});
    for dead in [false,true] {
        if dead {core.0.kill().unwrap();core.0.wait().unwrap();payload["expectBlocked"]=true.into();}
        let mut probe=Command::new("wsl.exe").args(["-d","Ubuntu","--exec","python3",&script]).stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped()).spawn().unwrap();
        probe.stdin.take().unwrap().write_all(&serde_json::to_vec(&payload).unwrap()).unwrap();
        let output=probe.wait_with_output().unwrap();
        assert!(output.status.success(),"WSL interop (dead={dead}) failed: {} {}",String::from_utf8_lossy(&output.stdout),String::from_utf8_lossy(&output.stderr));
        assert_eq!(first.hits.load(Ordering::Acquire),1,"Codex missed its node or retried after failure");assert_eq!(second.hits.load(Ordering::Acquire),1,"Claude missed its node or retried after failure");
    }
    println!("Two assigned nodes, rejected missing auth, real Windows/WSL bridge, diagnostics and core-death isolation verified");
}
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
        stop_only: false,
        wsl: vec![],
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
async fn stop_confirmed(pipe:&mut tokio::net::windows::named_pipe::NamedPipeServer,helper:&security::Handle){
    ipc::write_frame(pipe,&ipc::Control::Stop).await.unwrap();
    let reply:Result<Reply,String>=tokio::time::timeout(Duration::from_secs(20),ipc::read_frame(pipe)).await.unwrap();
    let reply=reply.unwrap_or_else(|e|{let mut code=0;unsafe{windows_sys::Win32::System::Threading::GetExitCodeProcess(helper.0,&mut code)};panic!("Stop response failed: {e}; helper exit={code:#x}, protection={:?}",super::windows::protection::active());});
    assert!(matches!(reply,Reply::Stopped),"helper did not confirm explicit Stop");
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
    assert!(!super::windows::protection::active().unwrap(),"requires a clean disposable runner");
    struct ClearProtection;
    impl Drop for ClearProtection {fn drop(&mut self){super::windows::protection::clear().unwrap();}}
    let _protection_cleanup=ClearProtection;
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
    stop_confirmed(&mut pipe,&helper).await;
    drop(pipe);
    exited(helper, &name, luid).await;
    assert!(!super::windows::protection::active().unwrap(),"explicit Stop must restore ordinary network access");
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
    assert!(super::windows::protection::active().unwrap(),"owner EOF silently removed fail-closed protection");
    super::runtime::clear_protection().await.unwrap();
    assert!(!super::windows::protection::active().unwrap());
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
    stop_confirmed(&mut pipe,&helper).await;
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

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="Creates TUN and WFP policy on a disposable elevated Windows runner only"]
async fn native_cli_discovery_resolves_assigned_nodes_and_routes_tcp_udp(){
    use super::{cli,discovery,windows::protection};
    assert!(security::elevated());assert!(!protection::active().unwrap());
    struct Cleanup;impl Drop for Cleanup{fn drop(&mut self){protection::clear().unwrap();}}
    let _cleanup=Cleanup;
    let root=tempfile::tempdir().unwrap();let home=root.path().join("user");let npm=home.join("npm");
    let base=root.path().join("probe.exe");
    assert!(Command::new("rustc").arg(std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/application_proxy_probe.rs")).args(["--edition=2021","-o"]).arg(&base).status().unwrap().success());
    let claude=home.join(".local/bin/claude.exe");
    let extension=home.join(".vscode/extensions/anthropic.claude-code-2.1.322-win32-x64/resources/native-binary/claude.exe");
    let codex=npm.join("node_modules/@openai/codex/node_modules/@openai/codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
    for path in [&claude,&codex,&extension]{std::fs::create_dir_all(path.parent().unwrap()).unwrap();std::fs::copy(&base,path).unwrap();}
    let pnpm=root.path().join("pnpm-global");
    let wrapper=root.path().join(".pnpm/codex-version/node_modules/@openai/codex");
    std::fs::create_dir_all(&wrapper).unwrap();
    let pnpm_codex=wrapper.parent().unwrap().join("codex-win32-x64/vendor/x86_64-pc-windows-msvc/bin/codex.exe");
    std::fs::create_dir_all(pnpm_codex.parent().unwrap()).unwrap();std::fs::copy(&base,&pnpm_codex).unwrap();
    let link=pnpm.join("node_modules/@openai/codex");std::fs::create_dir_all(link.parent().unwrap()).unwrap();
    std::os::windows::fs::symlink_dir(&wrapper,&link).unwrap();
    let mut apps=discovery::empty_apps();cli::discover_in(&mut apps,&home,&[npm,pnpm]);
    assert_eq!(apps.iter().find(|a|a.id=="codex").unwrap().paths.len(),2,"dormant pnpm CLI missed");
    let mut settings=Settings::default();
    for rule in &mut settings.rules{if ["claude","codex"].contains(&rule.app.as_str()){rule.enabled=true;rule.proxy_id=Some(ID.into());}}
    settings.rules.iter_mut().find(|r|r.app=="claude").unwrap().executable=claude.to_string_lossy().into_owned();
    let (rules,_)=super::runtime::resolve(&settings,&apps).unwrap();
    assert_eq!(rules.len(),2);assert!(rules.iter().all(|r|r.proxy_id==ID));
    let fixture=Fixture::start();let mut req=request(&[],fixture.port);req.rules=rules;
    let (mut pipe,helper,luid)=begin(&req).await;
    let interface=super::config::interface_name(&req.session).unwrap();
    for path in [&claude,&codex,&pnpm_codex,&extension]{for mode in ["tcp","udp"]{
        let output=Command::new(path).arg(mode).output().unwrap();
        assert!(output.status.success(),"{} {mode} missed assigned proxy: {output:?}",path.display());
    }}
    assert_eq!(fixture.hits.load(Ordering::Acquire),8);
    let unrelated=root.path().join("node.exe");std::fs::copy(&base,&unrelated).unwrap();
    assert!(!Command::new(unrelated).arg("tcp").output().unwrap().status.success());
    assert_eq!(fixture.hits.load(Ordering::Acquire),8,"shared Node runtime was proxied");
    stop_confirmed(&mut pipe,&helper).await;drop(pipe);exited(helper,&interface,luid).await;
    assert!(!protection::active().unwrap());
    println!("Claude Code native + VS Code extension + npm Codex CLI: discovered, resolved, TCP/UDP assigned node verified; unrelated Node excluded");
}

#[tokio::test(flavor="multi_thread",worker_threads=2)]
#[ignore="Persistent WFP filters and routes: disposable elevated Windows runner only"]
async fn native_persistent_application_guard_survives_helper_and_core_failure(){
    use super::windows::protection;
    assert!(security::elevated());
    assert!(!protection::active().unwrap(),"never run against real application protection");
    struct Cleanup;
    impl Drop for Cleanup{fn drop(&mut self){protection::clear().unwrap();}}
    let _cleanup=Cleanup;
    let root=tempfile::tempdir().unwrap();
    let base=root.path().join("unmatched.exe");
    let source=std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/application_proxy_probe.rs");
    assert!(Command::new("rustc").arg(source).args(["--edition=2021","-o"]).arg(&base).status().unwrap().success());
    let selected=root.path().join("selected.exe");std::fs::copy(&base,&selected).unwrap();
    let probe=|path:&std::path::Path|Command::new(path).arg("external-connect").output().unwrap().status.success();
    assert!(probe(&selected),"runner must reach baseline external TCP endpoint");
    let fixture=Fixture::start();
    for failure in ["core","helper","owner"]{
        let req=request(&[("Selected".into(),selected.to_string_lossy().into_owned())],fixture.port);
        let (pipe,helper,luid)=begin(&req).await;
        let name=super::config::interface_name(&req.session).unwrap();
        if failure=="core"{
            let mut system=sysinfo::System::new_all();system.refresh_processes(sysinfo::ProcessesToUpdate::All,true);
            let core=system.processes().values().find(|p|p.parent().is_some_and(|p|p.as_u32()==helper.pid())&&p.name().to_string_lossy().eq_ignore_ascii_case("sing-box.exe")).expect("owned core missing");
            assert!(core.kill());
        }else if failure=="helper"{
            assert_ne!(unsafe{windows_sys::Win32::System::Threading::TerminateProcess(helper.0,99)},0);
        }
        drop(pipe);
        exited(helper,&name,luid).await;
        assert!(protection::active().unwrap(),"{failure} removed protection");
        assert!(!probe(&selected),"{failure} leaked selected app over direct TCP");
        assert!(probe(&base),"{failure} blocked unrelated app");
        super::runtime::clear_protection().await.unwrap();
        assert!(!protection::active().unwrap());
        assert!(probe(&selected),"explicit recovery did not restore connectivity");
        println!("{failure}: persistent blocking and explicit recovery verified");
    }
}
