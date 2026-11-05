//! Pure configuration/route parsing, also exercised on non-Mac hosts.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize)]
pub struct Route {
    pub destination: String,
    pub gateway: String,
    pub flags: String,
    pub interface: String,
}
pub fn routes(text: &str) -> Result<Vec<Route>, String> {
    let mut index = None;
    let mut rows = vec![];
    for line in text.lines() {
        let parts: Vec<_> = line.split_whitespace().collect();
        if parts.first() == Some(&"Destination") {
            index = parts.iter().position(|s| *s == "Netif");
            if index.is_none() {
                return Err("无法识别 macOS 路由表格式".into());
            }
            continue;
        }
        let Some(i) = index else { continue };
        if parts.is_empty() {
            index = None;
            continue;
        }
        if parts.len() <= i || parts.len() < 4 {
            return Err("macOS 路由记录不完整".into());
        }
        rows.push(Route {
            destination: parts[0].into(),
            gateway: parts[1].into(),
            flags: parts[2].into(),
            interface: parts[i].into(),
        });
    }
    if !text.contains("Destination") {
        return Err("无法读取 macOS 路由表".into());
    }
    Ok(rows)
}
fn overlap(destination: &str) -> bool {
    use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
    if destination == "default" {
        return false;
    }
    let (raw, bits) = destination
        .split_once('/')
        .map(|(a, b)| (a, b.parse::<u32>().ok()))
        .unwrap_or((destination, None));
    let raw = raw.split('%').next().unwrap_or(raw);
    let ip = raw.parse::<IpAddr>().ok().or_else(|| {
        let values: Vec<_> = raw.split('.').collect();
        if !(1..=4).contains(&values.len()) {
            return None;
        }
        let mut v = [0; 4];
        for (i, s) in values.iter().enumerate() {
            v[i] = s.parse().ok()?
        }
        Some(Ipv4Addr::from(v).into())
    });
    match ip {
        Some(IpAddr::V4(ip)) => {
            let bits = bits.unwrap_or((raw.split('.').count() * 8) as u32);
            if bits <= 1 {
                return false;
            }
            if bits > 32 {
                return true;
            }
            let m = u32::MAX << (32 - bits.min(30));
            (u32::from(ip) & m) == (u32::from(Ipv4Addr::new(172, 29, 255, 0)) & m)
        }
        Some(IpAddr::V6(ip)) => {
            let bits = bits.unwrap_or(128);
            if bits <= 1 {
                return false;
            }
            if bits > 128 {
                return true;
            }
            let m = u128::MAX << (128 - bits.min(126));
            (u128::from(ip) & m)
                == (u128::from("fd64:6e6f:6465::".parse::<Ipv6Addr>().unwrap()) & m)
        }
        _ => false,
    }
}
pub fn preflight(
    routes: &[Route],
    interfaces: &[String],
    owned: &str,
    starting: bool,
) -> Result<(), String> {
    if starting && interfaces.iter().any(|i| i == owned) {
        return Err("此应用代理接口已存在，已保留原网络".into());
    }
    for route in routes.iter().filter(|r| r.interface != owned) {
        if !starting
            && route.interface == "lo0"
            && [
                "172.29.255.1",
                "172.29.255.1/32",
                "fd64:6e6f:6465::1",
                "fd64:6e6f:6465::1/128",
            ]
            .contains(&route.destination.as_str())
        {
            continue;
        }
        let tun = ["utun", "tun", "tap", "ppp", "ipsec"]
            .iter()
            .any(|s| route.interface.starts_with(s));
        if tun
            && !route.flags.contains('I')
            && [
                "default",
                "0/1",
                "0.0.0.0/1",
                "128.0/1",
                "128.0.0.0/1",
                "::/1",
                "8000::/1",
            ]
            .contains(&route.destination.as_str())
        {
            return Err(format!(
                "检测到 {} 正在接管网络，请先停止该 VPN/TUN 后再开启应用代理",
                route.interface
            ));
        }
        if overlap(&route.destination) {
            return Err("应用代理地址与现有网络路由重叠，已保留原网络设置".into());
        }
    }
    Ok(())
}
fn quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}
pub fn applescript(script: &str) -> String {
    format!("with timeout of 21474836 seconds\n do shell script \"{}\" with administrator privileges\nend timeout",script.replace('\\',"\\\\").replace('"',"\\\"").replace('\n',"\\n"))
}
pub fn check_network_now(
    monotonic: std::time::Duration,
    wall: Option<std::time::Duration>,
) -> Result<bool, String> {
    let wall = wall.ok_or("系统时间已变化，应用代理已停止；请重新开启")?;
    if wall > std::time::Duration::from_secs(15) {
        return Err("系统已唤醒或网络检查延迟，应用代理已停止；请重新开启".into());
    }
    Ok(monotonic >= std::time::Duration::from_secs(2))
}
pub fn bootstrap(
    executable: &str,
    suffix: &str,
    parent: u32,
    hash: &str,
    test: bool,
    manifest: &[(String, String)],
) -> Result<String, String> {
    if !executable.starts_with('/')
        || executable.chars().any(char::is_control)
        || !super::config::valid_id(suffix)
        || parent == 0
        || hash.len() != 40
        || !hash.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return Err("应用代理启动身份无效".into());
    }
    let label = format!(
        "com.nodecloak.ap.{}",
        uuid::Uuid::parse_str(suffix).unwrap().simple()
    );
    let arguments = if test {
        "<string>--exact</string><string>app_proxy::macos::native_checks::root_entry</string><string>--ignored</string><string>--nocapture</string>".into()
    } else {
        format!("<string>--nodecloak-app-proxy</string><string>{suffix}</string><string>{parent}</string>")
    };
    let requirement = quote(&format!("=cdhash H\"{hash}\""));
    let bundle = executable.rsplit_once(".app/");
    let mut hashes = String::new();
    if manifest.is_empty() || manifest.len() > 256 {
        return Err("应用代理校验清单无效".into());
    }
    for (path, hash) in manifest {
        if hash.len() != 64
            || !hash.bytes().all(|b| b.is_ascii_hexdigit())
            || path.chars().any(char::is_control)
            || path.starts_with('/')
            || path.contains('\\')
            || path
                .split('/')
                .any(|p| p.is_empty() || p == "." || p == "..")
            || if bundle.is_some() {
                !path.starts_with("Application.app/")
            } else {
                path != "helper"
            }
        {
            return Err("应用代理校验清单无效".into());
        }
        hashes.push_str(&format!("actual=$(/usr/bin/shasum -a 256 < \"$d\"/{})\ncase \"$actual\" in {hash}'  -') ;; *) /usr/bin/printf '%s\\n' 'Application proxy program changed during authorization' >&2; exit 1 ;; esac\n",quote(path)));
    }
    if hashes.len() > 100_000 {
        return Err("应用代理校验清单超过限制".into());
    }
    let symlinks = r#"root=$1
