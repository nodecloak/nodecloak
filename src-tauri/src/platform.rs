use std::{
    path::{Path, PathBuf},
    process::{Command, Stdio},
};
#[cfg(any(target_os = "linux", test))]
mod desktop_linux;
#[cfg(windows)]
mod endpoint;
#[cfg(any(test, target_os = "macos", target_os = "linux"))]
mod singleton;
#[cfg(any(test,target_os="macos", target_os = "linux"))]
mod quit;
#[cfg(target_os = "linux")]
mod linux;

pub fn command(program: &str) -> Command {
    let mut cmd = Command::new(program);
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x08000000); // Do not flash a console for background diagnostics.
    }
    cmd
}

fn output(program: &str, args: &[&str]) -> Result<String, String> {
    let result = command(program)
        .args(args)
        .output()
        .map_err(|e| format!("无法运行系统命令：{e}"))?;
    if !result.status.success() {
        return Err(command_failure(program, &result));
    }
    Ok(String::from_utf8_lossy(&result.stdout).trim().to_string())
}

fn command_failure(program: &str, result: &std::process::Output) -> String {
    let stderr = String::from_utf8_lossy(&result.stderr);
    let stdout = String::from_utf8_lossy(&result.stdout);
    let detail = if stderr.trim().is_empty() {
        stdout.trim()
    } else {
        stderr.trim()
    };
    format!(
        "系统命令 {program} 执行失败（退出码 {}）：{}",
        result
            .status
            .code()
            .map(|c| c.to_string())
            .unwrap_or_else(|| "未知".into()),
        detail.chars().take(600).collect::<String>()
    )
}

#[cfg(target_os = "windows")]
fn elevate_timezone(zone: &str) -> Result<(), String> {
    // Only tzutil is elevated; the main application and browser stay unelevated.
    // zone has already been restricted to characters that cannot escape either quote context.
    let script = format!(
        r#"$ErrorActionPreference='Stop'; try {{ $exe=Join-Path $env:SystemRoot 'System32\tzutil.exe'; $p=Start-Process -FilePath $exe -ArgumentList '/s "{zone}"' -Verb RunAs -WindowStyle Hidden -Wait -PassThru; Write-Output ('EXIT:'+ $p.ExitCode) }} catch {{ $ex=$_.Exception; while($ex.InnerException) {{ $ex=$ex.InnerException }}; if($ex.NativeErrorCode -eq 1223) {{ Write-Output 'CANCELLED' }} else {{ Write-Output ('ERROR:'+ $ex.Message) }} }}"#
    );
    let response = output(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-Command", &script],
    )?;
    match response.trim() {
        "EXIT:0" => Ok(()),
        "CANCELLED" => {
            Err("已取消管理员授权，系统时区未确认修改。可稍后重试或在系统设置中手动调整。".into())
        }
        value => Err(format!(
            "管理员授权后的时区修改未完成：{}。请检查组织策略或系统日期和时间设置。",
            value.chars().take(600).collect::<String>()
        )),
    }
}

