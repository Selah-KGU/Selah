//! Availability of on-device Apple Intelligence.
//!
//! The provider id stays `"local"`. On macOS 26+ it means the system model
//! exposed by Foundation Models. Windows and every other OS stay cloud-only.

use serde::Serialize;

pub const APPLE_INTELLIGENCE_MODEL_ID: &str = "apple-intelligence";
pub const MACOS_TOO_OLD_MESSAGE: &str = "Apple Intelligence を使うには macOS 26 以降が必要です。";
const WINDOWS_UNSUPPORTED_MESSAGE: &str =
    "ローカル AI は Windows では利用できません。クラウド API を使用してください。";
const OTHER_OS_MESSAGE: &str = "ローカル AI はこの OS では利用できません。";

#[derive(Debug, Clone, Serialize)]
pub struct LocalAiSupport {
    pub supported: bool,
    pub reason: String,
    pub permanent: bool,
    pub os: String,
    pub model: String,
    pub memory_gb: Option<f64>,
    pub chip: Option<String>,
    pub gpus: Vec<String>,
}

pub fn current() -> LocalAiSupport {
    probe_platform()
}

pub fn is_supported() -> bool {
    current().supported
}

pub fn should_demote_local_provider() -> bool {
    let support = current();
    !support.supported && support.permanent
}

pub fn ensure_supported() -> Result<(), String> {
    let support = current();
    if support.supported {
        Ok(())
    } else if support.reason.is_empty() {
        Err(unsupported_message())
    } else {
        Err(support.reason)
    }
}

pub fn unsupported_message() -> String {
    unsupported_message_for(std::env::consts::OS)
}

fn unsupported_message_for(os: &str) -> String {
    match os {
        "macos" => MACOS_TOO_OLD_MESSAGE.to_string(),
        "windows" => WINDOWS_UNSUPPORTED_MESSAGE.to_string(),
        _ => OTHER_OS_MESSAGE.to_string(),
    }
}

fn probe_platform() -> LocalAiSupport {
    #[cfg(target_os = "macos")]
    {
        return probe_macos();
    }
    #[cfg(not(target_os = "macos"))]
    {
        unsupported_platform(std::env::consts::OS)
    }
}

#[cfg(target_os = "macos")]
fn probe_macos() -> LocalAiSupport {
    match macos_major_version() {
        Some(major) if major >= 26 => match crate::local_ai::query_availability() {
            Ok(status) => LocalAiSupport {
                supported: status.supported,
                reason: status.reason,
                permanent: status.permanent,
                os: "macos".into(),
                model: status.model,
                memory_gb: None,
                chip: None,
                gpus: Vec::new(),
            },
            Err(reason) => LocalAiSupport {
                supported: false,
                reason,
                permanent: false,
                os: "macos".into(),
                model: String::new(),
                memory_gb: None,
                chip: None,
                gpus: Vec::new(),
            },
        },
        Some(_) => LocalAiSupport {
            supported: false,
            reason: MACOS_TOO_OLD_MESSAGE.into(),
            permanent: true,
            os: "macos".into(),
            model: String::new(),
            memory_gb: None,
            chip: None,
            gpus: Vec::new(),
        },
        None => LocalAiSupport {
            supported: false,
            reason: "macOS のバージョンを確認できませんでした。".into(),
            permanent: false,
            os: "macos".into(),
            model: String::new(),
            memory_gb: None,
            chip: None,
            gpus: Vec::new(),
        },
    }
}

#[cfg_attr(target_os = "macos", allow(dead_code))]
fn unsupported_platform(os: &str) -> LocalAiSupport {
    LocalAiSupport {
        supported: false,
        reason: unsupported_message_for(os),
        permanent: true,
        os: os.to_string(),
        model: String::new(),
        memory_gb: None,
        chip: None,
        gpus: Vec::new(),
    }
}

pub fn macos_major_version() -> Option<u32> {
    parse_product_version(&read_product_version_plist()?).map(|(major, _)| major)
}

fn read_product_version_plist() -> Option<String> {
    std::fs::read_to_string("/System/Library/CoreServices/SystemVersion.plist").ok()
}

pub(crate) fn parse_product_version(plist: &str) -> Option<(u32, u32)> {
    let key = "<key>ProductVersion</key>";
    let index = plist.find(key)? + key.len();
    let rest = &plist[index..];
    let start = rest.find("<string>")? + "<string>".len();
    let end = rest[start..].find("</string>")?;
    let mut parts = rest[start..start + end].trim().split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next().and_then(|part| part.parse().ok()).unwrap_or(0);
    Some((major, minor))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn product_version_plist_is_parsed() {
        let plist = "<key>ProductVersion</key>\\n\\t<string>27.2</string>";
        assert_eq!(parse_product_version(plist), Some((27, 2)));
        assert_eq!(
            parse_product_version("<key>ProductVersion</key><string>15</string>"),
            Some((15, 0))
        );
        assert_eq!(parse_product_version("no version"), None);
    }

    #[test]
    fn windows_and_other_platforms_cannot_use_local_models() {
        let windows = unsupported_platform("windows");
        assert!(!windows.supported);
        assert!(windows.permanent);
        assert!(windows.gpus.is_empty());
        assert!(windows.memory_gb.is_none());
        assert_eq!(windows.reason, WINDOWS_UNSUPPORTED_MESSAGE);

        let linux = unsupported_platform("linux");
        assert!(!linux.supported);
        assert!(linux.permanent);
        assert_eq!(linux.reason, OTHER_OS_MESSAGE);
    }
}
