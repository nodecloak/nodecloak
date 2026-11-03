#[cfg(windows)]
pub async fn run(suffix: &str, parent: u32) -> Result<(), String> {
    use super::{
        config,
        ipc::{self, Control, Reply, StartRequest},
        windows::{network, process, security::PrivateCore, transport},
    };
    use std::{
        sync::{
            atomic::{AtomicBool, Ordering},
            Arc,
        },
        time::Duration,
    };
    let (mut pipe, parent_handle) = transport::open_parent(suffix, parent).await?;
    let request: StartRequest =
        tokio::time::timeout(Duration::from_secs(10), ipc::read_frame(&mut pipe))
            .await
            .map_err(|_| "等待应用代理配置超时")??;
    // Validate before any privileged filesystem or network operation.
    let validation = (|| {
        let core_config = config::compile(
            &request.rules,
            &request.proxies,
            request.controller_port,
            &request.controller_secret,
            &request.session,
        )?;
        let interface = config::interface_name(&request.session)?;
        let ownership = process::ownership_mutex()?;
        network::preflight(&network::snapshot()?, &interface)?;
        Ok::<_, String>((core_config, interface, ownership))
    })();
    let (mut core_config, interface, _ownership) = match validation {
        Ok(valid) => valid,
        Err(message) => {
            ipc::write_frame(
                &mut pipe,
                &Reply::Failure {
                    message: message.clone(),
                },
            )
            .await
            .ok();
            return Err(message);
        }
    };
    let cancel = Arc::new(AtomicBool::new(false));
    let control_cancel = cancel.clone();
    let (mut input, mut output) = tokio::io::split(pipe);
    let control = tokio::spawn(async move {
        let _ = ipc::read_frame::<_, Control>(&mut input).await;
        control_cancel.store(true, Ordering::Release);
    });
    struct Reader(tokio::task::JoinHandle<()>);
    impl Drop for Reader {
        fn drop(&mut self) {
            self.0.abort();
        }
    }
    let _reader = Reader(control);
    let worker_cancel = cancel.clone();
    let worker_interface = interface.clone();
    let mut secrets: Vec<String> = request
        .proxies
        .iter()
        .map(|p| p.password.clone())
        .filter(|p| !p.is_empty())
        .collect();
    secrets.push(request.controller_secret.clone());
    let start = tokio::task::spawn_blocking(move || {
        let files = PrivateCore::prepare(&worker_cancel)?;
        network::preflight(&network::snapshot()?, &worker_interface)?;
        core_config["route"]["rules"][0]["process_path_regex"]
            .as_array_mut()
            .ok_or("应用代理配置无效")?
            .push(serde_json::json!(config::path_pattern(
                &files.binary.to_string_lossy()
            )));
        let bytes = serde_json::to_vec(&core_config).map_err(|_| "无法生成应用代理配置")?;
        let (core, stderr) = process::CoreProcess::start(files, bytes, worker_cancel)?;
        let signal = process::watch_output(stderr, secrets);
        Ok::<_, String>((core, signal))
    });
    let result: Result<(), String> = async {
        let (mut core, signal) = start.await.map_err(|_| "应用代理内核准备任务中断")??;
        let deadline = tokio::time::Instant::now() + Duration::from_secs(25);
        let client = reqwest::Client::builder()
            .no_proxy()
            .redirect(reqwest::redirect::Policy::none())
            .timeout(Duration::from_secs(1))
            .build()
            .map_err(|_| "无法检查应用代理接口")?;
        let mut luid = None;
        let mut started = false;
        let outcome = async {
            while tokio::time::Instant::now() < deadline
                && !cancel.load(Ordering::Acquire)
                && parent_handle.alive()
                && core.alive()
            {
                luid = network::owned_luid(&interface)?.or(luid);
                if signal.ready.load(Ordering::Acquire) {
                    luid = network::interface_luid(&interface)?;
                    if luid.is_some()
                        && client
                            .get(format!(
                                "http://127.0.0.1:{}/version",
                                request.controller_port
                            ))
                            .bearer_auth(&request.controller_secret)
                            .send()
                            .await
                            .is_ok_and(|r| r.status().is_success())
                    {
                        ipc::write_frame(
                            &mut output,
                            &Reply::Ready {
                                interface: interface.clone(),
                                luid: luid.unwrap(),
                            },
                        )
                        .await?;
                        started = true;
                        break;
                    }
                }
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            if !started {
                let error = signal
                    .error
                    .lock()
                    .ok()
                    .and_then(|e| e.clone())
                    .unwrap_or_else(|| {
                        if cancel.load(Ordering::Acquire) || !parent_handle.alive() {
                            "应用代理启动已取消".into()
                        } else {
                            format!("应用代理内核或接口未就绪（退出码 {}）", core.exit_code())
                        }
                    });
                return Err(error);
            }
            while !cancel.load(Ordering::Acquire) && parent_handle.alive() && core.alive() {
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
            let unexpected = !cancel.load(Ordering::Acquire) && parent_handle.alive();
            if unexpected {
                return Err(signal
                    .error
                    .lock()
                    .ok()
                    .and_then(|e| e.clone())
                    .unwrap_or_else(|| {
                        format!(
                            "应用代理内核意外退出（代码 {}），已停止应用代理",
                            core.exit_code()
                        )
                    }));
            }
            Ok(())
        }
        .await;
        // Capture even a not-yet-UP interface before stopping the core. All
        // paths (including failed Ready writes and cancellation) use one teardown.
        luid = network::owned_luid(&interface)?.or(luid);
        core.stop()?;
        wait_released(&interface, luid).await?;
        outcome?;
        ipc::write_frame(&mut output, &Reply::Stopped).await.ok();
        Ok(())
    }
    .await;
    if let Err(message) = &result {
        ipc::write_frame(
            &mut output,
            &Reply::Failure {
                message: message.clone(),
            },
        )
        .await
        .ok();
    }
    result
}
#[cfg(windows)]
async fn wait_released(interface: &str, luid: Option<u64>) -> Result<(), String> {
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(5);
    while tokio::time::Instant::now() < deadline {
        if super::windows::network::session_released(interface, luid)? {
            return Ok(());
        }
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
    }
    Err("应用代理接口仍在释放，请重试停止并查看诊断记录".into())
}
