use anyhow::Result;
use serde::{Deserialize, Serialize};
use serde_yaml_ng::Mapping;
use std::net::Ipv4Addr;

pub(crate) const MAX_SUBSCRIPTION_BYTES: usize = 10 * 1024 * 1024;
pub(crate) const MAX_NODES: usize = 5000;
pub(crate) const CAPABILITY_VERSION: u32 = 1;

#[derive(Debug)]
pub(crate) struct ChainFailure {
    pub code: &'static str,
    pub message: String,
}

impl std::fmt::Display for ChainFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ChainFailure {}

pub(crate) fn failure(code: &'static str, message: impl Into<String>) -> anyhow::Error {
    ChainFailure {
        code,
        message: message.into(),
    }
    .into()
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct FrontNode {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub server: String,
    pub port: u16,
    pub options: Mapping,
    pub rejection: Option<String>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct Subscription {
    pub name: String,
    pub imported_at: String,
    pub source: String,
    pub update_url: Option<String>,
    #[serde(skip)]
    pub nodes: Vec<FrontNode>,
    pub duplicates: usize,
}

#[derive(Clone, Default, Serialize, Deserialize)]
pub(crate) struct ExitAuthentication {
    pub username: String,
    pub password: String,
}

impl ExitAuthentication {
    pub fn validate(&self) -> Result<()> {
        let lengths = (self.username.len(), self.password.len());
        if lengths != (0, 0) && (!(1..=255).contains(&lengths.0) || !(1..=255).contains(&lengths.1)) {
            return Err(failure(
                "EXIT_AUTH_INVALID",
                "Use no authentication, or a username and password of 1-255 UTF-8 bytes each.",
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(crate) struct ExitEndpoint {
    pub host: String,
    pub port: u16,
}

impl ExitEndpoint {
    pub fn normalized(&self) -> Result<Self> {
        if self.port == 0 {
            return Err(failure(
                "EXIT_ADDRESS_INVALID",
                "The exit port must be between 1 and 65535.",
            ));
        }
        Ok(Self {
            host: normalize_host(&self.host)?,
            port: self.port,
        })
    }

    pub fn matches(&self, host: &str, port: u16) -> bool {
        self.port == port && normalize_host(&self.host).ok() == normalize_host(host).ok()
    }
}

#[derive(Clone, Copy, Default, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum RulePreset {
    ChinaDirect,
    #[default]
    AllProxy,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ExitInput {
    pub host: String,
    pub port: u16,
    pub username: Option<String>,
    pub password: Option<String>,
    pub remember: bool,
}

#[derive(Clone, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct FrontView {
    pub id: String,
    pub name: String,
    pub protocol: String,
    pub endpoint: String,
    pub rejection: Option<String>,
    pub excluded: bool,
}

#[derive(Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ChainPhase {
    #[default]
    Unconfigured,
    Ready,
    Probing,
    Connecting,
    Connected,
    Degraded,
    Disconnecting,
    Disconnected,
    RecoveryRequired,
}

#[derive(Clone, Default, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ProbeObservation {
    pub ipv4: String,
    pub checked_at: String,
    pub service: String,
    pub elapsed_ms: u32,
}

#[derive(Clone, Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChainErrorView {
    pub code: String,
    pub message: String,
}

#[derive(Serialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ChainView {
    pub phase: ChainPhase,
    pub fronts: Vec<FrontView>,
    pub subscription_name: Option<String>,
    pub subscription_updated_at: Option<String>,
    pub subscription_host: Option<String>,
    pub can_update_subscription: bool,
    pub duplicates: u32,
    pub host: String,
    pub port: u16,
    pub remember_authentication: bool,
    pub authentication_required: bool,
    pub has_session_authentication: bool,
    pub has_saved_authentication: bool,
    pub selected_front_id: Option<String>,
    pub preset: RulePreset,
    pub probe_consent: bool,
    pub background_checks: bool,
    pub observation: Option<ProbeObservation>,
    pub error: Option<ChainErrorView>,
    pub capability_version: u32,
}

#[derive(Clone, Deserialize, specta::Type)]
#[serde(rename_all = "camelCase")]
pub(crate) struct ConnectInput {
    pub front_id: String,
    pub preset: RulePreset,
    pub probe_consent: bool,
    pub background_checks: bool,
}

pub(crate) fn normalize_host(input: &str) -> Result<String> {
    let host = input.trim().trim_end_matches('.');
    let invalid = || {
        failure(
            "EXIT_ADDRESS_INVALID",
            "Enter an IPv4 address or DNS hostname without a scheme, path or port.",
        )
    };
    if host.is_empty()
        || host.len() > 253
        || host
            .chars()
            .any(|c| c.is_control() || c.is_whitespace() || ":/@?#[]\\".contains(c))
    {
        return Err(invalid());
    }
    if let Ok(ip) = host.parse::<Ipv4Addr>() {
        return Ok(ip.to_string());
    }
    if host.chars().all(|c| c.is_ascii_digit() || c == '.') {
        return Err(invalid());
    }
    let url = reqwest::Url::parse(&format!("https://{host}/")).map_err(|_| invalid())?;
    let normalized = url.host_str().ok_or_else(invalid)?.to_ascii_lowercase();
    if normalized.split('.').any(|label| {
        label.is_empty()
            || label.len() > 63
            || label.starts_with('-')
            || label.ends_with('-')
            || !label.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'-')
    }) {
        return Err(invalid());
    }
    Ok(normalized)
}

#[cfg(test)]
mod tests {
    use super::{ExitAuthentication, normalize_host};

    #[test]
    fn normalizes_idna_and_rejects_ambiguous_exit_addresses() -> anyhow::Result<()> {
        assert_eq!(normalize_host(" EXAMPLE.com. ")?, "example.com");
        assert_eq!(normalize_host("bücher.example")?, "xn--bcher-kva.example");
        for host in [
            "",
            "https://example.com",
            "example.com:1080",
            "::1",
            "999.1.2.3",
            "a/b",
            "a_b.test",
        ] {
            assert!(normalize_host(host).is_err(), "{host}");
        }
        Ok(())
    }

    #[test]
    fn bounds_socks_authentication_by_utf8_bytes_without_reencoding_special_characters() {
        let mut auth = ExitAuthentication {
            username: "名".repeat(85),
            password: "p:@&\"".into(),
        };
        assert!(auth.validate().is_ok());
        auth.username.push('x');
        assert!(auth.validate().is_err());
        auth.username.clear();
        assert!(auth.validate().is_err());
        auth.password.clear();
        assert!(auth.validate().is_ok());
    }
}
