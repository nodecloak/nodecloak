//! Persistent ALE policy. Closing this engine handle deliberately does NOT
//! remove the filters: helper/core/GUI crashes must never restore direct access.
use super::security::wide;
use crate::app_proxy::model::ResolvedRule;
use std::{collections::BTreeSet, ptr};
use windows_sys::{
    core::GUID,
    Win32::{Foundation::HANDLE, NetworkManagement::WindowsFilteringPlatform::*},
};

const SUBLAYER: GUID = GUID::from_u128(0x5ad0ca97_8d4d_4200_9411_d5718ec45051);
fn same(a: &GUID, b: &GUID) -> bool {
    a.data1 == b.data1 && a.data2 == b.data2 && a.data3 == b.data3 && a.data4 == b.data4
}
fn check(code: u32) -> Result<(), String> {
    if code == 0 {
        Ok(())
    } else {
        Err(format!(
            "应用代理断网保护操作失败（Windows 错误 {code}）；未解除已有保护"
        ))
    }
}
struct Engine(HANDLE);
impl Engine {
    fn open() -> Result<Self, String> {
        let mut handle = ptr::null_mut();
        check(unsafe { FwpmEngineOpen0(ptr::null(), 10, ptr::null(), ptr::null(), &mut handle) })?;
        Ok(Self(handle))
    }
    fn ids(&self) -> Result<Vec<u64>, String> {
        let mut enumerator = ptr::null_mut();
        check(unsafe { FwpmFilterCreateEnumHandle0(self.0, ptr::null(), &mut enumerator) })?;
        let result = (|| {
            let mut ids = Vec::new();
            loop {
                let mut rows = ptr::null_mut();
                let mut count = 0;
                check(unsafe { FwpmFilterEnum0(self.0, enumerator, 256, &mut rows, &mut count) })?;
                if !rows.is_null() {
                    for row in unsafe { std::slice::from_raw_parts(rows, count as usize) } {
                        if !row.is_null() {
                            let row = unsafe { &**row };
                            if same(&row.subLayerKey, &SUBLAYER) {
                                ids.push(row.filterId);
                            }
                        }
                    }
                    unsafe { FwpmFreeMemory0(&mut rows as *mut _ as *mut _) };
                }
                if count < 256 {
                    break;
                }
            }
            Ok(ids)
        })();
        unsafe { FwpmFilterDestroyEnumHandle0(self.0, enumerator) };
        result
    }
    fn transaction(&self, work: impl FnOnce() -> Result<(), String>) -> Result<(), String> {
        check(unsafe { FwpmTransactionBegin0(self.0, 0) })?;
        match work() {
            Ok(()) => {
                let code = unsafe { FwpmTransactionCommit0(self.0) };
                if code != 0 {
                    unsafe { FwpmTransactionAbort0(self.0) };
                }
                check(code)
            }
            Err(e) => {
                unsafe { FwpmTransactionAbort0(self.0) };
                Err(e)
            }
        }
    }
    fn delete(&self) -> Result<(), String> {
        for id in self.ids()? {
            check(unsafe { FwpmFilterDeleteById0(self.0, id) })?;
        }
        Ok(())
    }
}
impl Drop for Engine {
    fn drop(&mut self) {
        unsafe { FwpmEngineClose0(self.0) };
    }
}
const MARKER: &str = r"SOFTWARE\NodeCloak\ApplicationProxyProtection";
fn marker(value: u32) -> Result<(), String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    let (key, _) = RegKey::predef(HKEY_LOCAL_MACHINE)
        .create_subkey(MARKER)
        .map_err(|e| format!("无法保存断网保护恢复状态：{e}"))?;
    key.set_value("Active", &value)
        .map_err(|e| format!("无法保存断网保护恢复状态：{e}"))
}
fn saved_state() -> Result<u32, String> {
    use winreg::{enums::HKEY_LOCAL_MACHINE, RegKey};
    match RegKey::predef(HKEY_LOCAL_MACHINE).open_subkey(MARKER) {
        Ok(key) => match key.get_value::<u32, _>("Active") {
            Ok(v) => Ok(v),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(2),
            Err(e) => Err(format!("无法读取断网保护恢复状态：{e}")),
        },
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(0),
        Err(e) => Err(format!("无法读取断网保护恢复状态：{e}")),
    }
}
pub fn active() -> Result<bool, String> {
    if super::security::elevated() {
        return Ok(!Engine::open()?.ids()?.is_empty());
    }
    Ok(saved_state()? == 1)
}
pub fn pending() -> Result<bool, String> {
    Ok(saved_state()? == 2)
}
pub fn needs_recovery() -> Result<bool, String> {
    Ok(saved_state()? > 0 || active()?)
}
/// Called only by the authenticated privileged helper, under ownership_mutex.
pub fn clear() -> Result<(), String> {
    let engine = Engine::open()?;
    engine.transaction(|| engine.delete())?;
    marker(0)
}
pub fn replace(rules: &[ResolvedRule], allowed_luid: u64) -> Result<(), String> {
    let paths: BTreeSet<_> = rules.iter().flat_map(|r| r.paths.iter()).collect();
    if paths.is_empty() {
        return Err("没有可保护的应用，已保留原有断网保护".into());
    }
    let engine = Engine::open()?;
    // Keep a conservative recovery hint even if the helper dies immediately
    // after committing WFP policy. Failure never silently clears prior policy.
    marker(2)?;
    engine.transaction(|| {
        let name = wide("NodeCloak application proxy direct-connection protection");
        let layer = FWPM_SUBLAYER0 {
            subLayerKey: SUBLAYER,
            displayData: FWPM_DISPLAY_DATA0 {
                name: name.as_ptr() as _,
                description: ptr::null_mut(),
            },
            flags: FWPM_SUBLAYER_FLAG_PERSISTENT as _,
            weight: 0xffff,
            ..Default::default()
        };
        let code = unsafe { FwpmSubLayerAdd0(engine.0, &layer, ptr::null_mut()) };
        if code != 0 && code != 0x80320009 {
            check(code)?;
        } // FWP_E_ALREADY_EXISTS
        engine.delete()?;
        for path in paths {
            let filename = wide(path);
            let mut app = ptr::null_mut();
            check(unsafe { FwpmGetAppIdFromFileName0(filename.as_ptr(), &mut app) })?;
            let result: Result<(), String> = (|| {
                let mut luid = allowed_luid;
                let mut conditions = [
                    FWPM_FILTER_CONDITION0 {
                        fieldKey: FWPM_CONDITION_ALE_APP_ID,
                        matchType: FWP_MATCH_EQUAL,
                        conditionValue: FWP_CONDITION_VALUE0 {
                            r#type: FWP_BYTE_BLOB_TYPE,
                            Anonymous: FWP_CONDITION_VALUE0_0 { byteBlob: app },
                        },
                    },
                    FWPM_FILTER_CONDITION0 {
                        fieldKey: FWPM_CONDITION_IP_LOCAL_INTERFACE,
                        matchType: FWP_MATCH_NOT_EQUAL,
                        conditionValue: FWP_CONDITION_VALUE0 {
                            r#type: FWP_UINT64,
                            Anonymous: FWP_CONDITION_VALUE0_0 { uint64: &mut luid },
                        },
                    },
                    // Preserve local IPC. A separately configured loopback proxy
                    // is explicitly outside per-process TUN enforcement.
                    FWPM_FILTER_CONDITION0 {
                        fieldKey: FWPM_CONDITION_FLAGS,
                        matchType: FWP_MATCH_FLAGS_NONE_SET,
                        conditionValue: FWP_CONDITION_VALUE0 {
                            r#type: FWP_UINT32,
                            Anonymous: FWP_CONDITION_VALUE0_0 {
                                uint32: FWP_CONDITION_FLAG_IS_LOOPBACK,
                            },
                        },
                    },
                ];
                for layer_key in [
                    FWPM_LAYER_ALE_AUTH_CONNECT_V4,
                    FWPM_LAYER_ALE_AUTH_CONNECT_V6,
                ] {
                    let filter = FWPM_FILTER0 {
                        displayData: FWPM_DISPLAY_DATA0 {
                            name: name.as_ptr() as _,
                            description: ptr::null_mut(),
                        },
                        flags: FWPM_FILTER_FLAG_PERSISTENT,
                        layerKey: layer_key,
                        subLayerKey: SUBLAYER,
                        numFilterConditions: conditions.len() as _,
                        filterCondition: conditions.as_mut_ptr(),
                        action: FWPM_ACTION0 {
                            r#type: FWP_ACTION_BLOCK,
                            ..Default::default()
                        },
                        ..Default::default()
                    };
                    check(unsafe {
                        FwpmFilterAdd0(engine.0, &filter, ptr::null_mut(), ptr::null_mut())
                    })?;
                }
                Ok(())
            })();
            unsafe { FwpmFreeMemory0(&mut app as *mut _ as *mut _) };
            result?;
        }
        Ok(())
    })?;
    marker(1)
}

#[cfg(test)]
mod tests {
    #[test]
    fn protection_status_can_be_read_without_changing_network() {
        assert!(super::active().is_ok());
    }
}