pub fn browser_path(browser: &str) -> Result<PathBuf, String> {
    #[cfg(target_os = "linux")]
    return linux::browser_path(browser);
    #[cfg(not(target_os = "linux"))]
    {
    let suffix = match browser {
        "chrome" => "Google/Chrome/Application/chrome.exe",
        "edge" => "Microsoft/Edge/Application/msedge.exe",
        "firefox" => "Mozilla Firefox/firefox.exe",
        _ => return Err("不支持的浏览器".into()),
    };
    #[cfg(target_os = "windows")]
    for root in ["LOCALAPPDATA", "PROGRAMFILES", "PROGRAMFILES(X86)"] {
        if let Some(base) = std::env::var_os(root) {
            let path = PathBuf::from(base).join(suffix);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    #[cfg(target_os = "macos")]
    {
        let app = if browser == "chrome" {
            "Google Chrome.app/Contents/MacOS/Google Chrome"
        } else if browser == "firefox" {
            "Firefox.app/Contents/MacOS/firefox"
        } else {
            "Microsoft Edge.app/Contents/MacOS/Microsoft Edge"
        };
        for root in [
            PathBuf::from("/Applications"),
            home_dir().join("Applications"),
        ] {
            let path = root.join(app);
            if path.is_file() {
                return Ok(path);
            }
        }
    }
    let _ = suffix;
    Err("没有找到此浏览器，请先安装所选的 Chrome、Microsoft Edge 或 Firefox。".into())
    }
}

pub fn home_dir() -> PathBuf {
    std::env::var_os(if cfg!(target_os = "windows") {
        "USERPROFILE"
    } else {
        "HOME"
    })
    .map(PathBuf::from)
    .unwrap_or_else(std::env::temp_dir)
}

pub fn timezone() -> Result<String, String> {
    #[cfg(target_os = "windows")]
    {
        output("tzutil.exe", &["/g"])
    }
    #[cfg(target_os = "macos")]
    {
        read_macos_timezone(
            Path::new("/etc/localtime"),
            40,
            std::time::Duration::from_millis(50),
        )
    }
    #[cfg(target_os = "linux")]
    {
        desktop_linux::timezone()
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err("当前系统不在支持范围内".into())
    }
}

#[cfg(target_os = "macos")]
fn read_macos_timezone(
    localtime: &Path,
    attempts: u32,
    delay: std::time::Duration,
) -> Result<String, String> {
    // systemsetup can return while timed is replacing the timezone symlink.
    // Wait for the real file; never treat a dangling link as verified settings.
    for attempt in 0..attempts.max(1) {
        match std::fs::canonicalize(localtime) {
            Ok(path) => {
                return path
                    .to_string_lossy()
                    .split_once("zoneinfo/")
                    .map(|(_, zone)| zone.to_owned())
                    .filter(|zone| !zone.is_empty())
                    .ok_or_else(|| format!("无法识别系统时区路径：{}", path.display()));
            }
            Err(error)
                if error.kind() == std::io::ErrorKind::NotFound && attempt + 1 < attempts =>
            {
                std::thread::sleep(delay);
            }
            Err(error) => {
                return Err(format!(
                    "无法读取系统时区 {}：{error}。系统可能仍在更新时区文件，请稍后重新检测。",
                    localtime.display()
                ))
            }
        }
    }
    unreachable!()
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct TimezoneOption {
    pub id: String,
    pub label: String,
    pub offset_minutes: i32,
}

pub fn timezone_catalog() -> Result<Vec<TimezoneOption>, String> {
    #[cfg(target_os = "macos")]
    let text = output("/usr/bin/osascript", &["-l", "JavaScript", "-e",
        "ObjC.import('Foundation'); JSON.stringify(ObjC.deepUnwrap($.NSTimeZone.knownTimeZoneNames).map(id => ({id:id,label:id,offsetMinutes:-Number($.NSTimeZone.timeZoneWithName($(id)).secondsFromGMT)/60})))"])?;
    #[cfg(target_os = "windows")]
    let text = output("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command",
        "[Console]::OutputEncoding=[Text.UTF8Encoding]::new(); @(Get-TimeZone -ListAvailable | ForEach-Object { @{ id=$_.Id; label=$_.DisplayName; offsetMinutes=[int](-$_.GetUtcOffset([datetime]::UtcNow).TotalMinutes) } }) | ConvertTo-Json -Compress"])?;
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    let text = "[]".to_string();
    #[cfg(not(target_os = "linux"))]
    let mut options: Vec<TimezoneOption> =
        serde_json::from_str(&text).map_err(|_| "无法解析系统支持的时区清单".to_string())?;
    #[cfg(target_os = "linux")]
    let mut options = desktop_linux::timezone_catalog();
    options.retain(|option| valid_timezone_name(&option.id));
    options.sort_by(|a, b| a.id.cmp(&b.id));
    if options.is_empty() {
        return Err("系统没有返回可用时区，请在系统设置中检查".into());
    }
    Ok(options)
}

fn valid_timezone_name(zone: &str) -> bool {
    !zone.is_empty()
        && zone.len() <= 128
        && zone
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || " /_+-().".contains(c))
}

fn custom_timezone_from_catalog(
    zone: &str,
    options: &[TimezoneOption],
) -> Result<TimezoneOption, String> {
    if !valid_timezone_name(zone) {
        return Err("目标时区格式无效".into());
    }
    options
        .iter()
        .find(|option| option.id == zone)
        .cloned()
        .ok_or_else(|| "目标时区不在系统支持的清单中，请搜索并选择有效时区".into())
}

fn resolve_timezone_target(target: &str) -> Result<TimezoneOption, String> {
    if let Some(zone) = target.strip_prefix("custom:") {
        if !valid_timezone_name(zone) {
            return Err("目标时区格式无效".into());
        }
        return custom_timezone_from_catalog(zone, &timezone_catalog()?);
    }
    let (id, offset_minutes) = match (target, cfg!(target_os = "windows")) {
        ("singapore", true) => ("Singapore Standard Time", -480),
        ("singapore", false) => ("Asia/Singapore", -480),
        ("utc", true) => ("UTC", 0),
        ("utc", false) => (
            if cfg!(target_os = "macos") {
                "GMT"
            } else {
                "Etc/UTC"
            },
            0,
        ),
        _ => return Err("目标时区不在允许列表中".into()),
    };
    Ok(TimezoneOption {
        id: id.into(),
        label: id.into(),
        offset_minutes,
    })
}

pub fn target_timezone(target: &str) -> Result<String, String> {
    Ok(resolve_timezone_target(target)?.id)
}

pub fn offset_minutes() -> Result<i32, String> {
    #[cfg(target_os = "windows")]
    {
        let text = output(
            "powershell.exe",
            &[
                "-NoProfile",
                "-NonInteractive",
                "-Command",
                "[int]-[DateTimeOffset]::Now.Offset.TotalMinutes",
            ],
        )?;
        text.parse::<i32>()
            .map_err(|_| "无法读取系统 UTC 偏移".into())
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let result = command("/bin/date").arg("+%z").env_remove("TZ").output().map_err(|_| "无法读取系统 UTC 偏移")?;
        if !result.status.success() { return Err("无法读取系统 UTC 偏移".into()); }
        let text = String::from_utf8_lossy(&result.stdout);
        let text = text.trim();
        let bytes = text.as_bytes();
        if bytes.len() != 5
            || !matches!(bytes[0], b'+' | b'-')
            || !bytes[1..].iter().all(u8::is_ascii_digit)
        {
            return Err("无法读取系统 UTC 偏移".into());
        }
        let hours = text[1..3].parse::<i32>().map_err(|e| e.to_string())?;
        let minutes = text[3..5].parse::<i32>().map_err(|e| e.to_string())?;
        Ok((hours * 60 + minutes) * if bytes[0] == b'+' { -1 } else { 1 })
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    {
        Err("不支持当前系统".into())
    }
}

pub fn set_timezone(zone: &str) -> Result<(), String> {
    // Values come from an OS-read snapshot or our fixed target, never arbitrary UI input.
    if !valid_timezone_name(zone) {
        return Err("备份时区格式无效".into());
    }
    // systemsetup uses Foundation timezone names, which omit Etc/UTC.
    // Accept old journal snapshots but send the native zero-offset name.
    #[cfg(target_os = "macos")]
    let zone = if is_macos_utc_alias(zone) {
        "GMT"
    } else {
        zone
    };
    #[cfg(target_os = "windows")]
    {
        let direct = command("tzutil.exe")
            .args(["/s", zone])
            .output()
            .map_err(|e| format!("无法启动时区工具：{e}"))?;
        if !direct.status.success() {
            elevate_timezone(zone).map_err(|e| {
                format!("{e}\n首次尝试：{}", command_failure("tzutil.exe", &direct))
            })?;
        }
    }
    #[cfg(target_os = "macos")]
    {
        let shell = format!("/usr/sbin/systemsetup -settimezone '{zone}'");
        let script = format!("do shell script \"{shell}\" with administrator privileges");
        output("/usr/bin/osascript", &["-e", &script])?;
    }
    #[cfg(target_os = "linux")]
    {
        // systemd requests authorization through the desktop's PolicyKit agent.
        // The main application and browsers remain ordinary user processes.
        custom_timezone_from_catalog(zone, &timezone_catalog()?)?;
        output("/usr/bin/timedatectl", &["set-timezone", zone])?;
    }
    if !timezone_names_match(&timezone()?, zone) {
        return Err(
            "系统时区修改后未通过验证。自动时区或组织策略可能覆盖设置，请到系统设置检查。".into(),
        );
    }
    Ok(())
}

fn is_macos_utc_alias(zone: &str) -> bool {
    matches!(zone, "GMT" | "UTC" | "Etc/UTC" | "Etc/GMT")
}

pub fn timezone_names_match(current: &str, expected: &str) -> bool {
    current == expected
        || (cfg!(target_os = "macos")
            && is_macos_utc_alias(current)
            && is_macos_utc_alias(expected))
}

pub fn verify_timezone_target(target: &str) -> Result<String, String> {
    let expected = target_timezone(target)?;
    let current = timezone()?;
    let offset = offset_minutes()?;
    verify_timezone_values(target, &current, offset)?;
    Ok(format!(
        "系统时区 {expected} · 实际偏移 {}。请完全退出专用浏览器，再重新打开复检。",
        timezone_offset_label(offset)
    ))
}
fn verify_timezone_values(target: &str, current: &str, offset: i32) -> Result<(), String> {
    let resolved = resolve_timezone_target(target)?;
    verify_timezone_option(&resolved, current, offset)
}

fn verify_timezone_option(
    resolved: &TimezoneOption,
    current: &str,
    offset: i32,
) -> Result<(), String> {
    let expected = &resolved.id;
    let expected_offset = resolved.offset_minutes;
    if !timezone_names_match(current, expected) || offset != expected_offset {
        return Err(format!("时区 / 偏移核验失败：当前 {current}，getTimezoneOffset={offset}；目标 {expected}，预期 {expected_offset}。请检查自动时区或系统策略。"));
    }
    Ok(())
}

fn timezone_offset_label(offset: i32) -> String {
    let east = -offset;
    let sign = if east >= 0 { "+" } else { "-" };
    let minutes = east.abs() % 60;
    if minutes == 0 {
        format!("UTC{sign}{}", east.abs() / 60)
    } else {
        format!("UTC{sign}{}:{minutes:02}", east.abs() / 60)
    }
}

fn profile_argument_matches(line: &str, profile: &str) -> bool {
    let mut args = Vec::new();
    let mut word = String::new();
    let mut quoted = false;
    for ch in line.chars() {
        if ch == '"' {
            quoted = !quoted;
        } else if ch.is_whitespace() && !quoted {
            if !word.is_empty() {
                args.push(std::mem::take(&mut word));
            }
        } else {
            word.push(ch);
        }
    }
    if !word.is_empty() {
        args.push(word);
    }
    let normalize = |s: &str| s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
    let expected = normalize(profile);
    args.iter().enumerate().any(|(i, arg)| {
        let path = arg
            .strip_prefix("--user-data-dir=")
            .or_else(|| arg.strip_prefix("--profile="))
            .or_else(|| {
                if ["--user-data-dir", "--profile", "-profile"].contains(&arg.as_str()) {
                    args.get(i + 1).map(String::as_str)
                } else {
                    None
                }
            });
        path.is_some_and(|p| normalize(p) == expected)
    })
}

pub fn ensure_profile_closed(profile: &Path) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let processes = output("powershell.exe", &["-NoProfile", "-NonInteractive", "-Command", "$ErrorActionPreference='Stop'; Get-CimInstance Win32_Process -Filter \"Name='chrome.exe' OR Name='msedge.exe' OR Name='firefox.exe'\" | ForEach-Object { $_.CommandLine }"])?;
    #[cfg(target_os = "macos")]
    let processes = output("/bin/ps", &["-ax", "-o", "command="])?;
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let processes = String::new();
    #[cfg(target_os = "linux")]
    let running = !profile_process_ids(profile)?.is_empty();
    #[cfg(not(target_os = "linux"))]
    let running = if cfg!(target_os = "windows") {
        processes
            .lines()
            .any(|line| profile_argument_matches(line, &profile.to_string_lossy()))
    } else {
        processes.contains(profile.to_string_lossy().as_ref())
    };
    if running {
        return Err("专用浏览器仍在运行。请保存未完成的输入，并从该专用窗口的浏览器菜单选择“退出”；仅关闭标签页可能保留后台进程。日常浏览器无需退出。".into());
    }
    #[cfg(target_os = "windows")]
    if profile.join("parent.lock").exists() {
        use std::os::windows::fs::OpenOptionsExt;
        // Firefox leaves a stale parent.lock after exit; existence alone is not a lock.
        let lock = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .share_mode(0)
            .open(profile.join("parent.lock"));
        if let Err(e) = lock {
            return Err(format!(
                "Firefox 专用配置仍被锁定或无法读取，请正常退出后重试：{e}"
            ));
        }
    }
    #[cfg(any(target_os = "macos", target_os = "linux"))]
    singleton::check(profile)?;
    #[cfg(target_os = "linux")]
    linux::check_firefox_lock(profile)?;
    // Unix lock files can survive normal shutdown. Check ownership, not existence.
    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    if profile.join("SingletonLock").symlink_metadata().is_ok() {
        return Err(
            "专用浏览器仍被锁定。请关闭窗口；若已退出，请重启浏览器后正常退出再重试。".into(),
        );
    }
    Ok(())
}

pub fn open_timezone_settings() -> Result<(), String> {
    #[cfg(target_os = "windows")]
    {
        command("explorer.exe")
            .arg("ms-settings:dateandtime")
            .spawn()
            .map_err(|e| e.to_string())?;
        return Ok(());
    }
    #[cfg(target_os = "macos")]
    {
        // The same Date & Time link used by the Apple macOS user guide.
        let status = command("open")
            .arg("x-apple.systempreferences:com.apple.Date-Time-Settings.extension")
            .status()
            .map_err(|_| "请在系统设置中打开日期与时间。")?;
        if !status.success() {
            return Err("请在系统设置中打开日期与时间。".into());
        }
        return Ok(());
    }
    #[cfg(target_os = "linux")]
    {
        command("gnome-control-center").arg("datetime").spawn()
            .map_err(|_| "请在 Ubuntu 系统设置中打开日期与时间。")?;
        return Ok(());
    }
    #[cfg(not(any(target_os = "windows", target_os = "macos", target_os = "linux")))]
    Err("请在系统设置中打开日期与时间。".into())
}

pub fn open_telegram_group() -> Result<(), String> {
    open_external("https://t.me/nodecloak_official")
}

pub fn open_download_page() -> Result<(), String> {
    open_external("https://claudedone.com/#download")
}

pub fn open_firefox_download() -> Result<(), String> {
    open_external("https://www.firefox.com/en-US/download/all/desktop-release/")
}

pub(crate) fn open_external(url: &str) -> Result<(), String> {
    #[cfg(target_os = "windows")]
    let mut opener = {
        let mut cmd = command("rundll32.exe");
        cmd.args(["url.dll,FileProtocolHandler", url]);
        cmd
    };
    #[cfg(target_os = "macos")]
    let mut opener = {
        let mut cmd = command("open");
        cmd.arg(url);
        cmd
    };
    #[cfg(not(any(target_os = "windows", target_os = "macos")))]
    let mut opener = {
        let mut cmd = command("xdg-open");
        cmd.arg(url);
        cmd
    };
    opener.spawn().map_err(|e| format!("无法打开浏览器：{e}"))?;
    Ok(())
}

#[cfg(test)]
mod profile_tests {
    use super::*;
    #[test]
    fn settings_endpoint_survives_non_utf8_browser_diagnostics() {
        let mut bytes = b"Windows browser diagnostic: \xff\xfe\r\n".to_vec();
        bytes.extend_from_slice(b"DevTools listening on ws://127.0.0.1:41234/devtools/browser/abc-123\r\n");
        let result = read_manual_endpoint(&mut std::io::Cursor::new(bytes), 41234).unwrap();
        assert_eq!(result, Some("41234\n/devtools/browser/abc-123\n".into()));
    }
    #[test]
    fn settings_endpoint_skips_oversize_lines_and_rejects_other_ports() {
        let mut bytes = vec![b'x'; 128 * 1024];
        bytes.extend_from_slice(b"DevTools listening on ws://127.0.0.1:41234/devtools/browser/forged\n");
        bytes.extend_from_slice(b"DevTools listening on ws://127.0.0.1:41235/devtools/browser/wrong\n");
        assert_eq!(read_manual_endpoint(&mut std::io::Cursor::new(bytes.clone()),41234).unwrap(),None);
        bytes.extend_from_slice(b"DevTools listening on ws://127.0.0.1:41234/devtools/browser/real\n");
        assert_eq!(read_manual_endpoint(&mut std::io::Cursor::new(bytes),41234).unwrap(),Some("41234\n/devtools/browser/real\n".into()));
    }
    #[test]
    fn settings_endpoint_accepts_only_the_launched_loopback_port() {
        assert_eq!(
            manual_endpoint_marker("DevTools listening on ws://127.0.0.1:41234/devtools/browser/abc-123", 41234),
            Some("41234\n/devtools/browser/abc-123\n".into())
        );
        for endpoint in [
            "ws://127.0.0.1:41235/devtools/browser/abc-123",
            "ws://127.0.0.1:0/devtools/browser/abc-123",
            "ws://localhost:41234/devtools/browser/abc-123",
            "ws://example.com:41234/devtools/browser/abc-123",
            "wss://127.0.0.1:41234/devtools/browser/abc-123",
            "ws://user@127.0.0.1:41234/devtools/browser/abc-123",
            "ws://127.0.0.1:41234/devtools/page/abc-123",
            "ws://127.0.0.1:41234/devtools/browser/",
            "ws://127.0.0.1:41234/devtools/browser/abc-123?extra=1",
            "ws://127.0.0.1:41234/devtools/browser/abc-123#extra",
            "ws://127.0.0.1:41234/devtools/browser/abc/123",
        ] {
            assert!(manual_endpoint_marker(&format!("DevTools listening on {endpoint}"), 41234).is_none(), "{endpoint}");
        }
        assert!(manual_endpoint_marker("unrelated stderr output", 41234).is_none());
    }
    #[test]
    fn firefox_profile_is_created_and_checked_without_changing_existing_data() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory
            .path()
            .join("用户配置 with spaces/browser-firefox");
        let prepared = prepare_firefox_profile(&profile).unwrap();
        assert!(prepared.is_absolute());
        assert!(prepared.is_dir());
        std::fs::write(profile.join("prefs.js"), "keep existing preferences").unwrap();
        std::fs::write(profile.join("cookies.sqlite"), "keep login data").unwrap();
        prepare_firefox_profile(&profile).unwrap();
        assert_eq!(
            std::fs::read_to_string(profile.join("cookies.sqlite")).unwrap(),
            "keep login data"
        );
        assert_eq!(std::fs::read_dir(profile).unwrap().count(), 2);
        let blocked = directory.path().join("not-a-directory");
        std::fs::write(&blocked, "keep").unwrap();
        let error = prepare_firefox_profile(&blocked).unwrap_err();
        assert!(error.contains("配置目录"));
        assert_eq!(std::fs::read_to_string(blocked).unwrap(), "keep");
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn timezone_read_waits_for_a_temporarily_missing_zone_file() {
        let directory = tempfile::tempdir().unwrap();
        let zone = directory.path().join("zoneinfo/Asia/Singapore");
        std::fs::create_dir_all(zone.parent().unwrap()).unwrap();
        let localtime = directory.path().join("localtime");
        std::os::unix::fs::symlink(&zone, &localtime).unwrap();
        let writer = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_millis(40));
            std::fs::write(zone, b"test zone data").unwrap();
        });
        assert_eq!(
            read_macos_timezone(&localtime, 40, std::time::Duration::from_millis(10)).unwrap(),
            "Asia/Singapore"
        );
        writer.join().unwrap();
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn timezone_read_reports_persistent_missing_files_with_context() {
        let directory = tempfile::tempdir().unwrap();
        let localtime = directory.path().join("localtime");
        std::os::unix::fs::symlink(directory.path().join("zoneinfo/GMT"), &localtime).unwrap();
        let error =
            read_macos_timezone(&localtime, 2, std::time::Duration::from_millis(1)).unwrap_err();
        assert!(error.contains("无法读取系统时区"));
        assert!(error.contains(localtime.to_str().unwrap()));
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn firefox_parent_lock_detects_occupation_and_allows_stale_file() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let file = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .share_mode(0)
            .open(dir.path().join("parent.lock"))
            .unwrap();
        assert!(ensure_profile_closed(dir.path()).is_err());
        drop(file);
        assert!(ensure_profile_closed(dir.path()).is_ok());
        assert!(dir.path().join("parent.lock").exists());
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn closing_waits_for_the_profile_lock_after_the_main_process_exits() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir = tempfile::tempdir().unwrap();
        let lock = std::fs::OpenOptions::new().create(true).write(true).truncate(true)
            .share_mode(0).open(dir.path().join("parent.lock")).unwrap();
        let release = std::thread::spawn(move || {
            std::thread::sleep(std::time::Duration::from_secs(3));
            drop(lock);
        });
        let result = close_profile(dir.path());
        release.join().unwrap();
        assert!(result.is_ok(), "shutdown cleanup must finish within the normal close deadline: {result:?}");
        assert!(ensure_profile_closed(dir.path()).is_ok());
    }
    #[cfg(target_os = "windows")]
    #[test]
    fn forced_close_waits_for_auxiliary_profile_lock_release() {
        use std::os::windows::fs::OpenOptionsExt;
        let dir=tempfile::tempdir().unwrap();
        let lock=std::fs::OpenOptions::new().create(true).write(true).truncate(true).share_mode(0).open(dir.path().join("parent.lock")).unwrap();
        let release=std::thread::spawn(move||{std::thread::sleep(std::time::Duration::from_secs(2));drop(lock);});
        let result=force_close_profile(dir.path());release.join().unwrap();
        assert!(result.is_ok(),"force-close reported failure before auxiliary shutdown finished: {result:?}");
    }
    #[test]
    fn utc_offset_is_verified_even_when_timezone_name_matches() {
        assert!(verify_timezone_values("utc", &target_timezone("utc").unwrap(), 0).is_ok());
        assert!(verify_timezone_values("utc", &target_timezone("utc").unwrap(), -480).is_err());
        assert!(
            verify_timezone_values("singapore", &target_timezone("singapore").unwrap(), -480)
                .is_ok()
        );
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn macos_utc_target_and_legacy_snapshots_use_native_aliases() {
        assert_eq!(target_timezone("utc").unwrap(), "GMT");
        for alias in ["GMT", "UTC", "Etc/UTC", "Etc/GMT"] {
            assert!(verify_timezone_values("utc", alias, 0).is_ok());
            assert!(verify_timezone_values("utc", alias, -480).is_err());
            assert!(timezone_names_match(alias, "GMT"));
        }
        assert!(!timezone_names_match("Europe/London", "GMT"));
        assert!(!timezone_names_match("Asia/Shanghai", "GMT"));
    }
    #[test]
    fn rejects_timezone_quote_injection_before_running_any_system_command() {
        for value in ["UTC'; anything", "UTC\"", "$(anything)", "", "UTC\n"] {
            assert!(set_timezone(value).is_err());
        }
    }
    #[test]
    fn custom_timezones_require_system_membership_and_correct_offsets() {
        let options = vec![TimezoneOption {
            id: "Asia/Kolkata".into(),
            label: "Asia/Kolkata".into(),
            offset_minutes: -330,
        }];
        let target = custom_timezone_from_catalog("Asia/Kolkata", &options).unwrap();
        assert!(verify_timezone_option(&target, "Asia/Kolkata", -330).is_ok());
        assert!(verify_timezone_option(&target, "Asia/Kolkata", -480).is_err());
        assert!(verify_timezone_option(&target, "GMT", -330).is_err());
        assert_eq!(timezone_offset_label(-330), "UTC+5:30");
        for zone in [
            "Unknown/Zone",
            "Asia/Kolkata'; anything",
            "$(anything)",
            "",
            "GMT\n",
        ] {
            assert!(custom_timezone_from_catalog(zone, &options).is_err());
        }
        assert!(valid_timezone_name("W. Europe Standard Time"));
        assert!(valid_timezone_name("Pacific Standard Time (Mexico)"));
    }
    #[cfg(target_os = "macos")]
    #[test]
    fn native_catalog_supports_a_custom_timezone_without_changing_the_system() {
        let target = resolve_timezone_target("custom:Asia/Tokyo").unwrap();
        assert_eq!(target.id, "Asia/Tokyo");
        assert_eq!(target.offset_minutes, -540);
        assert!(resolve_timezone_target("custom:Unknown/Zone").is_err());
    }
    #[test]
    fn matches_only_the_exact_profile_argument() {
        let path = r"C:\Users\A User\AppData\browser-edge";
        assert!(profile_argument_matches(
            &format!(r#"firefox.exe --no-remote --profile "{path}""#),
            path
        ));
        assert!(!profile_argument_matches(
            &format!(r#"firefox.exe --profile "{path}-other""#),
            path
        ));
        assert!(profile_argument_matches(
            &format!(r#"msedge.exe "--user-data-dir={path}" --no-first-run"#),
            path
        ));
        assert!(profile_argument_matches(
            &format!(r#"msedge.exe --user-data-dir "{path}""#),
            path
        ));
        assert!(!profile_argument_matches(
            &format!(r#"msedge.exe "--user-data-dir={path}-other""#),
            path
        ));
        assert!(!profile_argument_matches(
            &format!(r#"msedge.exe "https://example.invalid/{path}""#),
            path
        ));
        assert!(!profile_argument_matches(
            "msedge.exe --profile-directory=Default",
            path
        ));
    }
    #[cfg(windows)]
    #[test]
    fn staged_profile_process_matching_accepts_windows_extended_path_spelling() {
        let processes = vec![
            (
                42,
                vec![
                    "chrome.exe".into(),
                    "--user-data-dir=C:\\temp\\profile".into(),
                ],
            ),
            (
                43,
                vec![
                    "chrome.exe".into(),
                    "--user-data-dir=C:\\temp\\profile-other".into(),
                ],
            ),
        ];
        assert_eq!(
            profile_ids_from(&processes, Path::new(r"\\?\C:\temp\profile")),
            vec![42]
        );
    }
}

pub fn spawn_browser(browser: &str, profile: &Path, url: &str) -> Result<(), String> {
    spawn_browser_with_proxy(browser, profile, url, "system", None)
}
pub fn spawn_browser_with_proxy(
    browser: &str,
    profile: &Path,
    url: &str,
    mode: &str,
    relay: Option<u16>,
) -> Result<(), String> {
    spawn_browser_impl(browser, profile, url, mode, relay, None, false, None, false)
}
pub(crate) fn spawn_resuming_browser(browser:&str, profile:&Path, mode:&str, relay:Option<u16>)->Result<(),String>{
    spawn_browser_impl(browser, profile, "about:blank", mode, relay, None, false, None, true)
}
pub fn spawn_controlled_browser(
    browser: &str,
    profile: &Path,
    mode: &str,
    relay: Option<u16>,
) -> Result<(), String> {
    ensure_profile_closed(profile)?;
    let marker = profile.join(if browser == "firefox" {
        "WebDriverBiDiServer.json"
    } else {
        "DevToolsActivePort"
    });
    if marker.exists() {
        std::fs::remove_file(&marker).map_err(|_| "无法清理旧的浏览器控制标记")?;
    }
    spawn_browser_impl(browser, profile, "about:blank", mode, relay, Some(0), false, None, false)
}
pub fn spawn_settings_browser(browser: &str, profile: &Path, mode: &str, relay: Option<u16>) -> Result<(), String> {
    if browser == "firefox" {
        return spawn_controlled_browser(browser, profile, mode, relay);
    }
    ensure_profile_closed(profile)?;
    let marker = profile.join("DevToolsActivePort");
    crate::profiles::guard_tree(&marker)?;
    if marker.exists() {
        std::fs::remove_file(&marker).map_err(|_| "无法清理旧的浏览器控制标记")?;
    }
    let reservation = std::net::TcpListener::bind("127.0.0.1:0").map_err(|_| "无法分配本地浏览器配置端口")?;
    let port = reservation.local_addr().map_err(|_| "无法读取本地配置端口")?.port();
    drop(reservation);
    spawn_browser_impl(browser, profile, "about:blank", mode, relay, Some(port), false, None, false)
}
pub(crate) fn spawn_firefox_addon_setup(profile: &Path, mode: &str, relay: Option<u16>) -> Result<(), String> {
    spawn_browser_impl("firefox", profile, "about:blank", mode, relay, None, true, None, false)
}
pub(crate) fn spawn_firefox_region_setup(profile: &Path, mode: &str, relay: Option<u16>, port: u16) -> Result<(), String> {
    spawn_browser_impl("firefox", profile, "about:blank", mode, relay, None, false, Some(port), false)
}

fn manual_endpoint_marker(line: &str, expected_port: u16) -> Option<String> {
    let endpoint = line.trim().strip_prefix("DevTools listening on ")?;
    let url = reqwest::Url::parse(endpoint).ok()?;
    let id = url.path().strip_prefix("/devtools/browser/")?;
    if url.scheme() != "ws" || url.host_str() != Some("127.0.0.1")
        || url.port() != Some(expected_port) || expected_port == 0
        || !url.username().is_empty() || url.password().is_some()
        || url.query().is_some() || url.fragment().is_some()
        || id.is_empty() || !id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        return None;
    }
    Some(format!("{expected_port}\n{}\n", url.path()))
}

fn read_manual_endpoint(reader: &mut impl std::io::BufRead, port: u16) -> std::io::Result<Option<String>> {
    use std::io::{BufRead, Read};
    const MAX_LINE: u64 = 64 * 1024;
    let mut discarding = false;
    loop {
        // Browser diagnostics can use the local Windows code page. A malformed
        // unrelated line must not terminate discovery of the ASCII endpoint.
        let mut bytes = Vec::new();
        let count = (&mut *reader).take(MAX_LINE).read_until(b'\n', &mut bytes)?;
        if count == 0 { return Ok(None); }
        let ended = bytes.ends_with(b"\n");
        if discarding { discarding = !ended; continue; }
        if !ended && count as u64 == MAX_LINE { discarding = true; continue; }
        if let Ok(line) = std::str::from_utf8(&bytes) {
            if let Some(marker) = manual_endpoint_marker(line, port) { return Ok(Some(marker)); }
        }
    }
}
fn spawn_browser_impl(
    browser: &str,
    profile: &Path,
    url: &str,
    mode: &str,
    relay: Option<u16>,
    debug_port: Option<u16>,
    marionette: bool,
    rdp_port: Option<u16>,
    resume: bool,
) -> Result<(), String> {
    let exe = browser_path(browser)?;
    let mut cmd = command(exe.to_str().ok_or("浏览器路径格式无效")?);
    #[cfg(target_os = "linux")]
    {
        // XWayland is available in Ubuntu's Wayland session. Its native window
        // protocols let us focus/close just this profile and honor save prompts.
        if browser == "firefox" {
            cmd.env("MOZ_ENABLE_WAYLAND", "0").env("GDK_BACKEND", "x11");
        } else {
            cmd.arg("--ozone-platform=x11");
        }
        // AppImage runtime libraries belong to NodeCloak, not external browsers.
        if std::env::var_os("APPIMAGE").is_some() {
            for key in ["LD_LIBRARY_PATH", "LD_PRELOAD", "APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
                cmd.env_remove(key);
            }
        }
    }
    // Installed-browser integration fixtures opt into the browser's real
    // headless mode on desktops that suspend background geolocation.
    #[cfg(test)]
    if std::env::var_os("NODECLOAK_TEST_HEADLESS_BROWSERS").is_some() {
        cmd.arg(if browser == "firefox" {
            "--headless"
        } else {
            "--headless=new"
        });
    }
    if browser != "firefox" {
        let file = profile.join("nodecloak-runtime-settings.json");
        if file.exists() {
            crate::profiles::guard_tree(&file)?;
            let bytes = std::fs::read(&file).map_err(|_| "无法读取浏览器运行设置")?;
            if bytes.len() > 16384 {
                return Err("浏览器运行设置过大".into());
            }
            let options: crate::profiles::AdvancedSettings =
                serde_json::from_slice(&bytes).map_err(|_| "浏览器运行设置无效")?;
            if options.block_webgl {
                cmd.args(["--disable-webgl", "--disable-webgl2"]);
            }
            if options.block_webgpu {
                cmd.arg("--disable-features=WebGPUService");
            }
            if options.block_speech {
                cmd.arg("--disable-blink-features=ScriptedSpeechSynthesis");
            }
        }
    }
    if let Some(port) = debug_port {
        cmd.arg(format!("--remote-debugging-port={port}"));
        if browser != "firefox" {
            cmd.args([
                "--remote-debugging-address=127.0.0.1",
                "--disable-background-mode",
            ]);
        }
    }

    if browser == "firefox" {
        ensure_profile_closed(profile)?;
        let profile = prepare_firefox_profile(profile)?;
        crate::firefox::prepare_startup(&profile)?;
        cmd.env("NODECLOAK_FIREFOX_PROFILE", &profile);
        if marionette { cmd.arg("--marionette"); }
        if let Some(port) = rdp_port { cmd.args(["--start-debugger-server", &port.to_string()]); }
        let kind = if relay.is_some() {
            1
        } else if mode == "direct" {
            0
        } else {
            5
        };
        let mut values = vec![
            ("network.proxy.type", serde_json::json!(kind)),
            (
                "network.proxy.http",
                serde_json::json!(if relay.is_some() { "127.0.0.1" } else { "" }),
            ),
            (
                "network.proxy.ssl",
                serde_json::json!(if relay.is_some() { "127.0.0.1" } else { "" }),
            ),
            (
                "network.proxy.http_port",
                serde_json::json!(relay.unwrap_or(0)),
            ),
            (
                "network.proxy.ssl_port",
                serde_json::json!(relay.unwrap_or(0)),
            ),
            ("network.proxy.socks", serde_json::json!("")),
            ("network.proxy.socks_port", serde_json::json!(0)),
            ("network.proxy.no_proxies_on", serde_json::json!("")),
            ("network.proxy.socks_remote_dns", serde_json::json!(true)),
            ("network.proxy.failover_direct", serde_json::json!(false)),
        ];
        if relay.is_some() {
            values.push(("media.peerconnection.enabled", serde_json::json!(false)));
        }
        let changes = values
            .into_iter()
            .map(|(key, value)| {
                Ok((
                    key.to_owned(),
                    crate::firefox::snapshot(&profile, key)?,
                    serde_json::json!({"user":value,"runtime":value}),
                ))
            })
            .collect::<Result<Vec<_>, String>>()?;
        crate::firefox::apply(&profile, &changes, false)?;
        // Do not inherit another Firefox session's profile/restart selection.
        cmd.env_remove("XRE_PROFILE_PATH")
            .env_remove("XRE_PROFILE_LOCAL_PATH")
            .env_remove("XRE_RESTARTED_BY_PROFILE_MANAGER")
            .args(["--no-remote", "--profile"])
            .arg(profile);
        if !resume {cmd.args(["--new-window",url]);}
    } else {
        if let Some(port) = relay {
            cmd.arg(format!("--proxy-server=http://127.0.0.1:{port}"))
                .args(["--proxy-bypass-list=<-loopback>", "--disable-quic"]);
        } else if mode == "direct" {
            cmd.arg("--no-proxy-server");
        }
        cmd.arg(format!("--user-data-dir={}", profile.display()))
            .args([
                "--profile-directory=Default",
                "--no-first-run",
                "--no-default-browser-check",
            ]);
        if resume {cmd.arg("--restore-last-session");} else {cmd.arg(url);}
    }
    let manual_endpoint = debug_port.filter(|port| *port > 0 && browser != "firefox");
    let mut child = cmd.stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(if manual_endpoint.is_some() { Stdio::piped() } else { Stdio::null() })
        .spawn()
        .map_err(|e| format!("无法打开浏览器：{e}"))?;
    let stderr=child.stderr.take();
    let launcher_exit=std::sync::Arc::new(std::sync::Mutex::new(None));
    let observed_exit=launcher_exit.clone();
    // Retain and reap only the child we launched. Dropping std::process::Child
    // does not reap Unix children; a zombie can keep a profile appearing live.
    // Reap independently of stderr, which descendant helpers may keep open.
    std::thread::spawn(move||{
        let status=child.wait().ok().and_then(|status|status.code());
        if let Ok(mut observed)=observed_exit.lock(){*observed=status;}
    });
    if let Some(port) = manual_endpoint {
        let stderr = stderr.ok_or("无法读取浏览器配置接口")?;
        let marker = profile.join("DevToolsActivePort");
        let profile = profile.to_path_buf();
        let browser = browser.to_owned();
        // Read the endpoint from this child, never from an unverified local HTTP
        // server. Only the validated marker is persisted; raw stderr is discarded.
        std::thread::spawn(move || {
            let mut reader = std::io::BufReader::new(stderr);
            let mut discovered = read_manual_endpoint(&mut reader, port);
            #[cfg(windows)]
            if !matches!(&discovered, Ok(Some(_))) {
                crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_handoff","launcher output unavailable; checking profile-owned listener");
                discovered = endpoint::discover(&profile, port).map(Some)
                    .map_err(|error| {
                        crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_discovery_failed",&error);
                        std::io::Error::other(error)
                    });
            }
            match discovered {
                Ok(Some(value)) => {
                    let published = crate::profiles::guard_tree(&marker)
                        .and_then(|_| crate::engine::atomic_write(&marker,value.as_bytes()));
                    match published {
                        Ok(()) => crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_published","validated child endpoint published"),
                        Err(_) => crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_publish_failed","validated endpoint could not be saved"),
                    }
                }
                Ok(None) => {
                    let status = launcher_exit.lock().ok().and_then(|status|*status);
                    crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_missing",&format!("child stderr ended without a valid endpoint; launcher_exit={status:?}"));
                }
                Err(error) => crate::browser_control::log_startup_endpoint(&profile,&browser,"endpoint_read_failed",&format!("browser stderr read failed; kind={:?}, os_code={:?}",error.kind(),error.raw_os_error())),
            }
            // Keep draining the pipe without retaining browser text or page data.
            let _ = std::io::copy(&mut reader,&mut std::io::sink());
        });
    }
    Ok(())
}

pub type BrowserProcess = (u32, Vec<String>);
pub fn browser_processes() -> Vec<BrowserProcess> {
    let system = sysinfo::System::new_all();
    system
        .processes()
        .values()
        .filter(|p| {
            let name = p.name().to_string_lossy().to_lowercase();
            ["chrome", "edge", "firefox"]
                .iter()
                .any(|b| name.contains(b))
        })
        .map(|p| {
            (
                p.pid().as_u32(),
                p.cmd()
                    .iter()
                    .map(|s| s.to_string_lossy().into_owned())
                    .collect(),
            )
        })
        .collect()
}
pub fn profile_ids_from(processes: &[BrowserProcess], profile: &Path) -> Vec<u32> {
    let normalize = |s: &str| {
        if cfg!(windows) {
            let path = s.replace('/', "\\").trim_end_matches('\\').to_lowercase();
            if let Some(unc) = path.strip_prefix(r"\\?\unc\") {
                format!(r"\\{unc}")
            } else {
                path.strip_prefix(r"\\?\").unwrap_or(&path).to_owned()
            }
        } else {
            s.trim_end_matches('/').to_owned()
        }
    };
    let expected = normalize(&profile.to_string_lossy());
    processes
        .iter()
        .filter(|(_, args)| {
            !args.iter().any(|s| s.starts_with("--type="))
                && args.iter().enumerate().any(|(i, arg)| {
                    let path = arg
                        .strip_prefix("--user-data-dir=")
                        .or_else(|| arg.strip_prefix("--profile="))
                        .or_else(|| {
                            if ["--user-data-dir", "--profile", "-profile"].contains(&arg.as_str())
                            {
                                args.get(i + 1).map(String::as_str)
                            } else {
                                None
                            }
                        });
                    path.is_some_and(|path| normalize(path) == expected)
                })
        })
        .map(|(id, _)| *id)
        .collect()
}
pub fn profile_process_ids(profile: &Path) -> Result<Vec<u32>, String> {
    Ok(profile_ids_from(&browser_processes(), profile))
}
#[cfg(windows)]
unsafe extern "system" fn profile_window(
    hwnd: windows_sys::Win32::Foundation::HWND,
    state: isize,
) -> i32 {
    use windows_sys::Win32::UI::WindowsAndMessaging::*;
    let state = &*(state as *const (Vec<u32>, bool));
    let mut pid = 0;
    GetWindowThreadProcessId(hwnd, &mut pid);
    if state.0.contains(&pid) && IsWindowVisible(hwnd) != 0 {
        if state.1 {
            PostMessageW(hwnd, WM_CLOSE, 0, 0);
        } else {
            ShowWindow(hwnd, SW_RESTORE);
            SetForegroundWindow(hwnd);
        }
    }
    1
}
pub fn focus_profile(profile: &Path) -> Result<(), String> {
    let ids = profile_process_ids(profile)?;
    if ids.is_empty() {
        return Err("副本已经关闭，请重新启动".into());
    }
    #[cfg(windows)]
    {
        let state = (ids, false);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                Some(profile_window),
                &state as *const _ as isize,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
        output("/usr/bin/osascript",&["-e",&format!("tell application \"System Events\" to set frontmost of first process whose unix id is {} to true",ids[0])])?;
    }
    #[cfg(target_os = "linux")]
    linux::focus(&ids)?;
    Ok(())
}
pub fn close_profile(profile: &Path) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    {
        return quit::wait(std::time::Duration::from_secs(8),
            || profile_process_ids(profile),
            |pid| linux::request_close(pid),
            || ensure_profile_closed(profile));
    }
    #[cfg(windows)]
    {
        let ids = profile_process_ids(profile)?;
        let state = (ids, true);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                Some(profile_window),
                &state as *const _ as isize,
            );
        }
    }
    #[cfg(target_os = "macos")]
    {
    let closed=quit::wait(std::time::Duration::from_secs(8),||profile_process_ids(profile),|pid|{
        // SIGTERM does not reliably run Firefox's storage shutdown phases.
        // Address only this profile's main PID, never every app of that name.
        let script = format!("ObjC.import('AppKit'); const app=$.NSRunningApplication.runningApplicationWithProcessIdentifier({pid}); let result='unregistered';if(!app.isNil()){{result='finishedLaunching='+app.finishedLaunching;if(!app.terminate) throw new Error('Browser refused normal quit');result+=';requested';}}result;");
        let quit = output("/usr/bin/osascript", &["-l", "JavaScript", "-e", &script]);
        match quit {
            Ok(status) => {
                crate::browser_control::log_startup_endpoint(profile,"browser","normal_quit_request",&format!("pid={pid}; {status}"));
                #[cfg(test)] eprintln!("native normal quit pid={pid}: {status}");
                match status.as_str(){
                    "unregistered"=>Ok(false),
                    "finishedLaunching=true;requested"|"finishedLaunching=false;requested"=>Ok(true),
                    _=>Err("无法确认浏览器正常退出请求是否已发送".into())
                }
            },
            Err(error) if profile_process_ids(profile)?.contains(&pid)=>Err(error),
            Err(_)=>Ok(true),
        }
    },||ensure_profile_closed(profile));
    if let Err(error)=&closed {
        crate::browser_control::log_startup_endpoint(profile,"browser","normal_quit_wait_failed",error);
        #[cfg(test)] {
            let ids=profile_process_ids(profile).unwrap_or_default();
            eprintln!("native normal quit timeout main_ids={ids:?}");
            if !ids.is_empty(){eprintln!("native normal quit remaining: {:?}",output("/bin/ps", &["-o","pid=,ppid=,stat=,comm=","-p",&ids.iter().map(u32::to_string).collect::<Vec<_>>().join(",")]));}
        }
    }
    return closed;
    }
    #[cfg(not(any(target_os="macos", target_os="linux")))]
    wait_profile_closed(profile, std::time::Duration::from_secs(8))
}

pub(crate) fn wait_profile_closed(profile: &Path, timeout: std::time::Duration) -> Result<(), String> {
    let deadline = std::time::Instant::now() + timeout;
    loop {
        // The main process can disappear before helper processes or the
        // Firefox profile lock are released. Wait for the complete closed
        // state, preserving the last concrete error if shutdown stays blocked.
        match ensure_profile_closed(profile) {
            Ok(()) => return Ok(()),
            Err(error) if std::time::Instant::now() >= deadline => return Err(error),
            Err(_) => {}
        }
        std::thread::sleep(std::time::Duration::from_millis(500)
            .min(deadline.saturating_duration_since(std::time::Instant::now())));
    }
}

/// Grant enforcement: a browser close prompt cannot prolong an expired authorization.
pub fn force_close_profile(profile: &Path) -> Result<(), String> {
    let ids = profile_process_ids(profile)?;
    #[cfg(windows)]
    {
        let state = (ids, true);
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::EnumWindows(
                Some(profile_window),
                &state as *const _ as isize,
            );
        }
    }
    #[cfg(not(windows))]
    for pid in ids {
        let _ = output("/bin/kill", &["-TERM", &pid.to_string()]);
    }
    if !profile_process_ids(profile)?.is_empty() {
        std::thread::sleep(std::time::Duration::from_millis(500));
    }
    for pid in profile_process_ids(profile)? {
        #[cfg(windows)]
        if let Err(error) = output("taskkill", &["/PID", &pid.to_string(), "/T", "/F"]) {
            if profile_process_ids(profile)?.contains(&pid) {
                return Err(error);
            }
        }
        #[cfg(not(windows))]
        if let Err(error) = output("/bin/kill", &["-KILL", &pid.to_string()]) {
            if profile_process_ids(profile)?.contains(&pid) {
                return Err(error);
            }
        }
    }
    // TERM/KILL and taskkill completion are not a guarantee that browser
    // helpers and profile locks have already finished releasing resources.
    wait_profile_closed(profile,std::time::Duration::from_secs(8))
}
pub(crate) fn prepare_firefox_profile(profile: &Path) -> Result<PathBuf, String> {
    let failure = |error: std::io::Error| {
        format!(
        "无法准备 Firefox 专用配置目录 {}：{error}。请检查此目录的访问权限和剩余空间，然后重试；无需删除日常 Firefox 配置。",
        profile.display()
    )
    };
    std::fs::create_dir_all(profile).map_err(failure)?;
    let path = std::fs::canonicalize(profile).map_err(failure)?;
    // Firefox needs to create locks and databases, not just read user.js.
    let probe = tempfile::NamedTempFile::new_in(&path).map_err(failure)?;
    probe.as_file().sync_all().map_err(failure)?;
    probe.close().map_err(failure)?;
    // Windows canonicalize adds a verbatim path prefix some Firefox versions
    // do not accept. The original absolute path uses normal Win32 syntax.
    #[cfg(target_os = "windows")]
    let path = if profile.is_absolute() {
        profile.to_path_buf()
    } else {
        std::env::current_dir().map_err(failure)?.join(profile)
    };
    Ok(path)
}

pub fn cli_installed() -> bool {
    let names = if cfg!(target_os = "windows") {
        vec!["claude.exe", "claude.cmd"]
    } else {
        vec!["claude"]
    };
    let mut roots: Vec<PathBuf> = std::env::var_os("PATH")
        .map(|p| std::env::split_paths(&p).collect())
        .unwrap_or_default();
    roots.extend([
        home_dir().join(".local/bin"),
        home_dir().join(".npm-global/bin"),
        PathBuf::from("/opt/homebrew/bin"),
        PathBuf::from("/usr/local/bin"),
    ]);
    roots
        .into_iter()
        .any(|p| names.iter().any(|n| crate::terminal::executable_file(&p.join(n))))
}

pub fn launcher_name() -> &'static str {
    if cfg!(target_os = "windows") {
        "claude-ready.ps1"
    } else if cfg!(target_os = "linux") {
        "claude-ready.sh"
    } else {
        "claude-ready.command"
    }
}

