use super::account::Connection;
use reqwest::Method;
use serde::Deserialize;
use serde_json::{json, Value};
use tauri::{Emitter, Manager};
use uuid::Uuid;

#[derive(Deserialize)]
#[serde(
    tag = "type",
    rename_all = "camelCase",
    rename_all_fields = "camelCase",
    deny_unknown_fields
)]
pub enum Action {
    List,
    Create {
        name: String,
    },
    Invite {
        team_id: Uuid,
        name: String,
        email: String,
    },
    Accept {
        team_id: Uuid,
        member_id: Uuid,
    },
    Remove {
        team_id: Uuid,
        member_id: Uuid,
    },
}

#[tauri::command]
pub async fn team_action(
    app: tauri::AppHandle,
    account_id: Uuid,
    action: Action,
) -> Result<Value, String> {
    tauri::async_runtime::spawn_blocking(move || {
        let root = app.state::<crate::data_root::DataRoot>();
        let mut connection = Connection::load(&root.0)?;
        if connection.account_id()? != account_id.to_string() {
            return Err("账号状态已变化，请重试".into());
        }
        match action {
            Action::List => connection.request(Method::GET, "/v1/teams", None),
            Action::Create { name } => {
                connection.request(Method::POST, "/v1/teams", Some(&json!({ "name": name })))
            }
            Action::Invite {
                team_id,
                name,
                email,
            } => connection.request(
                Method::POST,
                &format!("/v1/teams/{team_id}/members"),
                Some(&json!({ "name": name, "email": email })),
            ),
            Action::Accept { team_id, member_id } => connection.request(
                Method::POST,
                &format!("/v1/teams/{team_id}/members/{member_id}/accept"),
                None,
            ),
            Action::Remove { team_id, member_id } => {
                let result = connection.request(
                    Method::DELETE,
                    &format!("/v1/teams/{team_id}/members/{member_id}"),
                    None,
                )?;
                let _ = app.emit("sharing-changed", ());
                Ok(result)
            }
        }
    })
    .await
    .map_err(|_| "团队操作中断，请重试".to_string())?
}
