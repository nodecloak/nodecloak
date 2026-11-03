use super::*;
use serde_json::json;
use std::{path::PathBuf, thread};
#[test]
fn helper_accepts_hysteria2_and_keeps_system_network_guardrails() {
    let up=Upstream{config:serde_json::from_value(json!({"mode":"hysteria2","host":"node.invalid","port":443,"node":{"tls":"tls","serverName":"node.invalid","insecure":true},"preProxy":{"mode":"socks5","host":"127.0.0.1","port":7890}})).unwrap(),password:"user:OpaquePassword".into()};
    let config=node::config(&up,12345,"fixture","secret").unwrap();
    assert!(validate_helper_config(&config).is_ok());
    for key in ["set_system_proxy","listen"] {
        let mut invalid=config.clone();invalid["inbounds"][0][key]=if key=="listen"{json!("0.0.0.0")}else{json!(true)};
        assert!(validate_helper_config(&invalid).is_err());
    }
    let mut invalid=config;invalid["outbounds"][0]["uuid"]=json!("unexpected");assert!(validate_helper_config(&invalid).is_err());
}

#[test]
#[ignore = "Starts loopback-only HY2 server, UDP SOCKS5 hop and owned profile relays"]
fn installed_hysteria2_transfers_direct_and_udp_chain_and_rejects_bad_auth_tls() {
    let binary=PathBuf::from(std::env::var_os("NODECLOAK_TEST_SINGBOX").expect("Set pinned sing-box fixture binary"));
    let generated=crate::platform::command(binary.to_str().unwrap()).args(["generate","tls-keypair","nodecloak-test.invalid"]).output().unwrap();
    assert!(generated.status.success());
    let pem=String::from_utf8(generated.stdout).unwrap();
    let certificate=pem[pem.find("-----BEGIN CERTIFICATE-----").unwrap()..].trim().to_owned();
    let key=pem[..pem.find("-----BEGIN CERTIFICATE-----").unwrap()].trim().to_owned();
    tauri::async_runtime::block_on(async {
        let origin=TcpListener::bind("127.0.0.1:0").await.unwrap();let origin_port=origin.local_addr().unwrap().port();
        let hits=Arc::new(std::sync::atomic::AtomicUsize::new(0));let received=hits.clone();
        let target=tokio::spawn(async move { loop {
            let (mut stream,_)=origin.accept().await.unwrap();let received=received.clone();
            tokio::spawn(async move { if header(&mut stream).await.is_ok(){received.fetch_add(1,Ordering::SeqCst);let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").await;} });
        }});
        let reservation=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let node_port=reservation.local_addr().unwrap().port();drop(reservation);
        let config=json!({"log":{"disabled":true},"inbounds":[{"type":"hysteria2","listen":"127.0.0.1","listen_port":node_port,"users":[{"password":"fixture-user:OpaqueSecret"}],"tls":{"enabled":true,"certificate":[certificate],"key":[key]}}],"outbounds":[{"type":"direct","tag":"fixture"}],"route":{"rules":[{"ip_cidr":["127.0.0.1/32"],"action":"route","outbound":"fixture"},{"action":"reject"}]}});
        let mut child=crate::platform::command(binary.to_str().unwrap()).args(["run","-c","stdin"]).stdin(Stdio::piped()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().unwrap();
        child.stdin.take().unwrap().write_all(serde_json::to_string(&config).unwrap().as_bytes()).unwrap();
        let mut server=Server{child,_directory:tempfile::tempdir().unwrap()};
        tokio::time::sleep(Duration::from_millis(400)).await;assert!(server.child.try_wait().unwrap().is_none());
        let hop=TcpListener::bind("127.0.0.1:0").await.unwrap();let hop_port=hop.local_addr().unwrap().port();
        let packets=Arc::new(std::sync::atomic::AtomicUsize::new(0));let forwarded=packets.clone();
        let first_hop=tokio::spawn(async move {
            let (mut control,_)=hop.accept().await.unwrap();assert_eq!(control.read_u8().await.unwrap(),5);
            let count=control.read_u8().await.unwrap();let mut methods=vec![0;count as usize];control.read_exact(&mut methods).await.unwrap();assert!(methods.contains(&0));control.write_all(&[5,0]).await.unwrap();
            let mut request=[0;4];control.read_exact(&mut request).await.unwrap();assert_eq!(&request[..3],&[5,3,0]);
            let mut association=vec![0;match request[3]{1=>6,4=>18,_=>panic!("unexpected UDP associate address type")}];control.read_exact(&mut association).await.unwrap();
            let udp=tokio::net::UdpSocket::bind("127.0.0.1:0").await.unwrap();let udp_port=udp.local_addr().unwrap().port();
            let mut reply=vec![5,0,0,1,127,0,0,1];reply.extend(udp_port.to_be_bytes());control.write_all(&reply).await.unwrap();
            let node=std::net::SocketAddr::from(([127,0,0,1],node_port));let mut client=None;let mut data=vec![0;65535];
            loop {tokio::select! {
                _=control.read_u8()=>break,
                packet=udp.recv_from(&mut data)=>{
                    let (length,source)=packet.unwrap();
                    if source==node {
                        if let Some(client)=client {let mut frame=vec![0,0,0,1,127,0,0,1];frame.extend(node_port.to_be_bytes());frame.extend_from_slice(&data[..length]);udp.send_to(&frame,client).await.unwrap();}
                    }else{
                        assert!(length>=10);assert_eq!(&data[..8],&[0,0,0,1,127,0,0,1]);assert_eq!(u16::from_be_bytes([data[8],data[9]]),node_port);
                        client=Some(source);forwarded.fetch_add(1,Ordering::SeqCst);udp.send_to(&data[10..length],node).await.unwrap();
                    }
                }
            }}
        });
        for (insecure,password,chain,expected) in [(true,"fixture-user:OpaqueSecret",false,true),(false,"fixture-user:OpaqueSecret",false,false),(true,"wrong-password",false,false),(true,"fixture-user:OpaqueSecret",true,true)] {
            let up=Upstream {config:serde_json::from_value(json!({"mode":"hysteria2","host":"127.0.0.1","port":node_port,"node":{"tls":"tls","serverName":"nodecloak-test.invalid","insecure":insecure}})).unwrap(),password:password.into()};
            let mut up=up;if chain{up.config.pre_proxy=Some(LocalProxy{mode:"socks5".into(),host:"127.0.0.1".into(),port:hop_port});}
            let before=hits.load(Ordering::SeqCst);let relay=super::super::start(up,0).await.unwrap();
            let client=reqwest::Client::builder().no_proxy().proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}",relay.port)).unwrap()).timeout(Duration::from_secs(8)).build().unwrap();
            let result=client.get(format!("http://127.0.0.1:{origin_port}/")).send().await;
            let success=match result {Ok(response)=>response.status().is_success()&&response.text().await.unwrap_or_default()=="fixture",Err(_)=>false};
            assert_eq!(success,expected,"HY2 insecure={insecure},chain={chain}");assert_eq!(hits.load(Ordering::SeqCst)-before,usize::from(expected));
            drop(client);drop(relay);eprintln!("HY2 insecure={insecure},chain={chain},expected_success={expected}: passed");
        }
        assert!(packets.load(Ordering::SeqCst)>0,"SOCKS5 UDP first hop was bypassed");
        first_hop.abort();target.abort();drop(server);
    });
}
#[test]
fn windows_sid_accepts_local_and_entra_users_without_acl_arguments() {
    for sid in [
        "S-1-5-21-111-222-333-1001",
        "S-1-12-1-788341310-1134859379-3309005462-3346259773",
    ] {
        let csv = format!("\"example\\user\",\"{sid}\"\r\n");
        assert_eq!(sid_from_csv(&csv), Some(sid));
    }
    for sid in [
        "S-1-5-1:(F)",
        "S-1-12-x",
        "S-1-12-4294967296",
        "S-1-12-+1",
        "S-1-281474976710656-1",
    ] {
        assert!(sid_from_csv(&format!("\"user\",\"{sid}\"")).is_none());
    }
}

struct Server {
    child: Child,
    _directory: tempfile::TempDir,
}
impl Drop for Server {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
fn local_server(mode: &str, uuid: &str, port: u16) -> Server {
    transport_server(mode, uuid, port, "tcp")
}
fn transport_server(mode: &str, uuid: &str, port: u16, transport: &str) -> Server {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(if cfg!(windows) {
        "server.exe"
    } else {
        "server"
    });
    let mut bytes = Vec::new();
    flate2::read::GzDecoder::new(&include_bytes!(concat!(env!("OUT_DIR"), "/core.gz"))[..])
        .read_to_end(&mut bytes)
        .unwrap();
    std::fs::write(&path, bytes).unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o700)).unwrap();
    }
    let mut config = json!({"log":{"disabled":true},"inbounds":[{"type":mode,"listen":"127.0.0.1","listen_port":port,"users":[{"uuid":uuid}]}],"outbounds":[{"type":"direct","tag":"fixture"}],"route":{"rules":[{"domain":["nodecloak-browser-fixture.invalid"],"action":"route","outbound":"fixture","override_address":"127.0.0.1"},{"ip_cidr":["127.0.0.1/32"],"action":"route","outbound":"fixture"},{"action":"reject"}]}});
    if transport != "tcp" {
        config["inbounds"][0]["transport"] = if transport == "grpc" {
            json!({"type":transport,"service_name":"fixture"})
        } else {
            json!({"type":transport,"path":"/fixture"})
        };
    }
    let mut child = crate::platform::command(path.to_str().unwrap())
        .args(["run", "-c", "stdin"])
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(&config).unwrap().as_bytes())
        .unwrap();
    let deadline = Instant::now() + Duration::from_secs(5);
    while std::net::TcpStream::connect(("127.0.0.1", port)).is_err() {
        assert!(Instant::now() < deadline);
        thread::sleep(Duration::from_millis(40));
    }
    Server {
        child,
        _directory: directory,
    }
}
fn free_port() -> u16 {
    std::net::TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}
