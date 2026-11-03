use crate::{
    browser_control::LaunchPurpose,
    data_root, engine, platform,
    profiles::{self, Draft, Profile, ProfileView, Runtime},
    proxy::{self, ProxyTest},
    OperationLock,
};
use std::path::Path;
use tauri::Manager;

#[tauri::command]
pub fn generate_fingerprint_seed() -> Result<String, String> {
    crate::fingerprint::seed()
}

async fn operation<T: Send + 'static>(
    app: tauri::AppHandle,
    action: impl FnOnce(&Path, &Runtime) -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用")?;
        action(
            &app.state::<data_root::DataRoot>().0,
            &app.state::<Runtime>(),
        )
    })
    .await
    .map_err(|_| "副本操作未完成")?
}
#[tauri::command]
pub async fn list_profiles(app: tauri::AppHandle) -> Result<Vec<ProfileView>, String> {
    operation(app, |root, runtime| profiles::views(root, runtime)).await
}
#[tauri::command]
pub async fn read_profile_diagnostics(app: tauri::AppHandle, id:String) -> Result<crate::profile_diagnostics::Report,String> {
    let root=app.state::<data_root::DataRoot>().0.clone();
    // Viewing startup progress must not wait for the launch operation lock.
    tauri::async_runtime::spawn_blocking(move||crate::profile_diagnostics::read(&root,&id))
        .await.map_err(|_|"无法读取副本启动日志")?
}
#[tauri::command]
pub async fn save_profile(
    app: tauri::AppHandle,
    id: Option<String>,
    draft: Draft,
    copy_id: Option<String>,
) -> Result<Profile, String> {
    operation(app, move |root, _| {
        if let Some(id) = id {
            profiles::update(root, &id, draft)
        } else {
            profiles::create(root, draft, copy_id.as_deref())
        }
    })
    .await
}
#[tauri::command]
pub async fn delete_profile(app: tauri::AppHandle, id: String, purge: bool) -> Result<(), String> {
    operation(app, move |root, runtime| {
        crate::cloud::sharing::editable(root, &id)?;
        profiles::remove(root, &id, purge, runtime)
    })
    .await
}
#[tauri::command]
pub async fn reset_profile(app: tauri::AppHandle,id: String,reset_proxy: bool) -> Result<(),String> {
    operation(app,move |root,runtime|profiles::reset::reset(root,&id,reset_proxy,runtime)).await
}
#[tauri::command]
pub async fn restore_profile(app: tauri::AppHandle, id: String) -> Result<(), String> {
    operation(app, move |root, _| {
        crate::cloud::sharing::editable(root, &id)?;
        profiles::restore(root, &id)
    })
    .await
}
#[tauri::command]
pub async fn test_profile_proxy(
    app: tauri::AppHandle,
    id: Option<String>,
    draft: Option<Draft>,
) -> Result<ProxyTest, String> {
    operation(app, move |root, _| {
        let up = if let Some(draft) = &draft {
            profiles::draft_upstream(root, id.as_deref(), draft)?
        } else {
            let p = profiles::get(root, id.as_deref().ok_or("请选择副本")?, false)?;
            profiles::upstream(&p, profiles::running(root, &p)?)?
        };
        let result = tauri::async_runtime::block_on(proxy::test(up))?;
        if draft.is_none() {
            if let Some(id) = &id {
                let p = profiles::get(root, id, false)?;
                if p.active_proxy
                    .as_ref()
                    .is_none_or(|active| active == &p.proxy)
                {
                    profiles::remember_test(root, id, result.clone())?;
                }
            }
        }
        Ok(result)
    })
    .await
}
#[tauri::command]
pub async fn match_profile_region(
    app: tauri::AppHandle,
    id: Option<String>,
    draft: Draft,
) -> Result<crate::regional::IpRegion, String> {
    operation(app, move |root, _| {
        let up = profiles::draft_upstream(root, id.as_deref(), &draft)?;
        tauri::async_runtime::block_on(crate::regional::lookup(up))
    })
    .await
}
pub fn launch(root: &Path, id: &str, destination: &str, runtime: &Runtime) -> Result<(), String> {
    launch_with_lookup(root, id, destination, runtime, crate::regional::LOOKUP_URL)
}
pub(crate) fn launch_with_lookup(
    root: &Path,
    id: &str,
    destination: &str,
    runtime: &Runtime,
    lookup_url: &str,
) -> Result<(), String> {
    launch_with_purpose(root, id, destination, runtime, lookup_url, LaunchPurpose::Manual)
}
pub(crate) fn launch_for_automation(
    root: &Path, id: &str, destination: &str, runtime: &Runtime,
) -> Result<(), String> {
    launch_with_purpose(root, id, destination, runtime, crate::regional::LOOKUP_URL, LaunchPurpose::Mcp)
}
fn launch_with_purpose(
    root: &Path, id: &str, destination: &str, runtime: &Runtime,
    lookup_url: &str, purpose: LaunchPurpose,
) -> Result<(), String> {
    let automation = purpose == LaunchPurpose::Mcp;
    crate::cloud::sharing::before_launch(root, id)?;
    let p = profiles::get(root, id, false)?;
    let path = profiles::path(root, &p)?;
    crate::cloud::browser_session::recover(&path)?;
    if profiles::running(root, &p)? {
        if automation && !runtime.1.automation_ready(id) {
            return Err("RESTART_REQUIRED: 此副本未启用 MCP 自动化，请关闭后通过 MCP 启动".into());
        }
        if destination == "startup" || destination == "focus" {
            if crate::regional::manual_needs_control_for(
                &p.browser,
                p.active_preferences.as_ref().unwrap_or(&p.preferences),
            ) && !runtime.1.ready(id)
            {
                return Err("此副本的区域设置控制器未连接，请关闭并重新启动".into());
            }
            return platform::focus_profile(&path);
        }
        if runtime.1.ready(id) || crate::regional::manual_needs_control_for(
            &p.browser,
            p.active_preferences.as_ref().unwrap_or(&p.preferences),
        ) {
            return runtime
                .1
                .open(id, destination_url(root, &p, destination)?, true)
                .map_err(|error| error.to_string());
        }
        if p.browser == "firefox" {
            return Err("此 Firefox 副本正在运行。请先关闭再打开检测页，或在该窗口访问 https://claudedone.com/check/。".into());
        }
        let port =
            tauri::async_runtime::block_on(profiles::prepare_relay(root, &p, runtime, true))?;
        let url = destination_url(root, &p, destination)?;
        return platform::spawn_browser_with_proxy(
            &p.browser,
            &path,
            &url,
            &p.active_proxy.as_ref().unwrap_or(&p.proxy).mode,
            port,
        );
    }
    crate::cloud::session_sync::before_launch(root, id)?;
    let p = profiles::get(root, id, false)?;
    runtime.1.stop(id);
    if p.browser=="firefox" {crate::session_restore::FirefoxSession::recover(&path)?;}
    let restore_firefox = !automation && destination=="startup";
    let mut firefox_session=if p.browser=="firefox" {
        crate::session_restore::FirefoxSession::park(&path)?
    }else{None};
    let resume = !automation && destination=="startup" && p.browser!="firefox"
        && if crate::session_restore::uses_tab_urls(&p.browser) && crate::regional::manual_needs_control_for(&p.browser,&p.preferences) { !crate::session_restore::read_tabs(&path)?.is_empty() }
            else {crate::session_restore::chromium_available(&path)?};
    let port = tauri::async_runtime::block_on(profiles::prepare_relay(root, &p, runtime, false))?;
    let use_extension = !automation && crate::firefox_extension::eligible(&p.browser, &p.preferences);
    if p.browser == "firefox" {
        crate::firefox_region::recover(&path)?;
        if let Err(error) = crate::firefox_extension::prepare(root, &path, &p.preferences, use_extension, &p.proxy.mode, port) {
            runtime.0.lock().map_err(|_| "代理管理器不可用")?.remove(id);
            return Err(error);
        }
    }
    if automation || crate::regional::manual_needs_control_for(&p.browser, &p.preferences) {
        let mut keep_browser = false;
        let result = (|| -> Result<(), String> {
            let mut resolved = crate::regional::initial(&p.preferences)?;
            if p.preferences.regional.language_mode == "ip" {
                if let Some(last) = &p.last_region {
                    resolved.language = last.language.clone();
                }
            }
            for attempt in 0..2 {
                let mut planned = p.clone();
                if p.preferences.regional.language_mode == "ip" {
                    planned.preferences.language = resolved.language.clone();
                    planned.preferences_managed = true;
                    planned.preferences_dirty = true;
                }
                profiles::apply_preferences_for_control(root, &planned,automation)?;
                let mut provider_region = resolved.clone();
                if p.browser == "firefox" {
                    if p.preferences.regional.location_mode == "custom"
                        && p.preferences.advanced.location != profiles::PermissionMode::Block
                    {
                        provider_region.latitude = Some(p.preferences.regional.latitude);
                        provider_region.longitude = Some(p.preferences.regional.longitude);
                    }
                    crate::firefox::prepare_geolocation(&path, Some(&provider_region))?;
                }
                if automation {
                    platform::spawn_controlled_browser(&p.browser, &path, &p.proxy.mode, port)?;
                } else {
                    platform::spawn_settings_browser(&p.browser, &path, &p.proxy.mode, port)?;
                }
                let (control, actual) = crate::browser_control::start_with_lookup(
                    p.browser.clone(),
                    path.clone(),
                    p.preferences.clone(),
                    resolved.clone(),
                    lookup_url.into(),
                    purpose,
                )?;
                if (p.preferences.regional.language_mode == "ip"
                    && actual.language != resolved.language)
                    || (p.browser == "firefox"
                        && p.preferences.regional.location_mode == "ip"
                        && p.preferences.advanced.location != profiles::PermissionMode::Block
                        && (actual.latitude != provider_region.latitude
                            || actual.longitude != provider_region.longitude
                            || actual.accuracy != provider_region.accuracy))
                {
                    control.quit_before_navigation()?;
                    platform::close_profile(&path)?;
                    drop(control);
                    if attempt == 1 {
                        return Err(
                            "浏览器出口地区持续变化，无法稳定匹配区域，请固定代理或使用自定义设置"
                                .into(),
                        );
                    }
                    resolved = actual;
                    continue;
                }
                let restored = resume && control.resume_previous()?;
                runtime.1.insert(id.to_owned(), control)?;
                profiles::mark_started(root, id, port, runtime.proxy_endpoint(id)?)?;
                profiles::remember_region(root, id, actual)?;
                if restored {return Ok(());}
                return match runtime
                    .1
                    .open(id, destination_url(root, &p, destination)?, false)
                {
                    Err(crate::browser_control::OpenError::Navigation(message) | crate::browser_control::OpenError::Preparation(message)) => {
                        keep_browser = true;
                        Err(message)
                    }
                    Err(error) => Err(error.to_string()),
                    Ok(()) => Ok(()),
                };
            }
            Err("区域设置准备失败".into())
        })();
        if result.is_err() && !keep_browser {
            let _ = platform::close_profile(&path);
            runtime.1.stop(id);
            runtime.0.lock().map_err(|_| "代理管理器不可用")?.remove(id);
        }
        if result.is_ok() || keep_browser {
            if let Some(saved)=firefox_session.as_mut(){
                if let Err(error)=saved.finish_without_restore(){crate::browser_control::log_startup_endpoint(&path,&p.browser,"session_backup_pending",&error);}
            }
        }
        return result;
    }
    profiles::apply_preferences(root, &p)?;
    if p.browser == "firefox" {
        crate::firefox::prepare_geolocation(&path, None)?;
    }
    let url = destination_url(root, &p, destination)?;
    profiles::mark_started(root, id, port, runtime.proxy_endpoint(id)?)?;
    let use_native_region = p.browser == "firefox" && (crate::firefox_region::required(&p.preferences)||(restore_firefox&&firefox_session.is_some()));
    let result = if let Some(saved)=firefox_session.as_mut().filter(|_|restore_firefox) {
        crate::firefox_region::launch_resuming(&path,&url,&p.preferences,&p.proxy.mode,port,lookup_url,saved)
            .and_then(|region|profiles::remember_region(root,id,region))
    }else if use_native_region {
        crate::firefox_region::launch(&path, &url, &p.preferences, &p.proxy.mode, port, lookup_url)
            .and_then(|region| profiles::remember_region(root, id, region))
    } else if use_extension {
        crate::firefox_extension::launch(&path, &url, &p.proxy.mode, port)
    } else {
        if resume {platform::spawn_resuming_browser(&p.browser,&path,&p.proxy.mode,port)}
        else {platform::spawn_browser_with_proxy(&p.browser, &path, &url, &p.proxy.mode, port)}
    };
    if let Err(error) = result {
        if use_extension || use_native_region { let _ = platform::close_profile(&path); }
        runtime.0.lock().map_err(|_| "代理管理器不可用")?.remove(id);
        return Err(error);
    }
    if !restore_firefox {
        if let Some(saved)=firefox_session.as_mut(){
            if let Err(error)=saved.finish_without_restore(){crate::browser_control::log_startup_endpoint(&path,&p.browser,"session_backup_pending",&error);}
        }
    }
    Ok(())
}
fn destination_url(root: &Path, p: &Profile, destination: &str) -> Result<String, String> {
    if let Some(url) = crate::browser_sites::destination_url(root, destination) {
        return Ok(url.into());
    }
    Ok(match destination {
        "startup" => p.preferences.startup_url.clone(),
        // Native configuration fixtures request their new page explicitly;
        // ordinary Start now intentionally restores the previous session.
        #[cfg(test)]
        "__fixture_startup" => p.preferences.startup_url.clone(),
        "claude" => "https://claude.ai".into(),
        "verify" => format!(
            "https://claudedone.com/check/?profile={}&browser={}",
            p.id, p.browser
        ),
        "dns" => {
            if p.browser == "firefox" {
                "about:preferences#privacy".into()
            } else {
                "chrome://settings/security".into()
            }
        }
        "privacyDocs" => "https://code.claude.com/docs/en/data-usage".into(),
        "diagnostics" => {
            let file = profiles::path(root, p)?.join("browser-check.html");
            let page = include_str!("../resources/browser-check.html")
                .replace("__CLAUDE_READY_BROWSER__", &p.browser)
                .replace("__CLAUDE_DONE_PROFILE__", &p.id);
            engine::atomic_write(&file, page.as_bytes())?;
            reqwest::Url::from_file_path(file)
                .map_err(|_| "检测页路径无效")?
                .to_string()
        }
        _ => return Err("不支持的启动页面".into()),
    })
}