for link in "$root"/**/*(DN@); do
  resolved=${link:A}
  [[ "$resolved" == "$root/"* && -e "$resolved" ]] || { print -u2 'Application proxy bundle contains an external or broken symlink'; exit 1; }
done"#;
    let prepare = if let Some((bundle, tail)) = bundle {
        if !tail.starts_with("Contents/MacOS/")
            || tail.split('/').count() != 3
            || !tail
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"/._-".contains(&b))
        {
            return Err("应用代理程序在应用包中的位置无效".into());
        }
        format!("/usr/bin/ditto --noqtn --noextattr {} \"$d/Application.app\"\n/usr/sbin/chown -Rh -P 0:0 \"$d/Application.app\"\n/bin/zsh -f -c {} -- \"$d/Application.app\"\n{hashes}/usr/bin/codesign --verify --strict --deep \"$d/Application.app\"\n/usr/bin/codesign --verify --strict -R {requirement} \"$d/Application.app\"\nprogram=\"$d/Application.app/{tail}\"",quote(&format!("{bundle}.app")),quote(symlinks))
    } else {
        format!("/bin/cp {} \"$d/helper\"\n/bin/chmod 500 \"$d/helper\"\n{hashes}/usr/bin/codesign --verify --strict -R {requirement} \"$d/helper\"\nprogram=\"$d/helper\"",quote(executable))
    };
    Ok(format!(
        r#"set -eu
umask 077
d=$(/usr/bin/mktemp -d /private/var/run/nodecloak-ap.XXXXXXXX)
cleanup() {{
  trap - EXIT HUP INT TERM
  /bin/launchctl bootout system/{label} >/dev/null 2>&1 || :
  /bin/rm -f "$d/helper" "$d/sing-box" "$d/job.plist" "$d/helper.stderr"
  case "$d" in /private/var/run/nodecloak-ap.????????) /bin/rm -rf -- "$d/Application.app" ;; *) exit 1 ;; esac
  if test -d "$d"; then /bin/rmdir "$d"; fi
}}
trap cleanup EXIT HUP INT TERM
{prepare}
: > "$d/helper.stderr"
exec 5< "$d/helper.stderr"
/bin/cat > "$d/job.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
<key>Label</key><string>{label}</string>
<key>ProgramArguments</key><array><string>$program</string>{arguments}</array>
<key>EnvironmentVariables</key><dict><key>PATH</key><string>/usr/bin:/bin:/usr/sbin:/sbin</string><key>NC_AP_TEST_SUFFIX</key><string>{suffix}</string><key>NC_AP_TEST_PARENT</key><string>{parent}</string></dict>
<key>RunAtLoad</key><true/><key>KeepAlive</key><false/>
<key>StandardErrorPath</key><string>$d/helper.stderr</string>
<key>AbandonProcessGroup</key><false/>
</dict></plist>
PLIST
/bin/launchctl bootstrap system "$d/job.plist"
i=0
while state=$(/bin/launchctl print system/{label} 2>/dev/null); do
  if /usr/bin/printf '%s\n' "$state" | /usr/bin/grep -Eq 'pid = [1-9][0-9]*'; then
    /bin/sleep 1
  elif /usr/bin/printf '%s\n' "$state" | /usr/bin/grep -Eq 'runs = [1-9][0-9]*'; then
    break
  else
    i=$((i+1)); test "$i" -lt 300; /bin/sleep 0.1
  fi
