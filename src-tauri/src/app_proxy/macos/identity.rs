//! Bind to kernel audit tokens (including PID generation), not a PID-only lookup.
use security_framework::os::macos::code_signing::{Flags, SecCode, SecRequirement};
use std::os::fd::AsRawFd;
extern "C" {
    fn csops_audittoken(
        pid: libc::pid_t,
        ops: libc::c_uint,
        useraddr: *mut libc::c_void,
        usersize: usize,
        token: *const u32,
    ) -> libc::c_int;
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
        return Err(format!(
            "无法读取应用代理通道身份（{}）",
            std::io::Error::last_os_error()
        ));
    }
    if token[5] == 0 || pid.is_some_and(|p| p != token[5]) || uid.is_some_and(|u| u != token[1]) {
        return Err("应用代理通道的进程或用户不匹配".into());
    }
    if hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("应用代理程序身份无效".into());
    }
    validate_token(&token, hash)?;
    Ok(Peer {
        pid: token[5],
        uid: token[1],
    })
}

/// Query the executing image through its kernel audit generation, without
/// reading the peer's filesystem. GUI cannot traverse the root-only copy.
/// This complements (never replaces) self strict and full root-copy checks.
pub(super) fn validate_token(token: &[u32; 8], hash: &str) -> Result<(), String> {
    if token[5] == 0 || hash.len() != 40 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("应用代理程序身份无效".into());
    }
    let mut digest = [0u8; 20];
    if unsafe {
        csops_audittoken(
            token[5] as i32,
            5,
            digest.as_mut_ptr().cast(),
            digest.len(),
            token.as_ptr(),
        )
    } != 0
    {
        return Err(format!(
            "无法读取应用代理进程代码身份（{}）",
            std::io::Error::last_os_error()
        ));
    }
    let actual = digest
        .iter()
        .map(|b| format!("{b:02x}"))
        .collect::<String>();
    if !actual.eq_ignore_ascii_case(hash) {
        return Err("应用代理通道的程序身份不匹配".into());
    }
    let mut status = 0u32;
    if unsafe {
        csops_audittoken(
            token[5] as i32,
            0,
            (&mut status as *mut u32).cast(),
            std::mem::size_of_val(&status),
            token.as_ptr(),
        )
    } != 0
    {
        return Err(format!(
            "无法核验应用代理进程状态（{}）",
            std::io::Error::last_os_error()
        ));
    }
    // XNU cs_blobs.h: CS_VALID, CS_KILLED and CS_DEBUGGED.
    if status & 1 == 0 || status & 0x11000000 != 0 {
        return Err("应用代理通道的代码状态无效".into());
    }
    Ok(())
}
