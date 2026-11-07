use super::super::macos_support::{self, Route};
use std::{
    ffi::CString,
    io::Write,
    process::{Command, Stdio},
};
#[derive(Clone, serde::Serialize)]
pub struct Snapshot {
    pub interfaces: Vec<String>,
    pub routes: Vec<Route>,
    pub signature: String,
}
fn output(command: &str, args: &[&str]) -> Result<String, String> {
    let result = Command::new(command)
        .args(args)
        .env_clear()
        .env("PATH", "/usr/bin:/bin:/usr/sbin:/sbin")
        .output()
        .map_err(|_| "无法读取 macOS 网络状态")?;
    if !result.status.success() || result.stdout.len() > 4_194_304 {
        return Err("无法确认 macOS 网络状态，已保留原网络".into());
    }
    String::from_utf8(result.stdout).map_err(|_| "macOS 网络状态格式无效".into())
}
pub fn signature() -> Result<String, String> {
    let mut child = Command::new("/usr/sbin/scutil")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .env_clear()
        .spawn()
        .map_err(|_| "无法读取当前网络连接")?;
    child
        .stdin
        .take()
        .ok_or("无法读取当前网络连接")?
        .write_all(b"show State:/Network/Global/IPv4\nshow State:/Network/Global/IPv6\nquit\n")
        .map_err(|_| "无法读取当前网络连接")?;
    let out = child
        .wait_with_output()
        .map_err(|_| "无法读取当前网络连接")?;
    if !out.status.success() || out.stdout.len() > 65536 {
        return Err("无法确认当前网络连接".into());
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}
pub fn snapshot() -> Result<Snapshot, String> {
    let interfaces = output("/sbin/ifconfig", &["-l"])?
        .split_whitespace()
        .map(str::to_owned)
        .collect();
    let mut routes = macos_support::routes(&output("/usr/sbin/netstat", &["-rn", "-f", "inet"])?)?;
    routes.extend(macos_support::routes(&output(
        "/usr/sbin/netstat",
        &["-rn", "-f", "inet6"],
    )?)?);
    Ok(Snapshot {
        interfaces,
        routes,
        signature: signature()?,
    })
}
pub fn preflight(snapshot: &Snapshot, interface: &str) -> Result<(), String> {
    macos_support::preflight(&snapshot.routes, &snapshot.interfaces, interface, true)
}
fn valid_interface(s: &str) -> bool {
    s.strip_prefix("utun").is_some_and(|n| {
        !n.is_empty()
            && n.bytes().all(|b| b.is_ascii_digit())
            && n.parse::<u32>().is_ok_and(|n| (1024..=32767).contains(&n))
    })
}
pub fn interface_luid(interface: &str) -> Result<Option<u64>, String> {
    if !valid_interface(interface) {
        return Err("应用代理接口名称无效".into());
    }
    let name = CString::new(interface).map_err(|_| "应用代理接口名称无效")?;
    let index = unsafe { libc::if_nametoindex(name.as_ptr()) };
    Ok((index != 0).then_some(index as u64))
}
pub fn owned_luid(interface: &str) -> Result<Option<u64>, String> {
    let Some(index) = interface_luid(interface)? else {
        return Ok(None);
    };
    let state = output("/sbin/ifconfig", &[interface])?;
    if !state
        .lines()
        .any(|l| l.trim().starts_with("inet 172.29.255.1 "))
    {
        return Ok(None);
    }
    Ok(Some(index))
}
pub fn session_released(interface: &str, index: Option<u64>) -> Result<bool, String> {
    let current = interface_luid(interface)?;
    if current.is_some() {
        if index.is_some() && current != index {
            return Err("应用代理接口身份已变化，未修改其他网络接口".into());
        }
        return Ok(false);
    }
    let snapshot = snapshot()?;
    Ok(!snapshot.routes.iter().any(|r| r.interface == interface))
}
pub fn released(interface: &str, index: u64) -> Result<bool, String> {
    session_released(interface, Some(index))
}
pub fn unchanged(base: &str, owned: &str) -> Result<bool, String> {
    let now = snapshot()?;
    Ok(now.signature == base
        && macos_support::preflight(&now.routes, &now.interfaces, owned, false).is_ok())
}