#[cfg(test)]
mod website_shortcut_tests {
    use super::*;

    #[test]
    fn website_shortcuts_keep_startup_and_profile_diagnostics_independent() {
        let root = tempfile::tempdir_in(std::env::temp_dir().canonicalize().unwrap()).unwrap();
        let draft: Draft = serde_json::from_value(serde_json::json!({
            "name": "Shopping", "browser": "chrome",
            "proxy": {"mode": "system", "host": "", "port": 0, "username": ""}
        }))
        .unwrap();
        let profile = profiles::create(root.path(), draft, None).unwrap();
        let startup = profile.preferences.startup_url.clone();
        assert_eq!(
            destination_url(root.path(), &profile, "site:chatgpt").unwrap(),
            "https://chatgpt.com/"
        );
        assert_eq!(
            destination_url(root.path(), &profile, "site:amazon").unwrap(),
            "https://www.amazon.com/"
        );
        assert_eq!(
            destination_url(root.path(), &profile, "startup").unwrap(),
            startup
        );
        assert!(destination_url(root.path(), &profile, "verify")
            .unwrap()
            .contains(&format!("profile={}", profile.id)));
        assert!(destination_url(root.path(), &profile, "site:unknown").is_err());
    }
}
#[tauri::command]
pub async fn start_profile(app: tauri::AppHandle, id: String) -> Result<(), String> {
    operation(app, move |root, runtime| {
        launch(root, &id, "startup", runtime)
    })
    .await
}
#[tauri::command]
pub async fn close_profile(app: tauri::AppHandle, id: String) -> Result<(), String> {
    operation(app, move |root, runtime| {
        let p = profiles::get(root, &id, false)?;
        platform::close_profile(&profiles::path(root, &p)?)?;
        runtime.1.stop(&id);
        runtime
            .0
            .lock()
            .map_err(|_| "代理管理器不可用")?
            .remove(&id);
        profiles::views(root, runtime)?;
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn restart_profile(app: tauri::AppHandle, id: String) -> Result<(), String> {
    operation(app, move |root, runtime| {
        let p = profiles::get(root, &id, false)?;
        platform::close_profile(&profiles::path(root, &p)?)?;
        runtime.1.stop(&id);
        runtime
            .0
            .lock()
            .map_err(|_| "代理管理器不可用")?
            .remove(&id);
        launch(root, &id, "startup", runtime)
    })
    .await
}
#[tauri::command]
pub async fn prepare_application_update(app: tauri::AppHandle) -> Result<(), String> {
    crate::app_proxy::commands::stop_before_exit(&app).await?;
    operation(app, |root, runtime| {
        for p in profiles::load(root)? {
            if profiles::running(root, &p)? {
                if p.received_share.is_some() {
                    platform::force_close_profile(&profiles::path(root, &p)?)?;
                } else {
                    platform::close_profile(&profiles::path(root, &p)?)?;
                }
            }
        }
        runtime.0.lock().map_err(|_| "代理管理器不可用")?.clear();
        runtime.1.clear();
        Ok(())
    })
    .await
}
#[tauri::command]
pub async fn quit_application(app: tauri::AppHandle, close_profiles: bool) -> Result<(), String> {
    crate::app_proxy::commands::stop_before_exit(&app).await?;
    let closing = app.clone();
    operation(app, move |root, runtime| {
        let active = profiles::load(root)?
            .into_iter()
            .filter(|p| profiles::running(root, p).unwrap_or(true))
            .collect::<Vec<_>>();
        if !active.is_empty() && !close_profiles {
            return Err("仍有浏览器副本运行，请选择关闭副本后退出，或继续后台运行。".into());
        }
        for p in active {
            if p.received_share.is_some() {
                platform::force_close_profile(&profiles::path(root, &p)?)?;
            } else {
                platform::close_profile(&profiles::path(root, &p)?)?;
            }
        }
        runtime.0.lock().map_err(|_| "代理管理器不可用")?.clear();
        runtime.1.clear();
        Ok(())
    })
    .await?;
    closing.exit(0);
    Ok(())
}