fn owned_child(core: &Core) -> (sysinfo::Pid, PathBuf) {
    let host = core.child.lock().unwrap().id();
    let system = sysinfo::System::new_all();
    eprintln!(
        "Owned host {host} child inventory: {:?}",
        system
            .processes()
            .values()
            .filter(|p| p.parent().is_some_and(|p| p.as_u32() == host))
            .map(|p| (p.pid(), p.name(), p.exe()))
            .collect::<Vec<_>>()
    );
    let child = system
        .processes()
        .values()
        .find(|p| {
            p.parent().is_some_and(|p| p.as_u32() == host)
                && p.exe()
                    .and_then(|path| path.file_name())
                    .is_some_and(|name| {
                        name == if cfg!(windows) {
                            "sing-box.exe"
                        } else {
                            "sing-box"
                        }
                    })
        })
        .expect("owned fixture child");
    (child.pid(), child.exe().unwrap().to_path_buf())
}
async fn wait_removed(binary: &std::path::Path) {
    let deadline = Instant::now() + Duration::from_secs(10);
    while binary.exists() && Instant::now() < deadline {
        tokio::time::sleep(Duration::from_millis(25)).await;
    }
    assert!(
        !binary.exists(),
        "owned process/extracted binary survived parent pipe EOF"
    );
}

