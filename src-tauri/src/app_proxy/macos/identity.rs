//! Bind to kernel audit tokens (including PID generation), not a PID-only lookup.
use core_foundation::{base::TCFType, data::CFData};
use security_framework::os::macos::code_signing::{
    Flags, GuestAttributes, SecCode, SecRequirement,
};
use std::os::fd::AsRawFd;
extern "C" {
    fn csops(
        pid: libc::pid_t,
        ops: libc::c_uint,
        useraddr: *mut libc::c_void,
        usersize: usize,
    ) -> libc::c_int;
}
pub fn live_hash() -> Result<String, String> {
    let mut digest = [0u8; 20];
    if unsafe {
        csops(
            std::process::id() as i32,
            5,
            digest.as_mut_ptr().cast(),
            digest.len(),
        )
    } != 0
        || digest.iter().all(|b| *b == 0)
    {
        return Err("无法核验应用代理程序身份，请重新安装 NodeCloak".into());
    }
    let hash = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    let requirement: SecRequirement = format!("cdhash H\"{hash}\"")
        .parse()
        .map_err(|_| "应用代理身份无效")?;
    SecCode::for_self(Flags::NONE)
        .and_then(|c| c.check_validity(Flags::STRICT_VALIDATE, &requirement))
        .map_err(|_| "应用代理程序身份已变化，请重新启动 NodeCloak")?;
    Ok(hash)
}
pub struct Peer {
    pub pid: u32,
    pub uid: u32,
}
impl Peer {
    pub fn alive(&self) -> bool {
        unsafe { libc::kill(self.pid as i32, 0) == 0 }
    }
}
pub fn peer(
    stream: &tokio::net::UnixStream,
    hash: &str,
    pid: Option<u32>,
    uid: Option<u32>,
) -> Result<Peer, String> {
    let mut token = [0u32; 8];
    let mut length = std::mem::size_of_val(&token) as libc::socklen_t;
    if unsafe {
        libc::getsockopt(
            stream.as_raw_fd(),
            0,
            0x006,
            token.as_mut_ptr().cast(),
            &mut length,
        )
    } != 0
        || length as usize != std::mem::size_of_val(&token)
    {
        return Err("无法读取应用代理通道身份".into());
    }
    if token[5] == 0 || pid.is_some_and(|p| p != token[5]) || uid.is_some_and(|u| u != token[1]) {
        return Err("应用代理通道的进程或用户不匹配".into());
    }
    if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("应用代理程序身份无效".into());
    }
    let bytes = unsafe { std::slice::from_raw_parts(token.as_ptr().cast::<u8>(), 32) };
    let data = CFData::from_buffer(bytes);
    let mut attributes = GuestAttributes::new();
    attributes.set_audit_token(data.as_concrete_TypeRef());
    let requirement: SecRequirement = format!("cdhash H\"{hash}\"")
        .parse()
        .map_err(|_| "应用代理程序身份无效")?;
    SecCode::copy_guest_with_attribues(None, &attributes, Flags::NONE)
        .and_then(|c| c.check_validity(Flags::STRICT_VALIDATE, &requirement))
        .map_err(|_| "应用代理通道的程序身份核验失败")?;
    Ok(Peer {
        pid: token[5],
        uid: token[1],
    })
}
