fn normalize(value: &str) -> Option<String> {
    let value = value.split(['.', '@']).next()?.replace('_', "-");
    if matches!(value.as_str(), "C" | "POSIX")
        || value.len() < 2
        || value.len() > 35
        || !value.bytes().all(|b| b.is_ascii_alphabetic() || b == b'-')
    {
        return None;
    }
    Some(value)
}

fn from_values(values: &[Option<String>]) -> Vec<String> {
    let mut languages = vec![];
    for value in values.iter().flatten() {
        for item in value.split(':').filter_map(normalize) {
            if !languages.contains(&item) {
                languages.push(item);
            }
        }
    }
    languages
}

#[cfg(target_os = "linux")]
pub(crate) fn linux() -> Vec<String> {
    from_values(&["LANGUAGE", "LC_ALL", "LC_MESSAGES", "LANG"].map(|name| std::env::var(name).ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn linux_locale_preferences_strip_encoding_and_preserve_priority() {
        assert_eq!(
            from_values(&[
                Some("zh_CN:en_US.UTF-8".into()),
                None,
                Some("zh_CN.UTF-8".into()),
                Some("en_GB.UTF-8@euro".into())
            ]),
            vec!["zh-CN", "en-US", "en-GB"]
        );
        assert!(from_values(&[Some("C.UTF-8:POSIX:../../bad:$(touch bad)".into())]).is_empty());
    }
}
