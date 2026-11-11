use super::*;

pub(super) fn prepare_command(cmd: &mut Command, current_exe: Option<&Path>, bundled: bool) {
    if !bundled || current_exe.is_none_or(|exe| Path::new(cmd.get_program()) == exe) {
        return;
    }
    // System utilities use system libraries. Self-hosted proxy helpers need
    // the mounted application's bundled runtime and retain its environment.
    for name in [
        "LD_LIBRARY_PATH",
        "LD_PRELOAD",
        "APPDIR",
        "APPIMAGE",
        "ARGV0",
        "OWD",
    ] {
        cmd.env_remove(name);
    }
}

#[cfg(target_os = "linux")]
pub(super) fn timezone() -> Result<String, String> {
    timezone_from(Path::new("/etc/localtime"), Path::new("/etc/timezone"))
}

fn timezone_from(localtime: &Path, timezone_file: &Path) -> Result<String, String> {
    let linked = std::fs::canonicalize(localtime).ok().and_then(|path| {
        path.to_string_lossy()
            .split_once("zoneinfo/")
            .map(|(_, zone)| zone.to_owned())
    });
    linked
        .or_else(|| {
            std::fs::read_to_string(timezone_file)
                .ok()
                .map(|text| text.trim().to_owned())
        })
        .filter(|zone| zone.parse::<chrono_tz::Tz>().is_ok())
        .ok_or_else(|| "无法读取系统时区，请在 Ubuntu 日期与时间设置中检查。".into())
}

#[cfg(target_os = "linux")]
pub(super) fn timezone_catalog() -> Vec<TimezoneOption> {
    use chrono::Offset;
    let now = chrono::Utc::now();
    chrono_tz::TZ_VARIANTS
        .into_iter()
        .filter(|zone| Path::new("/usr/share/zoneinfo").join(zone.name()).is_file())
        .map(|zone| TimezoneOption {
            id: zone.name().into(),
            label: zone.name().into(),
            offset_minutes: -now.with_timezone(&zone).offset().fix().local_minus_utc() / 60,
        })
        .collect()
}

#[cfg(target_os = "linux")]
pub(super) fn font_evidence() -> Vec<String> {
    // Fontconfig knows Ubuntu's nested font directories. Return only system
    // font files; user fonts remain under the separate removable-font catalog.
    let Ok(text) = output("/usr/bin/fc-list", &["--format", "%{file}\n"]) else {
        return vec![];
    };
    let mut files: Vec<_> = text
        .lines()
        .filter(|line| {
            let path = Path::new(line);
            (path.starts_with("/usr/share/fonts") || path.starts_with("/usr/local/share/fonts"))
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(crate::fonts::candidate)
        })
        .map(str::to_owned)
        .collect();
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    #[cfg(target_os = "linux")]
    #[test]
    fn timezone_verification_accepts_canonical_utc_links_without_accepting_seasonal_zones() {
        for alias in ["UTC", "GMT", "Etc/UTC", "Etc/GMT"] {
            assert!(super::super::timezone_names_match("Etc/UTC", alias));
            assert!(super::super::verify_timezone_values("utc", alias, 0).is_ok());
            assert!(super::super::verify_timezone_values("utc", alias, -480).is_err());
        }
        assert!(!super::super::timezone_names_match(
            "Europe/London",
            "Etc/UTC"
        ));
    }
    #[test]
    fn appimage_external_tools_do_not_inherit_bundle_but_self_helpers_do() {
        let exe = Path::new("/tmp/.mount_NodeCloak/usr/bin/nodecloak");
        for (program, bundled, sanitized) in [
            ("/usr/bin/gnome-terminal", true, true),
            ("/usr/bin/fc-cache", false, false),
            (exe.to_str().unwrap(), true, false),
        ] {
            let mut cmd = Command::new(program);
            prepare_command(&mut cmd, Some(exe), bundled);
            let removals: Vec<_> = cmd
                .get_envs()
                .filter(|(_, value)| value.is_none())
                .map(|(key, _)| key.to_str().unwrap())
                .collect();
            assert_eq!(removals.contains(&"LD_LIBRARY_PATH"), sanitized);
            assert_eq!(removals.contains(&"APPIMAGE"), sanitized);
        }
    }
    #[test]
    fn timezone_file_fallback_validates_names() {
        let root = tempfile::tempdir().unwrap();
        let timezone = root.path().join("timezone");
        let localtime = root.path().join("localtime");
        std::fs::write(&timezone, "Asia/Singapore\n").unwrap();
        assert_eq!(
            timezone_from(&localtime, &timezone).unwrap(),
            "Asia/Singapore"
        );
        std::fs::write(&timezone, "../../etc/shadow").unwrap();
        assert!(timezone_from(&localtime, &timezone).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn current_localtime_link_takes_priority_over_stale_timezone_file() {
        let root = tempfile::tempdir().unwrap();
        let target = root.path().join("zoneinfo/Europe/Paris");
        std::fs::create_dir_all(target.parent().unwrap()).unwrap();
        std::fs::write(&target, "fixture").unwrap();
        let localtime = root.path().join("localtime");
        std::os::unix::fs::symlink(&target, &localtime).unwrap();
        let timezone = root.path().join("timezone");
        std::fs::write(&timezone, "Asia/Singapore\n").unwrap();
        assert_eq!(
            timezone_from(&localtime, &timezone).unwrap(),
            "Europe/Paris"
        );
    }
}
