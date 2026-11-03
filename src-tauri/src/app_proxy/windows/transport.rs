use super::{
    process,
    security::{self, Descriptor, Handle},
};
use crate::app_proxy::ipc::{self, Hello};
use std::{os::windows::io::AsRawHandle, time::Duration};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::System::Pipes::*;

pub fn name(suffix: &str) -> Result<String, String> {
    let id = uuid::Uuid::parse_str(suffix).map_err(|_| "应用代理通道标识无效")?;
    Ok(format!(r"\\.\pipe\nodecloak-app-proxy-{}", id.simple()))
}
fn peer(handle: std::os::windows::io::RawHandle, server: bool) -> Result<u32, String> {
    let mut id = 0;
    let ok = unsafe {
        if server {
            GetNamedPipeClientProcessId(handle, &mut id)
        } else {
            GetNamedPipeServerProcessId(handle, &mut id)
        }
    };
    if ok == 0 || id == 0 {
        return Err("无法核验应用代理通道身份".into());
    }
    Ok(id)
}
pub async fn connect_helper() -> Result<(NamedPipeServer, Handle), String> {
    let suffix = uuid::Uuid::new_v4().to_string();
    let pipe_name = name(&suffix)?;
    let server = {
        let sid = security::current_sid()?;
        let descriptor = Descriptor::new(&format!("D:P(A;;GA;;;SY)(A;;GA;;;BA)(A;;GA;;;{sid})"))?;
        let mut attributes = descriptor.attributes();
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(true)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(&pipe_name, &mut attributes as *mut _ as _)
        }
        .map_err(|_| "无法创建受限应用代理通道")?
    };
    let helper =
        tokio::task::spawn_blocking(move || process::launch_helper(&suffix, std::process::id()))
            .await
            .map_err(|_| "应用代理授权任务中断")??;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    loop {
        tokio::time::timeout_at(deadline, server.connect())
            .await
            .map_err(|_| "应用代理管理员辅助进程连接超时")?
            .map_err(|_| "无法连接应用代理管理员辅助进程")?;
        if helper.alive() && peer(server.as_raw_handle(), true)? == helper.pid() {
            break;
        }
        server
            .disconnect()
            .map_err(|_| "应用代理通道身份核验失败")?;
        if !helper.alive() {
            return Err("应用代理管理员辅助进程已退出".into());
        }
    }
    let mut server = server;
    let token = format!(
        "{}{}",
        uuid::Uuid::new_v4().simple(),
        uuid::Uuid::new_v4().simple()
    );
    let challenge = Hello { protocol: 1, token };
    tokio::time::timeout(
        Duration::from_secs(5),
        ipc::write_frame(&mut server, &challenge),
    )
    .await
    .map_err(|_| "应用代理身份核验超时")??;
    let hello: Hello = tokio::time::timeout(Duration::from_secs(5), ipc::read_frame(&mut server))
        .await
        .map_err(|_| "应用代理身份核验超时")??;
    if !ipc::authenticated(&hello, &challenge.token) {
        return Err("应用代理管理员辅助进程身份不匹配".into());
    }
    Ok((server, helper))
}
pub async fn open_parent(suffix: &str, parent: u32) -> Result<(NamedPipeClient, Handle), String> {
    if !security::elevated() {
        return Err("应用代理需要 Windows 管理员授权".into());
    }
    let parent_handle = Handle::open(parent)?;
    let pipe_name = name(suffix)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(15);
    let mut pipe = loop {
        if !parent_handle.alive() {
            return Err("应用代理主程序已退出".into());
        }
        match ClientOptions::new().open(&pipe_name) {
            Ok(pipe) => break pipe,
            Err(_) if tokio::time::Instant::now() < deadline => {
                tokio::time::sleep(Duration::from_millis(50)).await
            }
            Err(_) => return Err("无法连接应用代理主程序".into()),
        }
    };
    if peer(pipe.as_raw_handle(), false)? != parent || !parent_handle.alive() {
        return Err("应用代理主程序身份不匹配".into());
    }
    let hello: Hello = tokio::time::timeout(Duration::from_secs(5), ipc::read_frame(&mut pipe))
        .await
        .map_err(|_| "应用代理身份核验超时")??;
    if !ipc::authenticated(&hello, &hello.token) {
        return Err("应用代理主程序身份核验失败".into());
    }
    ipc::write_frame(&mut pipe, &hello).await?;
    Ok((pipe, parent_handle))
}