done
/usr/bin/head -c 4096 <&5 >&2
"#,
    ))
}
#[cfg(test)]
mod checks {
    use super::*;
    #[test]
    fn clock_regression_and_sleep_do_not_suspend_network_guard() {
        use std::time::Duration;
        assert!(check_network_now(Duration::from_secs(2), None).is_err());
        assert!(check_network_now(Duration::from_secs(2), Some(Duration::from_secs(120))).is_err());
        assert!(check_network_now(Duration::from_secs(2), Some(Duration::from_secs(2))).unwrap());
        assert!(
            !check_network_now(Duration::from_millis(100), Some(Duration::from_millis(100)))
                .unwrap()
        );
    }
    #[test]
    fn scoped_apple_interfaces_are_not_competing_vpns_but_unscoped_defaults_are() {
        let base="Routing tables\nInternet6:\nDestination Gateway Flags Netif Expire\ndefault fe80::%utun0 UGcIg utun0\n";
        let rows = routes(base).unwrap();
        assert!(preflight(&rows, &[], "utun1000", true).is_ok());
        let global = routes(&base.replace("UGcIg", "UGc")).unwrap();
        assert!(preflight(&global, &[], "utun1000", true).is_err());
        let collision =
            routes("Destination Gateway Flags Netif\n172.29.255/30 link#9 UCS en0\n").unwrap();
        assert!(preflight(&collision, &[], "utun1000", true).is_err());
        assert!(preflight(&[], &["utun1000".into()], "utun1000", true).is_err());
    }
    #[test]
    fn bootstrap_pins_code_before_launch_and_preserves_launchd_process_ownership() {
        let s = bootstrap(
            "/Applications/O'Brian $(whoami).app/Contents/MacOS/app",
            "0b4fcb93-ec83-4f9b-996a-f89f7aa96025",
            123,
            &"a".repeat(40),
            false,
            &[("Application.app/Contents/MacOS/app".into(), "b".repeat(64))],
        )
        .unwrap();
        assert!(s.contains("O'\\''Brian $(whoami)"));
        assert!(
            s.find("codesign --verify --strict -R").unwrap()
                < s.find("launchctl bootstrap").unwrap()
        );
        assert!(s.contains("<key>AbandonProcessGroup</key><false/>"));
        assert!(s.contains("case \"$d\" in /private/var/run/nodecloak-ap.????????) /bin/rm -rf -- \"$d/Application.app\""));
        assert!(s.contains("/usr/bin/ditto --noqtn --noextattr"));
        assert!(!s.contains("setpgid"));
        assert!(bootstrap(
            "/tmp/host",
            "bad",
            123,
            &"a".repeat(40),
            false,
            &[("helper".into(), "b".repeat(64))]
        )
        .is_err());
        assert!(bootstrap(
            "/tmp/host",
            "0b4fcb93-ec83-4f9b-996a-f89f7aa96025",
            123,
            "bad'",
            false,
            &[("helper".into(), "b".repeat(64))]
        )
        .is_err());
    }
}