pub fn launcher_contents() -> &'static str {
    if cfg!(target_os = "windows") {
        "$ErrorActionPreference = 'Stop'\r\n$env:TZ = 'Asia/Singapore'\r\n$env:LANG = 'en_US.UTF-8'\r\n$env:LC_ALL = 'en_US.UTF-8'\r\n$env:PATH = (Join-Path $env:USERPROFILE '.local/bin') + ';' + $env:PATH\r\nSet-Location -LiteralPath $env:USERPROFILE\r\nif (-not (Get-Command claude -CommandType Application -ErrorAction SilentlyContinue)) { throw 'Claude Code is not installed or not on PATH.' }\r\n& claude @args\r\n"
    } else if cfg!(target_os = "linux") {
        "#!/bin/bash\nexport PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:$HOME/.bun/bin:/usr/local/bin:/usr/bin:/bin:$PATH\"\nexport TZ=Asia/Singapore\nexport LANG=en_US.UTF-8\nexport LC_ALL=en_US.UTF-8\ncd -- \"$HOME\" || exit 1\nif ! command -v claude >/dev/null 2>&1; then echo 'Claude Code is not installed or not on PATH.'; exit 1; fi\nexec claude \"$@\"\n"
    } else {
        "#!/bin/zsh\nexport PATH=\"$HOME/.local/bin:$HOME/.npm-global/bin:/opt/homebrew/bin:/usr/local/bin:$PATH\"\nexport TZ=Asia/Singapore\nexport LANG=en_US.UTF-8\nexport LC_ALL=en_US.UTF-8\ncd \"$HOME\" || exit 1\nif ! command -v claude >/dev/null 2>&1; then echo 'Claude Code is not installed or not on PATH.'; exit 1; fi\nexec claude \"$@\"\n"
    }
}

