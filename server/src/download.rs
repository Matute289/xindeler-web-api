use serde::Deserialize;
use std::time::Duration;

const DOWNLOADS_BASE_URL: &str = "https://downloads.xindeler.com";
const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
const REQUEST_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone, Deserialize)]
pub struct Platform {
    pub os: String,
    pub arch: String,
    pub file: String,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Manifest {
    pub version: String,
    pub platforms: Vec<Platform>,
}

pub struct DownloadManifestClient {
    client: reqwest::blocking::Client,
    manifest_url: String,
}

impl DownloadManifestClient {
    pub fn new(manifest_url: &str) -> Self {
        let client = reqwest::blocking::Client::builder()
            .connect_timeout(CONNECT_TIMEOUT)
            .timeout(REQUEST_TIMEOUT)
            .build()
            .expect("reqwest client config is always valid");
        Self {
            client,
            manifest_url: manifest_url.to_owned(),
        }
    }

    /// `None` covers every failure mode (network, non-2xx, malformed JSON)
    /// uniformly -- the caller (the `/api/download` handler) treats a
    /// missing manifest exactly like "no matching platform found", so there
    /// is nothing for a caller to branch on beyond presence/absence.
    pub fn fetch_manifest(&self) -> Option<Manifest> {
        let response = self.client.get(&self.manifest_url).send().ok()?;
        if !response.status().is_success() {
            log::warn!(
                "downloads manifest fetch returned {}: {}",
                response.status(),
                self.manifest_url
            );
            return None;
        }
        response.json().ok()
    }
}

/// Hand-rolled substring matching, not a User-Agent-parsing crate -- this
/// service's convention is a minimal dependency footprint for anything this
/// size (see `game_server_client.rs`'s own hand-rolled HTTP client).
pub(crate) fn detect_os(user_agent: &str) -> Option<&'static str> {
    if user_agent.contains("Windows") {
        Some("windows")
    } else if user_agent.contains("Mac OS X") || user_agent.contains("Macintosh") {
        Some("macos")
    } else if user_agent.contains("Linux") && !user_agent.contains("Android") {
        Some("linux")
    } else {
        None
    }
}

/// Always returns a value (never `None`) -- arch detection is a best-effort
/// default, not authoritative. `x86_64` is the fallback because it's the
/// broadly compatible choice; a wrong guess is recoverable via the manual
/// platform list on the frontend, unlike an undetected OS, which has no
/// sane default across three unrelated platforms.
pub(crate) fn detect_arch(user_agent: &str) -> &'static str {
    if user_agent.contains("ARM64") || user_agent.contains("aarch64") {
        "arm64"
    } else {
        "x86_64"
    }
}

pub(crate) fn resolve_platform<'a>(
    manifest: &'a Manifest,
    os: &str,
    arch: &str,
) -> Option<&'a Platform> {
    manifest.platforms.iter().find(|platform| {
        platform.os.eq_ignore_ascii_case(os) && platform.arch.eq_ignore_ascii_case(arch)
    })
}

pub(crate) fn download_url(manifest: &Manifest, platform: &Platform) -> String {
    format!(
        "{DOWNLOADS_BASE_URL}/releases/{}/{}",
        manifest.version, platform.file
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_manifest() -> Manifest {
        Manifest {
            version: "v0.25.0".to_owned(),
            platforms: vec![
                Platform {
                    os: "windows".into(),
                    arch: "x86_64".into(),
                    file: "xindeler-voxygen-windows-x86_64.zip".into(),
                },
                Platform {
                    os: "windows".into(),
                    arch: "arm64".into(),
                    file: "xindeler-voxygen-windows-arm64.zip".into(),
                },
                Platform {
                    os: "macos".into(),
                    arch: "arm64".into(),
                    file: "xindeler-voxygen-macos-arm64.dmg".into(),
                },
            ],
        }
    }

    #[test]
    fn detects_windows_from_a_typical_user_agent() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36";
        assert_eq!(detect_os(ua), Some("windows"));
    }

    #[test]
    fn detects_macos_from_a_typical_user_agent() {
        let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
        assert_eq!(detect_os(ua), Some("macos"));
    }

    #[test]
    fn detects_linux_but_not_android() {
        let linux_ua = "Mozilla/5.0 (X11; Linux x86_64) AppleWebKit/537.36";
        let android_ua = "Mozilla/5.0 (Linux; Android 14; Pixel 8) AppleWebKit/537.36";
        assert_eq!(detect_os(linux_ua), Some("linux"));
        assert_eq!(detect_os(android_ua), None);
    }

    #[test]
    fn detects_no_os_for_an_unrecognized_user_agent() {
        assert_eq!(detect_os("curl/8.4.0"), None);
    }

    #[test]
    fn detects_arm64_from_the_arm64_marker() {
        let ua = "Mozilla/5.0 (Windows NT 10.0; ARM64) AppleWebKit/537.36";
        assert_eq!(detect_arch(ua), "arm64");
    }

    #[test]
    fn defaults_to_x86_64_when_arch_is_ambiguous() {
        // The documented Apple Silicon / Rosetta case: no ARM64/aarch64
        // marker present even on some real ARM64 Macs.
        let ua = "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15";
        assert_eq!(detect_arch(ua), "x86_64");
    }

    #[test]
    fn resolves_an_existing_platform_case_insensitively() {
        let manifest = sample_manifest();
        let platform = resolve_platform(&manifest, "Windows", "X86_64").unwrap();
        assert_eq!(platform.file, "xindeler-voxygen-windows-x86_64.zip");
    }

    #[test]
    fn resolve_platform_returns_none_for_a_combination_not_in_the_manifest() {
        let manifest = sample_manifest();
        assert!(resolve_platform(&manifest, "linux", "x86_64").is_none());
    }

    #[test]
    fn builds_the_full_download_url() {
        let manifest = sample_manifest();
        let platform = resolve_platform(&manifest, "macos", "arm64").unwrap();
        assert_eq!(
            download_url(&manifest, platform),
            "https://downloads.xindeler.com/releases/v0.25.0/xindeler-voxygen-macos-arm64.dmg"
        );
    }
}
