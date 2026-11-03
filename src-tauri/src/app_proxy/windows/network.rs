use serde::Serialize;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use windows_sys::Win32::{Foundation::*, NetworkManagement::IpHelper::*, Networking::WinSock::*};
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Adapter {
    pub name: String,
    pub description: String,
    pub luid: u64,
    pub index: u32,
    pub up: bool,
    pub kind: u32,
    pub dns: Vec<String>,
}
#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct Route {
    pub luid: u64,
    pub prefix: String,
}
#[derive(Clone, Serialize)]
pub struct Snapshot {
    pub adapters: Vec<Adapter>,
    pub routes: Vec<Route>,
}
pub fn preflight(snapshot: &Snapshot, interface: &str) -> Result<(), String> {
    if snapshot
        .adapters
        .iter()
        .any(|a| a.name.eq_ignore_ascii_case(interface))
    {
        return Err("此应用代理接口已存在，已停止启动以保留原网络".into());
    }
    for adapter in snapshot.adapters.iter().filter(|a| a.up) {
        let description = format!("{} {}", adapter.name, adapter.description).to_lowercase();
        let tunnel = [23, 53, 131].contains(&adapter.kind)
            || [
                "wintun",
                "wireguard",
                "tap-windows",
                "clash",
                "mihomo",
                "sing-box",
                "sing-tun",
                "openvpn",
                "tailscale",
                "zerotier",
                "wan miniport",
            ]
            .iter()
            .any(|word| description.contains(word));
        let owns_default = snapshot.routes.iter().any(|r| {
            r.luid == adapter.luid
                && [
                    "0.0.0.0/0",
                    "0.0.0.0/1",
                    "128.0.0.0/1",
                    "::/0",
                    "::/1",
                    "8000::/1",
                ]
                .contains(&r.prefix.as_str())
        });
        if tunnel && owns_default {
            return Err(format!(
                "检测到 {} 正在接管网络，请先停止该 VPN/TUN 后再开启应用代理",
                adapter.name
            ));
        }
    }
    for route in &snapshot.routes {
        if overlaps(&route.prefix) {
            return Err("应用代理地址与现有网络路由重叠，已保留原网络设置".into());
        }
    }
    Ok(())
}
fn overlaps(prefix: &str) -> bool {
    let Some((ip, bits)) = prefix.split_once('/') else {
        return true;
    };
    let (Ok(ip), Ok(bits)) = (ip.parse::<IpAddr>(), bits.parse::<u32>()) else {
        return true;
    };
    if bits <= 1 {
        return false;
    }
    match ip {
        IpAddr::V4(ip) => {
            if bits > 32 {
                return true;
            }
            let n = bits.min(30);
            let mask = u32::MAX << (32 - n);
            (u32::from(ip) & mask) == (u32::from(Ipv4Addr::new(172, 29, 255, 0)) & mask)
        }
        IpAddr::V6(ip) => {
            if bits > 128 {
                return true;
            }
            let n = bits.min(126);
            let mask = u128::MAX << (128 - n);
            let target = "fd64:6e6f:6465::".parse::<Ipv6Addr>().unwrap();
            (u128::from(ip) & mask) == (u128::from(target) & mask)
        }
    }
}
pub fn snapshot() -> Result<Snapshot, String> {
    let mut size = 32768u32;
    let mut buffer = Vec::<u64>::new();
    let mut success = false;
    for _ in 0..4 {
        if size > 8 * 1024 * 1024 {
            return Err("系统网络接口信息过大".into());
        }
        buffer.resize((size as usize + 7) / 8, 0);
        let code = unsafe {
            GetAdaptersAddresses(
                AF_UNSPEC as u32,
                GAA_FLAG_INCLUDE_ALL_INTERFACES,
                std::ptr::null(),
                buffer.as_mut_ptr() as _,
                &mut size,
            )
        };
        if code == NO_ERROR {
            success = true;
            break;
        }
        if code != ERROR_BUFFER_OVERFLOW {
            return Err(format!("无法读取系统网络接口（Windows 错误 {code}）"));
        }
    }
    if !success {
        return Err("无法稳定读取系统网络接口".into());
    }
    let mut adapters = vec![];
    let mut cursor = buffer.as_ptr() as *const IP_ADAPTER_ADDRESSES_LH;
    for _ in 0..1024 {
        if cursor.is_null() {
            break;
        }
        let adapter = unsafe { &*cursor };
        let mut dns = vec![];
        let mut server = adapter.FirstDnsServerAddress;
        for _ in 0..64 {
            if server.is_null() {
                break;
            }
            let record = unsafe { &*server };
            let address = record.Address.lpSockaddr;
            if !address.is_null() {
                let family = unsafe { (*address).sa_family };
                if family == AF_INET
                    && record.Address.iSockaddrLength >= std::mem::size_of::<SOCKADDR_IN>() as i32
                {
                    let value = unsafe { std::ptr::read_unaligned(address as *const SOCKADDR_IN) };
                    dns.push(
                        Ipv4Addr::from(unsafe { value.sin_addr.S_un.S_addr }.to_ne_bytes())
                            .to_string(),
                    );
                } else if family == AF_INET6
                    && record.Address.iSockaddrLength >= std::mem::size_of::<SOCKADDR_IN6>() as i32
                {
                    let value = unsafe { std::ptr::read_unaligned(address as *const SOCKADDR_IN6) };
                    dns.push(Ipv6Addr::from(unsafe { value.sin6_addr.u.Byte }).to_string());
                }
            }
            server = record.Next;
        }
        adapters.push(Adapter {
            name: super::security::read_wide(adapter.FriendlyName, 512),
            description: super::security::read_wide(adapter.Description, 512),
            luid: unsafe { adapter.Luid.Value },
            index: unsafe { adapter.Anonymous1.Anonymous.IfIndex },
            up: adapter.OperStatus == 1,
            kind: adapter.IfType,
            dns,
        });
        cursor = adapter.Next;
    }
    let mut table = std::ptr::null_mut();
    let code = unsafe { GetIpForwardTable2(AF_UNSPEC, &mut table) };
    if code != NO_ERROR || table.is_null() {
        return Err(format!("无法读取系统路由（Windows 错误 {code}）"));
    }
    let mut routes = vec![];
    unsafe {
        let count = (*table).NumEntries as usize;
        if count > 65536 {
            FreeMibTable(table as _);
            return Err("系统路由表过大".into());
        }
        for row in std::slice::from_raw_parts((*table).Table.as_ptr(), count) {
            let address = &row.DestinationPrefix.Prefix;
            let ip = if address.si_family == AF_INET {
                IpAddr::V4(Ipv4Addr::from(
                    address.Ipv4.sin_addr.S_un.S_addr.to_ne_bytes(),
                ))
            } else if address.si_family == AF_INET6 {
                IpAddr::V6(Ipv6Addr::from(address.Ipv6.sin6_addr.u.Byte))
            } else {
                continue;
            };
            routes.push(Route {
                luid: row.InterfaceLuid.Value,
                prefix: format!("{ip}/{}", row.DestinationPrefix.PrefixLength),
            });
        }
        FreeMibTable(table as _);
    }
    Ok(Snapshot { adapters, routes })
}
pub fn interface_luid(name: &str) -> Result<Option<u64>, String> {
    Ok(snapshot()?
        .adapters
        .iter()
        .find(|a| a.name == name && a.up)
        .map(|a| a.luid))
}
pub fn owned_luid(name: &str) -> Result<Option<u64>, String> {
    Ok(snapshot()?
        .adapters
        .iter()
        .find(|a| a.name == name)
        .map(|a| a.luid))
}
pub fn session_released(name: &str, luid: Option<u64>) -> Result<bool, String> {
    let state = snapshot()?;
    let luid = luid.or_else(|| {
        state
            .adapters
            .iter()
            .find(|a| a.name == name)
            .map(|a| a.luid)
    });
    Ok(!state.adapters.iter().any(|a| a.name == name && a.up)
        && luid.is_none_or(|id| !state.routes.iter().any(|r| r.luid == id)))
}
pub fn released(name: &str, luid: u64) -> Result<bool, String> {
    let state = snapshot()?;
    Ok(!state
        .adapters
        .iter()
        .any(|a| a.luid == luid && a.name == name && a.up)
        && !state.routes.iter().any(|r| r.luid == luid))
}
#[cfg(test)]
mod checks {
    use super::*;
    fn adapter(name: &str, luid: u64, kind: u32) -> Adapter {
        Adapter {
            name: name.into(),
            description: name.into(),
            luid,
            index: luid as u32,
            up: true,
            kind,
            dns: vec![],
        }
    }
    #[test]
    fn competing_tun_default_routes_are_rejected_but_regular_lan_is_kept() {
        let mut snapshot = Snapshot {
            adapters: vec![adapter("Ethernet", 1, 6)],
            routes: vec![Route {
                luid: 1,
                prefix: "0.0.0.0/0".into(),
            }],
        };
        assert!(preflight(&snapshot, "NCApps-test").is_ok());
        snapshot.adapters.push(adapter("Mihomo Wintun", 2, 53));
        snapshot.routes.push(Route {
            luid: 2,
            prefix: "0.0.0.0/1".into(),
        });
        assert!(preflight(&snapshot, "NCApps-test")
            .unwrap_err()
            .contains("Mihomo"));
    }
    #[test]
    fn existing_owned_name_or_overlapping_tun_addresses_cannot_be_adopted() {
        let mut snapshot = Snapshot {
            adapters: vec![adapter("NCApps-test", 1, 53)],
            routes: vec![],
        };
        assert!(preflight(&snapshot, "NCApps-test").is_err());
        snapshot.adapters[0].name = "Ethernet".into();
        snapshot.routes.push(Route {
            luid: 1,
            prefix: "172.16.0.0/12".into(),
        });
        assert!(preflight(&snapshot, "NCApps-test").is_err());
    }
    #[test]
    fn windows_ras_ppp_default_owner_is_a_competing_vpn() {
        let state = Snapshot {
            adapters: vec![adapter("Work VPN", 3, 23)],
            routes: vec![Route {
                luid: 3,
                prefix: "0.0.0.0/0".into(),
            }],
        };
        assert!(preflight(&state, "NCApps-test").is_err());
    }
}
