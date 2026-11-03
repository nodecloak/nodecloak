use super::*;
use serde_json::json;
use std::{path::PathBuf, thread};
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
        let target = tokio::spawn(async move {
            loop {
                let (mut stream, _) = origin.accept().await.unwrap();
                let hits = origin_hits.clone();
                let requests = requests.clone();
                tokio::spawn(async move {
                    let bytes = header(&mut stream).await.unwrap();
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
                assert!(paths.lock().unwrap().contains(&path),"{browser} did not use {mode}/{first_mode} to reach an unresolvable fixture hostname");
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