#[test]
fn generated_config_cannot_modify_system_network_and_first_hop_is_node_dialer() {
    let up=Upstream{config:serde_json::from_value(json!({"mode":"vless","host":"node.invalid","port":443,"node":{},"preProxy":{"mode":"socks5","host":"192.168.10.1","port":7890}})).unwrap(),password:uuid::Uuid::new_v4().to_string()};
    let config = node::config(&up, 34234, "internal", "secret").unwrap();
    assert_eq!(config["inbounds"][0]["listen"], "127.0.0.1");
    assert_eq!(config["inbounds"][0]["set_system_proxy"], false);
    assert_eq!(config["outbounds"][0]["detour"], "first-hop");
    assert_eq!(config["outbounds"][1]["type"], "socks");
    assert_eq!(config["route"]["final"], "node");
    assert!(config.get("services").is_none() && config.get("experimental").is_none());
    assert!(config["dns"].get("listen").is_none());
    assert_eq!(config["inbounds"].as_array().unwrap().len(), 1);
}

#[test]
fn helper_accepts_supported_node_tls_and_transport_combinations() {
    for mode in ["vless", "vmess"] {
        for transport in ["tcp", "ws", "httpupgrade", "grpc"] {
            for tls in ["none", "tls", "reality"] {
                if mode == "vmess" && tls == "reality" {
                    continue;
                }
                let mut options = node::NodeOptions::default();
                options.transport = transport.into();
                options.tls = tls.into();
                if tls != "none" {
                    options.server_name = "node.invalid".into();
                    options.fingerprint = "chrome".into();
                    options.alpn = vec!["h2".into()];
                }
                if tls == "reality" {
                    options.public_key = "A".repeat(43);
                    options.short_id = "0123".into();
                }
                if transport == "grpc" {
                    options.service_name = "fixture".into();
                } else if transport != "tcp" {
                    options.path = "/fixture".into();
                    options.host = "cdn.invalid".into();
                }
                let up = Upstream {
                    config: ProxyConfig {
                        mode: mode.into(),
                        host: "node.invalid".into(),
                        port: 443,
                        node: Some(options),
                        pre_proxy: Some(LocalProxy {
                            mode: "https".into(),
                            host: "first.invalid".into(),
                            port: 443,
                        }),
                        ..Default::default()
                    },
                    password: uuid::Uuid::new_v4().to_string(),
                };
                assert!(
                    validate_helper_config(
                        &node::config(&up, 12345, "fixture", "fixture").unwrap()
                    )
                    .is_ok(),
                    "{mode}/{transport}/{tls}"
                );
            }
        }
    }
}

