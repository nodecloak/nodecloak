mod autostart;
mod app_proxy;
pub fn run_app_proxy_helper(suffix:&str,parent:u32)->Result<(),String>{
    #[cfg(any(windows,target_os="macos"))] {tauri::async_runtime::block_on(app_proxy::helper::run(suffix,parent))}
    #[cfg(not(any(windows,target_os="macos")))] {let _=(suffix,parent);Err("应用代理支持 Windows 和 macOS".into())}
}
mod browser_control;
mod browser_sites;
mod claude_probe;
mod clock_probe;
mod cloud;
mod data_root;
mod engine;
mod firefox;
mod firefox_extension;
mod firefox_region;
pub mod firefox_native_host;
mod fingerprint;
mod fonts;
mod platform;
mod profile_commands;
mod profile_diagnostics;
mod profiles;
mod proxy;
pub fn run_proxy_core_stdio()->Result<(),String>{proxy::core::run_stdio()}
mod regional;
mod session_restore;
mod terminal;
#[cfg(any(target_os = "linux", test))]
mod system_languages;
pub mod mcp;
#[cfg(windows)]
mod windows_icons;

#[tauri::command]
fn is_portable_build() -> Result<bool, String> {
    #[cfg(windows)]
    {
        let exe = std::env::current_exe().map_err(|error| error.to_string())?;
        Ok(!exe
            .parent()
            .is_some_and(|folder| folder.join("uninstall.exe").is_file()))
    }
    #[cfg(not(windows))]
    {
        Ok(false)
    }
}

#[tauri::command]
fn supports_in_app_updates(app: tauri::AppHandle) -> bool {
    #[cfg(target_os = "linux")]
    {
        // The Linux updater replaces an AppImage. deb installations are updated
        // through the downloaded package, never by installing an AppImage over it.
        app.env().appimage.map(std::path::PathBuf::from)
            .is_some_and(|path| path.is_absolute() && path.is_file())
    }
    #[cfg(not(target_os = "linux"))]
    { let _ = app; true }
}

#[tauri::command]
fn open_telegram_group() -> Result<(), String> {
    platform::open_telegram_group()
}

#[tauri::command]
fn open_download_page() -> Result<(), String> {
    platform::open_download_page()
}

#[tauri::command]
fn open_firefox_download() -> Result<(), String> {
    platform::open_firefox_download()
}

use std::sync::Mutex;
use tauri::{Manager, State};

#[derive(Default)]
struct OperationLock(Mutex<()>);

#[tauri::command]
async fn scan_environment(
    app: tauri::AppHandle,
    browser: String,
    profile_id: Option<String>,
) -> Result<engine::Scan, String> {
    let (mut local, upstream) = tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        let root = &app.state::<data_root::DataRoot>().0;
        let scoped = profiles::context(root, &browser, profile_id.as_deref())?;
        let scan = engine::scan_local(&scoped, &browser)?;
        let upstream = if let Some(id) = profile_id {
            let p = profiles::get(root, &id, false)?;
            Some(profiles::upstream(&p, profiles::running(root, &p)?)?)
        } else {
            None
        };
        Ok::<_, String>((scan, upstream))
    })
    .await
    .map_err(|e| e.to_string())??;
    if let Some(upstream) = upstream {
        let result = proxy::test(upstream).await;
        match result {
            Ok(test) => {
                if let Some(clock) = test.clock {
                    if let Some(c) = local.checks.iter_mut().find(|c| c.id == "clock") {
                        *c = clock;
                    }
                }
                local.ip = test.ip;
                local.location = test.country;
                local.latency = Some(test.latency);
                for c in &mut local.checks {
                    if c.id == "connection" {
                        c.value = test.target.clone();
                        c.status = "manual".into();
                        c.detail =
                            "通过当前副本代理路径检测；HTTP 状态和实际浏览器结果需分别确认".into();
                    }
                    if c.id == "route" {
                        c.value = local.ip.clone().unwrap_or_else(|| "未读取到出口 IP".into());
                        c.detail = test.connection.clone();
                    }
                }
            }
            Err(message) => {
                local.ip = None;
                local.location = None;
                local.latency = None;
                for c in &mut local.checks {
                    if ["connection", "route"].contains(&c.id.as_str()) {
                        c.status = "warning".into();
                        c.value = "副本网络检测失败".into();
                        c.detail = message.clone();
                    }
                }
            }
        }
        Ok(local)
    } else {
        Ok(engine::scan_network(local).await)
    }
}

