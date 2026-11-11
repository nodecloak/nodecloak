use super::*;

pub(super) fn timezone() -> Result<String, String> {
    timezone_from(Path::new("/etc/localtime"), Path::new("/etc/timezone"))
}

fn timezone_from(localtime: &Path, timezone_file: &Path) -> Result<String, String> {
    let linked = std::fs::canonicalize(localtime).ok().and_then(|path| {
        path.to_string_lossy().split_once("zoneinfo/").map(|(_, zone)| zone.to_owned())
    });
    linked.or_else(|| std::fs::read_to_string(timezone_file).ok().map(|text| text.trim().to_owned()))
        .filter(|zone| zone.parse::<chrono_tz::Tz>().is_ok())
        .ok_or_else(|| "无法读取系统时区，请在 Ubuntu 日期与时间设置中检查。".into())
}

pub(super) fn timezone_catalog() -> Vec<TimezoneOption> {
    use chrono::Offset;
    let now = chrono::Utc::now();
    chrono_tz::TZ_VARIANTS.into_iter().filter(|zone| Path::new("/usr/share/zoneinfo").join(zone.name()).is_file())
        .map(|zone| TimezoneOption { id: zone.name().into(), label: zone.name().into(), offset_minutes: -now.with_timezone(&zone).offset().fix().local_minus_utc() / 60 }).collect()
}

pub(super) fn font_evidence() -> Vec<String> {
    // Fontconfig knows Ubuntu's nested font directories. Return only system
    // font files; user fonts remain under the separate removable-font catalog.
    let Ok(text) = output("/usr/bin/fc-list", &["--format", "%{file}\n"]) else { return vec![]; };
    let mut files: Vec<_> = text.lines().filter(|line| {
        let path = Path::new(line);
        (path.starts_with("/usr/share/fonts") || path.starts_with("/usr/local/share/fonts"))
            && path.file_name().and_then(|name| name.to_str()).is_some_and(crate::fonts::candidate)
    }).map(str::to_owned).collect();
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn timezone_file_fallback_validates_names() {
        let root = tempfile::tempdir().unwrap();
        let timezone = root.path().join("timezone");
        let localtime = root.path().join("localtime");
        std::fs::write(&timezone, "Asia/Singapore\n").unwrap();
        assert_eq!(timezone_from(&localtime, &timezone).unwrap(), "Asia/Singapore");
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
        assert_eq!(timezone_from(&localtime, &timezone).unwrap(), "Europe/Paris");
    }
}