#[test]
fn helper_rejects_system_network_configuration() {
    for config in [
        json!({"inbounds":[{"type":"tun"}],"route":{"final":"node"}}),
        json!({"inbounds":[{"type":"http","listen":"0.0.0.0","set_system_proxy":false}],"route":{"final":"node"}}),
        json!({"inbounds":[{"type":"http","listen":"127.0.0.1","set_system_proxy":true}],"route":{"final":"node"}}),
    ] {
        assert!(validate_helper_config(&config).is_err());
    }
    let up = Upstream {
        config: serde_json::from_value(
            json!({"mode":"vless","host":"node.invalid","port":443,"node":{}}),
        )
        .unwrap(),
        password: uuid::Uuid::new_v4().to_string(),
    };
    let generated = node::config(&up, 23456, "fixture", "fixture").unwrap();
    assert!(validate_helper_config(&generated).is_ok());
    for (pointer, value) in [
        ("/log/output", json!("arbitrary-file")),
        ("/outbounds/0/type", json!("wireguard")),
        ("/outbounds/0/bind_interface", json!("other-network")),
        ("/dns/servers/0/type", json!("https")),
        ("/route/auto_detect_interface", json!(true)),
    ] {
        let mut changed = generated.clone();
        let (parent, key) = pointer.rsplit_once('/').unwrap();
        changed.pointer_mut(parent).unwrap()[key] = value;
        assert!(
            validate_helper_config(&changed).is_err(),
            "arbitrary configuration accepted: {pointer}"
        );
    }
}