pub fn launch_cli(path: &Path) -> Result<(), String> {
    if !cli_installed() {
        return Err("未找到 Claude Code，请先安装后重启 NodeCloak。".into());
    }
    #[cfg(target_os = "windows")]
    {
        use std::os::windows::process::CommandExt;
        // A visible terminal is intentional: this command is called by the user's Open Claude Code button.
        Command::new("powershell.exe")
            .args(["-NoExit", "-NoProfile", "-File"])
            .arg(path)
            .current_dir(home_dir())
            .creation_flags(0x00000010)
            .spawn()
            .map_err(|e| e.to_string())?;
    }
    #[cfg(target_os = "macos")]
    {
        // AppleScript and shell quoting are separate; application data paths can contain spaces and apostrophes.
        let shell_path = format!("'{}'", path.to_string_lossy().replace('\'', "'\\''"));
        let script_path = shell_path.replace('\\', "\\\\").replace('"', "\\\"");
        output(
            "/usr/bin/osascript",
            &[
                "-e",
                &format!("tell application \"Terminal\" to do script \"{script_path}\""),
                "-e",
                "tell application \"Terminal\" to activate",
            ],
        )?;
    }
    #[cfg(target_os = "linux")]
    crate::terminal::linux::open(path, &home_dir(), None)?;
    let _ = path;
    Ok(())
}

pub fn font_evidence() -> Vec<String> {
    #[cfg(target_os = "linux")]
    return desktop_linux::font_evidence();
    #[cfg(not(target_os = "linux"))]
    {
    let root = if cfg!(target_os = "windows") {
        std::env::var_os("WINDIR")
            .map(PathBuf::from)
            .unwrap_or_else(|| PathBuf::from("C:/Windows"))
            .join("Fonts")
    } else {
        PathBuf::from("/System/Library/Fonts")
    };
    [
        "msyh.ttc",
        "msyhbd.ttc",
        "msyhl.ttc",
        "simfang.ttf",
        "simsun.ttc",
        "Deng.ttf",
        "STHeiti Light.ttc",
        "PingFang.ttc",
    ]
    .iter()
    .filter(|name| root.join(name).is_file())
    .map(|s| s.to_string())
    .collect()
    }
}