#[tauri::command]
async fn check_repair_readiness(
    app: tauri::AppHandle,
    browser: String,
    ids: Vec<String>,
    profile_id: Option<String>,
) -> Result<engine::Readiness, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        let root = &app.state::<data_root::DataRoot>().0;
        if ids.iter().any(|id| id == "timezone") {
            if let Err(message) = profiles::ensure_all_closed(root) {
                return Ok(engine::Readiness {
                    ready: false,
                    message,
                });
            }
        }
        engine::repair_readiness(
            &profiles::context(root, &browser, profile_id.as_deref())?,
            &browser,
            &ids,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}
#[tauri::command]
fn open_timezone_settings() -> Result<(), String> {
    platform::open_timezone_settings()
}

#[tauri::command]
async fn get_timezone_catalog() -> Result<Vec<platform::TimezoneOption>, String> {
    tauri::async_runtime::spawn_blocking(platform::timezone_catalog)
        .await
        .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn get_font_catalog() -> Result<fonts::Catalog, String> {
    tauri::async_runtime::spawn_blocking(fonts::catalog)
        .await
        .map_err(|e| e.to_string())?
}
#[tauri::command]
async fn remove_user_fonts(
    app: tauri::AppHandle,
    browser: String,
    ids: Vec<String>,
    consent: bool,
) -> Result<Vec<fonts::FontOutcome>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        profiles::ensure_all_closed(&app.state::<data_root::DataRoot>().0)?;
        engine::remove_user_fonts(
            &app.state::<data_root::DataRoot>().0,
            &browser,
            &ids,
            consent,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn repair_environment(
    app: tauri::AppHandle,
    browser: String,
    ids: Vec<String>,
    consent_timezone: bool,
    timezone_target: Option<String>,
    consent_fonts: Option<bool>,
    profile_id: Option<String>,
) -> Result<Vec<engine::Outcome>, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        let root = &app.state::<data_root::DataRoot>().0;
        if ids.iter().any(|id| id == "timezone") {
            profiles::ensure_all_closed(root)?;
        }
        let scoped = profiles::context(root, &browser, profile_id.as_deref())?;
        let local: Vec<_> = ids
            .iter()
            .filter(|id| !["timezone", "cli"].contains(&id.as_str()))
            .cloned()
            .collect();
        let global: Vec<_> = ids
            .iter()
            .filter(|id| ["timezone", "cli"].contains(&id.as_str()))
            .cloned()
            .collect();
        let mut output = vec![];
        if !local.is_empty() {
            if let Some(id) = profile_id.as_deref() {
                profiles::apply_pending_preferences(root, id)?;
            }
            output.extend(engine::repair_with_font_consent(
                &scoped,
                &browser,
                &local,
                consent_timezone,
                timezone_target.as_deref().unwrap_or("singapore"),
                consent_fonts.unwrap_or(false),
            )?);
        }
        if let Some(id) = profile_id.as_deref() {
            if output.iter().any(|o| o.success) {
                profiles::sync_preferences(root, id)?;
            }
        }
        if !global.is_empty() {
            output.extend(engine::repair_with_font_consent(
                root,
                &browser,
                &global,
                consent_timezone,
                timezone_target.as_deref().unwrap_or("singapore"),
                consent_fonts.unwrap_or(false),
            )?);
        }
        Ok(output)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
fn get_history(
    app: tauri::AppHandle,
    lock: State<OperationLock>,
    profile_id: Option<String>,
) -> Result<Vec<engine::Record>, String> {
    let _lock = lock.0.lock().map_err(|_| "操作锁不可用".to_string())?;
    profiles::history(&app.state::<data_root::DataRoot>().0, profile_id.as_deref())
}

#[tauri::command]
async fn undo_repair(
    app: tauri::AppHandle,
    record_id: String,
    profile_id: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        let root = &app.state::<data_root::DataRoot>().0;
        let records = profiles::history(root, None)?;
        let record = records
            .iter()
            .find(|r| r.id == record_id && r.profile_id == profile_id)
            .ok_or("修复记录与副本不匹配")?;
        if record.profile_id.is_none() {
            profiles::ensure_all_closed(root)?;
        }
        if let Some(id) = profile_id.as_deref() {
            profiles::apply_pending_preferences(root, id)?;
        }
        engine::undo(
            &profiles::context(root, &record.browser, profile_id.as_deref())?,
            &record_id,
        )?;
        if let Some(id) = profile_id {
            profiles::sync_preferences(root, &id)?;
        }
        Ok(())
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn launch_browser(
    app: tauri::AppHandle,
    browser: String,
    destination: String,
    profile_id: Option<String>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        if let Some(id) = profile_id {
            let p = profiles::get(&app.state::<data_root::DataRoot>().0, &id, false)?;
            if p.browser != browser {
                return Err("副本浏览器类型不匹配".into());
            }
            return profile_commands::launch(
                &app.state::<data_root::DataRoot>().0,
                &id,
                &destination,
                &app.state::<profiles::Runtime>(),
            );
        }
        engine::launch_browser(
            &app.state::<data_root::DataRoot>().0,
            &browser,
            &destination,
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn launch_cli(app: tauri::AppHandle) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        engine::launch_cli(&app.state::<data_root::DataRoot>().0)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn list_terminal_tools() -> Result<terminal::Catalog, String> {
    tauri::async_runtime::spawn_blocking(terminal::catalog)
        .await
        .map_err(|e| e.to_string())
}

#[tauri::command]
async fn get_terminal_environment(app: tauri::AppHandle, tool: terminal::Tool) -> Result<terminal::environment::Environment, String> {
    tauri::async_runtime::spawn_blocking(move || terminal::environment::read(&app.state::<data_root::DataRoot>().0,tool)).await.map_err(|_|"无法读取终端环境".to_string())?
}
#[tauri::command]
async fn save_terminal_environment(app: tauri::AppHandle, tool: terminal::Tool, environment: terminal::environment::Environment) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || terminal::environment::save(&app.state::<data_root::DataRoot>().0,tool,&environment)).await.map_err(|_|"无法保存终端环境".to_string())?
}

#[tauri::command]
fn open_terminal_guide(tool: terminal::Tool) -> Result<(), String> {
    terminal::open_guide(tool)
}

#[tauri::command]
async fn get_system_languages() -> Vec<String> {
    tauri::async_runtime::spawn_blocking(|| {
        #[cfg(target_os = "linux")]
        { system_languages::linux() }
        #[cfg(not(target_os = "linux"))]
        {
        #[cfg(windows)]
        let output = platform::command("powershell.exe")
            .args([
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "(Get-UICulture).Name",
            ])
            .output();
        #[cfg(target_os = "macos")]
        let output = platform::command("/usr/bin/defaults")
            .args(["read", "-g", "AppleLanguages"])
            .output();
        #[cfg(not(any(windows, target_os = "macos")))]
        let output: Result<std::process::Output, std::io::Error> =
            Err(std::io::Error::other("Unsupported platform"));
        output
            .ok()
            .filter(|result| result.status.success())
            .map(|result| {
                String::from_utf8_lossy(&result.stdout)
                    .lines()
                    .filter_map(|line| {
                        let value = line
                            .trim()
                            .trim_matches(|c: char| c == '"' || c == ',' || c.is_whitespace());
                        (value.len() >= 2
                            && value.len() <= 35
                            && value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-'))
                        .then(|| value.to_string())
                    })
                    .collect()
            })
            .unwrap_or_default()
        }
    })
    .await
    .unwrap_or_default()
}

#[tauri::command]
fn set_interface_language(app: tauri::AppHandle, language: String) -> Result<(), String> {
    use tauri::menu::{Menu, MenuItem};
    let (show_text, quit_text, tooltip) = match language.as_str() {
        "zh" => (
            "打开 NodeCloak",
            "退出…",
            "NodeCloak · 浏览器副本与代理保持运行",
        ),
        "en" => (
            "Open NodeCloak",
            "Quit…",
            "NodeCloak · Browser profiles and proxies keep running",
        ),
        _ => return Err("Unsupported interface language".into()),
    };
    let show = MenuItem::with_id(&app, "show", show_text, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let quit = MenuItem::with_id(&app, "quit", quit_text, true, None::<&str>)
        .map_err(|e| e.to_string())?;
    let menu = Menu::with_items(&app, &[&show, &quit]).map_err(|e| e.to_string())?;
    if let Some(tray) = app.tray_by_id("nodecloak-tray") {
        tray.set_menu(Some(menu)).map_err(|e| e.to_string())?;
        tray.set_tooltip(Some(tooltip)).map_err(|e| e.to_string())?;
    }
    Ok(())
}

#[tauri::command]
async fn launch_terminal(
    app: tauri::AppHandle,
    tool: terminal::Tool,
    working_directory: String,
    environment: Option<terminal::environment::Environment>,
) -> Result<(), String> {
    tauri::async_runtime::spawn_blocking(move || {
        let guard = app.state::<OperationLock>();
        let _lock = guard.0.lock().map_err(|_| "操作锁不可用".to_string())?;
        terminal::launch(
            &app.state::<data_root::DataRoot>().0,
            tool,
            &working_directory,
            environment.unwrap_or_default(),
        )
    })
    .await
    .map_err(|e| e.to_string())?
}

#[tauri::command]
async fn select_terminal_directory(
    window: tauri::Window,
    working_directory: String,
    title: String,
) -> Result<Option<String>, String> {
    use tauri_plugin_dialog::DialogExt;
    tauri::async_runtime::spawn_blocking(move || {
        let current = std::path::PathBuf::from(working_directory.trim());
        let initial = if current.is_absolute() && current.is_dir() {
            current
        } else {
            platform::home_dir()
        };
        window
            .dialog()
            .file()
            .set_parent(&window)
            .set_title(title)
            .set_directory(initial)
            .blocking_pick_folder()
            .map(|selected| {
                selected
                    .into_path()
                    .map(|path| path.to_string_lossy().into_owned())
                    .map_err(|error| error.to_string())
            })
            .transpose()
    })
    .await
    .map_err(|error| error.to_string())?
}

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_updater::Builder::new().build())
        .plugin(tauri_plugin_process::init())
        .plugin(tauri_plugin_autostart::Builder::new().app_name(autostart::entry_name()).build())
        .plugin(tauri_plugin_single_instance::init(|app, _, _| {
            if let Some(window) = app.get_webview_window("main") {
                let _ = window.show();
                let _ = window.unminimize();
                let _ = window.set_focus();
            }
        }))
        .manage(OperationLock::default())
        .manage(app_proxy::runtime::Runtime::default())
        .manage(autostart::StartupState::default())
        .manage(mcp::Runtime::default())
        .manage(profiles::Runtime::default())
        .manage(cloud::commands::Runtime::default())
        .manage(cloud::sharing::Runtime::default())
        .setup(|app| {
            let root = data_root::resolve(app.path().app_data_dir()?);
            #[cfg(debug_assertions)]
            let root = if let Ok(path)=std::env::var("NODECLOAK_TEST_DATA_ROOT"){let path=std::path::PathBuf::from(path).canonicalize()?;profiles::guard_tree(&path)?;path}else{root};
            profiles::load(&root)?;
            app.manage(data_root::DataRoot(root.clone()));
            if let Err(error)=autostart::initialize_app(app.handle()){
                eprintln!("NodeCloak autostart initialization failed: {error}");
            }
            mcp::start(app.handle().clone());
            cloud::scheduler::start(app.handle().clone());
            cloud::sharing::start(app.handle().clone());
            let handle = app.handle().clone();
            tauri::async_runtime::spawn_blocking(move || {
                let guard = handle.state::<OperationLock>();
                if let Ok(_lock) = guard.0.lock() {
                    if let Ok(list) = profiles::load(&root) {
                        for p in list {
                            if p.received_share.is_some(){continue;}
                            if p.deleted_at.is_none()
                                && profiles::running(&root, &p).unwrap_or(false)
                            {
                                let _ = tauri::async_runtime::block_on(profiles::prepare_relay(
                                    &root,
                                    &p,
                                    &handle.state::<profiles::Runtime>(),
                                    true,
                                ));
                            }
                        }
                    }
                };
            });
            use tauri::{
                menu::{Menu, MenuItem},
                tray::TrayIconBuilder,
            };
            let show = MenuItem::with_id(app, "show", "打开 NodeCloak", true, None::<&str>)?;
            let quit = MenuItem::with_id(app, "quit", "退出…", true, None::<&str>)?;
            let menu = Menu::with_items(app, &[&show, &quit])?;
            #[cfg(windows)]
            let tray_icon = windows_icons::initial_tray_icon();
            #[cfg(not(windows))]
            let tray_icon = app.default_window_icon().unwrap().clone();
            TrayIconBuilder::with_id("nodecloak-tray")
                .icon(tray_icon)
                .tooltip("NodeCloak · 浏览器副本与代理保持运行")
                .menu(&menu)
                .on_menu_event(|app, event| {
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.unminimize();
                        let _ = window.set_focus();
                        if event.id.as_ref() == "quit" {
                            use tauri::Emitter;
                            let _ = app.emit("request-quit", ());
                        }
                    }
                })
                .build(app)?;
            #[cfg(windows)]
            windows_icons::start(app.handle())?;
            Ok(())
        })
        .invoke_handler(tauri::generate_handler![
            supports_in_app_updates,
            app_proxy::commands::application_proxy_action,
            autostart::get_autostart_enabled,
            autostart::set_autostart_enabled,
            mcp::mcp_status,
            mcp::mcp_action,
            profile_commands::generate_fingerprint_seed,
            cloud::sharing::sharing_action,
            browser_sites::get_browser_site_catalog,
            browser_sites::get_browser_site_icon,
            browser_sites::open_catalog_support,
            browser_sites::open_announcement_link,
            scan_environment,
            check_repair_readiness,
            open_timezone_settings,
            get_timezone_catalog,
            open_telegram_group,
            is_portable_build,
            open_download_page,
            open_firefox_download,
            get_font_catalog,
            remove_user_fonts,
            repair_environment,
            get_history,
            undo_repair,
            launch_browser,
            launch_cli,
            list_terminal_tools,
            get_terminal_environment,
            save_terminal_environment,
            select_terminal_directory,
            launch_terminal,
            open_terminal_guide,
            set_interface_language,
            get_system_languages,
            profile_commands::list_profiles,
            profile_commands::read_profile_diagnostics,
            profile_commands::save_profile,
            profile_commands::reset_profile,
            profile_commands::delete_profile,
            profile_commands::restore_profile,
            profile_commands::test_profile_proxy,
            profile_commands::match_profile_region,
            profile_commands::start_profile,
            profile_commands::close_profile,
            profile_commands::restart_profile,
            profile_commands::prepare_application_update,
            profile_commands::quit_application,
            cloud::commands::cloud_status,
            cloud::commands::cloud_action
        ])
        .on_window_event(|window, event| {
            if let tauri::WindowEvent::CloseRequested { api, .. } = event {
                api.prevent_close();
                let _ = window.hide();
            }
        })
        .build(tauri::generate_context!())
        .expect("Failed to start NodeCloak")
        .run(|app, event| {
            if let tauri::RunEvent::ExitRequested { code, api, .. } = event {
                if code.is_none() {
                    api.prevent_exit();
                    if let Some(window) = app.get_webview_window("main") {
                        let _ = window.show();
                        let _ = window.set_focus();
                    }
                    use tauri::Emitter;
                    let _ = app.emit("request-quit", ());
                }
            }
        });
}