#[test]
#[ignore = "Runs a disposable owned helper against an unrelated fake listener"]
fn occupied_port_cannot_impersonate_owned_core() {
    tauri::async_runtime::block_on(async {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let fake = tokio::spawn(async move {
            loop {
                let (mut socket, _) = listener.accept().await.unwrap();
                tokio::spawn(async move {
                    let request = String::from_utf8(header(&mut socket).await.unwrap()).unwrap();
                    socket
                        .write_all(if request.contains("Proxy-Authorization:") {
                            b"HTTP/1.1 200 OK\r\n\r\n"
                        } else {
                            b"HTTP/1.1 407 Authentication Required\r\n\r\n"
                        })
                        .await
                        .unwrap();
                });
            }
        });
        let internal = Upstream {
            config: ProxyConfig {
                mode: "http".into(),
                host: "127.0.0.1".into(),
                port,
                username: "fixture".into(),
                ..Default::default()
            },
            password: "fixture".into(),
        };
        assert!(
            ready(&internal).await,
            "fixture must mimic the old HTTP-only readiness"
        );
        let up = Upstream {
            config: serde_json::from_value(
                json!({"mode":"vless","host":"node.invalid","port":443,"node":{}}),
            )
            .unwrap(),
            password: uuid::Uuid::new_v4().to_string(),
        };
        let core = Core::spawn(&node::config(&up, port, "fixture", "fixture").unwrap()).unwrap();
        assert!(
            !(core.alive() && ready(&internal).await),
            "unrelated listener impersonated the core before the owned bind completed"
        );
        drop(core);
        fake.abort();
    });
}

