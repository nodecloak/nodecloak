use super::{
    model::*,
    runtime::Runtime,
    store::{self, Credentials, NativeCredentials},
};
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::Manager;
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct NodeId {
    id: String,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Rules {
    rules: Vec<Rule>,
}
fn decode<T: serde::de::DeserializeOwned>(value: Value) -> Result<T, String> {
    serde_json::from_value(value).map_err(|_| "应用代理参数无效".into())
}

#[tauri::command]
pub async fn application_proxy_action(
    app: tauri::AppHandle,
    action: String,
    payload: Value,
) -> Result<Value, String> {
    let runtime = app.state::<Runtime>();
    let root = app.state::<crate::data_root::DataRoot>().0.clone();
    if action == "read" {
        return serde_json::to_value(runtime.view(&root)?)
            .map_err(|_| "无法读取应用代理状态".into());
    }
    if !cfg!(any(windows, target_os = "macos")) {
        return Err("应用代理支持 Windows 和 macOS".into());
    }
    match action.as_str() {
        "discover" => runtime.discover().await?,
        "start" => {
            if let Err(error) = runtime.start(&root).await {
                runtime.rejected_start(&root, &error);
                return Err(error);
            }
        }
        "stop" => runtime.stop(&root).await?,
        "saveNode" => {
            let _lock = runtime.gate.lock().await;
            runtime.editable()?;
            store::save_node(&root, decode(payload)?, &NativeCredentials)?;
        }
        "deleteNode" => {
            let _lock = runtime.gate.lock().await;
            runtime.editable()?;
            store::delete_node(&root, &decode::<NodeId>(payload)?.id, &NativeCredentials)?;
        }
        "saveRules" => {
            let _lock = runtime.gate.lock().await;
            runtime.editable()?;
            store::save_rules(&root, decode::<Rules>(payload)?.rules)?;
        }
        "pickExecutable" => {
            runtime.editable()?;
            use tauri_plugin_dialog::DialogExt;
            return tokio::task::spawn_blocking(move || {
                let dialog = app.dialog().file();
                #[cfg(target_os = "macos")]
                let path = dialog
                    .add_filter("Mac application", &["app"])
                    .blocking_pick_file();
                #[cfg(not(target_os = "macos"))]
                let path = dialog
                    .add_filter("Windows application", &["exe"])
                    .blocking_pick_file();
                let Some(path) = path else {
                    return Ok(Value::Null);
                };
                let path = path
                    .into_path()
                    .map_err(|_| "请选择本机程序文件")?
                    .canonicalize()
                    .map_err(|_| "无法读取选择的程序文件")?;
                let path = path
                    .to_string_lossy()
                    .strip_prefix(r"\\?\")
                    .unwrap_or(&path.to_string_lossy())
                    .to_owned();
                if cfg!(target_os = "macos") {
                    if !super::macos_bundle::valid_selection(&path) {
                        return Err("请选择 Mac 应用（.app）".into());
                    }
                    super::macos_bundle::executables(std::path::Path::new(&path))?;
                } else if !super::config::valid_path(&path) {
                    return Err(
                        "请选择本机应用程序；不能选择 NodeCloak、代理内核或系统共享进程".into(),
                    );
                }
                Ok(json!(path))
            })
            .await
            .map_err(|_| "选择应用程序的任务中断")?;
        }
        "testNode" => {
            let id = decode::<NodeId>(payload)?.id;
            let settings = store::load(&root)?;
            let node = settings
                .proxies
                .iter()
                .find(|p| p.id == id)
                .ok_or("此应用代理节点已不存在")?;
            let password = if let Some(reference) = &node.proxy.credential_ref {
                NativeCredentials.get(&node.id, reference)?
            } else {
                String::new()
            };
            let result = crate::proxy::test(crate::proxy::Upstream {
                config: node.proxy.clone(),
                password,
            })
            .await?;
            return serde_json::to_value(result).map_err(|_| "无法读取代理检测结果".into());
        }
        _ => return Err("未知应用代理操作".into()),
    }
    serde_json::to_value(runtime.view(&root)?).map_err(|_| "无法读取应用代理状态".into())
}
pub async fn stop_before_exit(app: &tauri::AppHandle) -> Result<(), String> {
    let root = app.state::<crate::data_root::DataRoot>().0.clone();
    app.state::<Runtime>().stop(&root).await
}