#[test]
#[ignore = "Starts synthetic local nodes, owned cores and disposable Chrome/Edge/Firefox profiles"]
fn installed_core_isolated_nodes_first_hop_shutdown_and_fail_closed() {
    let host = std::env::var_os("NODECLOAK_TEST_PROXY_HOST")
        .expect("Set built NodeCloak executable as NODECLOAK_TEST_PROXY_HOST");
    assert!(PathBuf::from(host).is_file());
    tauri::async_runtime::block_on(async {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        let hits = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let origin_hits = hits.clone();
        let paths = Arc::new(Mutex::new(std::collections::HashSet::<String>::new()));
        let requests = paths.clone();
        let observations=Arc::new(Mutex::new(std::collections::BTreeMap::<&'static str,usize>::new()));
        let traffic=observations.clone();
        let target = tokio::spawn(async move {
            loop {
                let (mut stream, _) = origin.accept().await.unwrap();
                let hits = origin_hits.clone();
                let requests = requests.clone();
                let traffic=traffic.clone();
                tokio::spawn(async move {
                    *traffic.lock().unwrap().entry("accepted").or_default()+=1;
                    // A plaintext HTTP fixture must reject a TLS probe or idle
                    // preconnect promptly, not wait forever for CRLFCRLF.
                    let first=match tokio::time::timeout(Duration::from_secs(5),stream.read_u8()).await{
                        Ok(Ok(first))=>first,
                        _=>{*traffic.lock().unwrap().entry("idle_or_closed").or_default()+=1;return;}
                    };
                    if !first.is_ascii_uppercase(){
                        *traffic.lock().unwrap().entry(if first==0x16{"tls_probe"}else{"non_http"}).or_default()+=1;
                        let _=stream.write_all(b"HTTP/1.1 400 Bad Request\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").await;return;
                    }
                    let rest=match tokio::time::timeout(Duration::from_secs(5),header(&mut stream)).await{
                        Ok(Ok(bytes))=>bytes,
                        _=>{*traffic.lock().unwrap().entry("incomplete_header").or_default()+=1;return;}
                    };
                    let mut bytes=vec![first];bytes.extend(rest);
                    let request = String::from_utf8_lossy(&bytes);
                    if let Some(path) = request.split_whitespace().nth(1) {
                        requests.lock().unwrap().insert(path.to_owned());
                    }
                    hits.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let _=stream.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").await;
                });
            }
        });
        let mut leases = vec![];
        let mut servers = vec![];
        let mut first_hops = vec![];
        for mode in ["vless", "vmess"] {
            let first_mode = if mode == "vless" { "http" } else { "socks5" };
            let uuid = uuid::Uuid::new_v4().to_string();
            let port = free_port();
            servers.push(local_server(mode, &uuid, port));
            let first = TcpListener::bind("127.0.0.1:0").await.unwrap();
            let first_port = first.local_addr().unwrap().port();
            let connects = Arc::new(std::sync::atomic::AtomicUsize::new(0));
            let count = connects.clone();
            let secret = uuid.clone();
            let first_task = tokio::spawn(async move {
                loop {
                    let (mut incoming, _) = first.accept().await.unwrap();
                    let count = count.clone();
                    let secret = secret.clone();
                    tokio::spawn(async move {
                        if first_mode == "http" {
                            let request =
                                String::from_utf8(header(&mut incoming).await.unwrap()).unwrap();
                            assert!(
                                request.starts_with(&format!(
                                    "CONNECT second-hop.invalid:{port} HTTP/1.1"
                                )),
                                "first hop received wrong destination"
                            );
                            assert!(
                                !request.contains(&secret),
                                "remote credential reused in first-hop CONNECT headers"
                            );
                            incoming
                                .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                                .await
                                .unwrap();
                        } else {
                            let mut greeting = [0u8; 3];
                            incoming.read_exact(&mut greeting).await.unwrap();
                            assert_eq!(greeting, [5, 1, 0]);
                            incoming.write_all(&[5, 0]).await.unwrap();
                            let mut request = [0u8; 5];
                            incoming.read_exact(&mut request).await.unwrap();
                            assert_eq!(&request[..4], &[5, 1, 0, 3]);
                            let mut host = vec![0; request[4] as usize];
                            incoming.read_exact(&mut host).await.unwrap();
                            assert_eq!(host, b"second-hop.invalid");
                            assert_eq!(incoming.read_u16().await.unwrap(), port);
                            incoming
                                .write_all(&[5, 0, 0, 1, 127, 0, 0, 1, 0, 0])
                                .await
                                .unwrap();
                        }
                        let mut remote = TcpStream::connect(("127.0.0.1", port)).await.unwrap();
                        count.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                        let _ = tokio::io::copy_bidirectional(&mut incoming, &mut remote).await;
                    });
                }
            });
            first_hops.push(first_task);
            let config=serde_json::from_value(json!({"mode":mode,"host":"second-hop.invalid","port":port,"node":{},"preProxy":{"mode":first_mode,"host":"127.0.0.1","port":first_port}})).unwrap();
            let relay = super::super::start(
                Upstream {
                    config,
                    password: uuid.clone(),
                },
                0,
            )
            .await
            .unwrap();
            assert_eq!(
                connects.load(std::sync::atomic::Ordering::SeqCst),
                0,
                "readiness dialed the user node"
            );
            assert!(relay.ready());
            let client = reqwest::Client::builder()
                .no_proxy()
                .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", relay.port)).unwrap())
                .timeout(Duration::from_secs(5))
                .build()
                .unwrap();
            let response = client
                .get(format!("http://127.0.0.1:{origin_port}/"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap();
            assert_eq!(response, "fixture");
            assert_eq!(connects.load(std::sync::atomic::Ordering::SeqCst), 1);
            struct BrowserCleanup(PathBuf);
            impl Drop for BrowserCleanup {
                fn drop(&mut self) {
                    let _ = crate::platform::force_close_profile(&self.0);
                }
            }
            for browser in ["chrome", "edge", "firefox"] {
                crate::platform::browser_path(browser).expect("all three browsers required");
                let directory =
                    tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
                let profile = directory.path().join("fixture");
                std::fs::create_dir_all(&profile).unwrap();
                let _cleanup = BrowserCleanup(profile.clone());
                let path = format!("/{mode}-{browser}");
                crate::platform::spawn_browser_with_proxy(
                    browser,
                    &profile,
                    &format!("http://nodecloak-browser-fixture.invalid:{origin_port}{path}"),
                    mode,
                    Some(relay.port),
                )
                .unwrap();
                let deadline = Instant::now() + Duration::from_secs(25);
                while !paths.lock().unwrap().contains(&path) && Instant::now() < deadline {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                if !paths.lock().unwrap().contains(&path){
                    let pids=crate::platform::profile_process_ids(&profile).unwrap_or_default();
                    panic!("{browser} did not use {mode}/{first_mode}: profile_pids={pids:?}, proxy_ready={}, first_hop_connects={}, origin={:?}, observed_paths={:?}, relay_error={:?}",relay.ready(),connects.load(std::sync::atomic::Ordering::SeqCst),observations.lock().unwrap(),paths.lock().unwrap(),relay.failure.lock().unwrap());
                }
                crate::platform::force_close_profile(&profile).unwrap();
                crate::platform::wait_profile_closed(&profile, Duration::from_secs(10)).unwrap();
                assert!(connects.load(std::sync::atomic::Ordering::SeqCst) > 1);
                eprintln!("{browser}: {first_mode} -> {mode} -> local fixture passed");
            }
            leases.push((relay, client));
        }
        assert_ne!(leases[0].0.port, leases[1].0.port);
        // Identify only the child belonging to this test host, never a global core name.
        let (owned_pid, _) = owned_child(leases[0].0.core.as_ref().unwrap());
        let system = sysinfo::System::new_all();
        let owned = system.process(owned_pid).unwrap();
        assert!(owned.name().to_string_lossy().contains("sing-box"));
        assert!(owned.kill());
        let exit_deadline = Instant::now() + Duration::from_secs(5);
        while leases[0].0.ready() && Instant::now() < exit_deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        let before = hits.load(std::sync::atomic::Ordering::SeqCst);
        assert!(!leases[0].0.ready());
        assert!(leases[0]
            .1
            .get(format!("http://127.0.0.1:{origin_port}/"))
            .send()
            .await
            .is_err());
        assert_eq!(
            hits.load(std::sync::atomic::Ordering::SeqCst),
            before,
            "failed proxy fell back to direct"
        );
        assert_eq!(
            leases[1]
                .1
                .get(format!("http://127.0.0.1:{origin_port}/"))
                .send()
                .await
                .unwrap()
                .text()
                .await
                .unwrap(),
            "fixture"
        );
        let remaining = leases.pop().unwrap();
        let core = remaining.0.core.as_ref().unwrap().clone();
        let (_, binary) = owned_child(&core);
        drop(remaining);
        let deadline = Instant::now() + Duration::from_secs(2);
        while Arc::strong_count(&core) > 1 && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert_eq!(
            Arc::strong_count(&core),
            1,
            "closing a relay retained its core"
        );
        drop(core);
        wait_removed(&binary).await;
        // Closing the owner pipe models abrupt app exit: no graceful Core Drop
        // call is needed in the parent for the helper to reap its child.
        let up = Upstream {
            config: serde_json::from_value(
                json!({"mode":"vless","host":"127.0.0.1","port":443,"node":{}}),
            )
            .unwrap(),
            password: uuid::Uuid::new_v4().to_string(),
        };
        let (_, pipe_core) = start(up).await.unwrap();
        let (_, pipe_binary) = owned_child(&pipe_core);
        pipe_core.input.lock().unwrap().take();
        wait_removed(&pipe_binary).await;
        assert!(!pipe_core.alive());
        // Cancellation before preparation completes must not start a late core.
        let up = Upstream {
            config: serde_json::from_value(
                json!({"mode":"vless","host":"node.invalid","port":443,"node":{}}),
            )
            .unwrap(),
            password: uuid::Uuid::new_v4().to_string(),
        };
        let cancelled =
            Core::spawn(&node::config(&up, free_port(), "fixture", "fixture").unwrap()).unwrap();
        cancelled.input.lock().unwrap().take();
        let deadline = Instant::now() + Duration::from_secs(5);
        while cancelled.running() && Instant::now() < deadline {
            tokio::time::sleep(Duration::from_millis(25)).await;
        }
        assert!(
            !cancelled.running(),
            "cancelled preparation did not release its helper"
        );
        drop(cancelled);
        #[cfg(windows)]
        {
            let up = Upstream {
                config: serde_json::from_value(
                    json!({"mode":"vmess","host":"127.0.0.1","port":443,"node":{}}),
                )
                .unwrap(),
                password: uuid::Uuid::new_v4().to_string(),
            };
            let (_, job_core) = start(up).await.unwrap();
            let (pid, binary) = owned_child(&job_core);
            job_core.child.lock().unwrap().kill().unwrap();
            job_core.child.lock().unwrap().wait().unwrap();
            let deadline = Instant::now() + Duration::from_secs(5);
            loop {
                let system = sysinfo::System::new_all();
                if system.process(pid).is_none() {
                    break;
                }
                assert!(
                    Instant::now() < deadline,
                    "owned core survived helper termination"
                );
                tokio::time::sleep(Duration::from_millis(25)).await;
            }
            assert!(!job_core.alive());
            // A forcibly killed helper cannot run TempDir Drop. Remove only
            // this test's known executable and now-empty private directory.
            std::fs::remove_file(&binary).unwrap();
            std::fs::remove_dir(binary.parent().unwrap()).unwrap();
            drop(job_core);
        }
        assert!(
            servers
                .iter_mut()
                .all(|server| server.child.try_wait().unwrap().is_none()),
            "an unrelated core was terminated"
        );
        drop(pipe_core);
        drop(leases);
        for task in first_hops {
            task.abort();
        }
        target.abort();
        drop(servers);
    });
}

#[test]
#[ignore = "Starts only local VLESS/VMess WS, HTTPUpgrade and gRPC fixtures"]
fn installed_node_transports_reach_only_local_fixture() {
    tauri::async_runtime::block_on(async {
        let origin = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let origin_port = origin.local_addr().unwrap().port();
        let target = tokio::spawn(async move {
            loop {
                let (mut socket, _) = origin.accept().await.unwrap();
                tokio::spawn(async move {
                    let _ = header(&mut socket).await;
                    let _=socket.write_all(b"HTTP/1.1 200 OK\r\nContent-Length: 7\r\nConnection: close\r\n\r\nfixture").await;
                });
            }
        });
        for mode in ["vless", "vmess"] {
            for transport in ["ws", "httpupgrade", "grpc"] {
                let uuid = uuid::Uuid::new_v4().to_string();
                let port = free_port();
                let _server = transport_server(mode, &uuid, port, transport);
                let mut options = node::NodeOptions::default();
                options.transport = transport.into();
                if transport == "grpc" {
                    options.service_name = "fixture".into();
                } else {
                    options.path = "/fixture".into();
                }
                let up = Upstream {
                    config: ProxyConfig {
                        mode: mode.into(),
                        host: "127.0.0.1".into(),
                        port,
                        node: Some(options),
                        ..Default::default()
                    },
                    password: uuid,
                };
                let relay = super::super::start(up, 0).await.unwrap();
                let client = reqwest::Client::builder()
                    .no_proxy()
                    .proxy(reqwest::Proxy::all(format!("http://127.0.0.1:{}", relay.port)).unwrap())
                    .timeout(Duration::from_secs(8))
                    .build()
                    .unwrap();
                assert_eq!(
                    client
                        .get(format!("http://127.0.0.1:{origin_port}/"))
                        .send()
                        .await
                        .unwrap()
                        .text()
                        .await
                        .unwrap(),
                    "fixture",
                    "{mode}/{transport}"
                );
                eprintln!("{mode}/{transport}: passed");
                drop(client);
                drop(relay);
            }
        }
        target.abort();
    });
}
