use std::collections::BTreeSet;
use std::fmt;
use std::net::{IpAddr, Ipv6Addr};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSecurityError {
    MissingHost,
    HostNotAllowed { received: String },
    OriginNotAllowed { received: String },
    OriginRequired,
    InvalidOrigin { received: String },
}

impl TransportSecurityError {
    pub fn reason_code(&self) -> &'static str {
        match self {
            Self::MissingHost => "missing_host",
            Self::HostNotAllowed { .. } => "host_not_allowed",
            Self::OriginNotAllowed { .. } => "origin_not_allowed",
            Self::OriginRequired => "origin_required",
            Self::InvalidOrigin { .. } => "invalid_origin",
        }
    }
}

impl fmt::Display for TransportSecurityError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for TransportSecurityError {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportSecurityPolicyError {
    InvalidHost,
    InvalidOrigin,
}

impl TransportSecurityPolicyError {
    pub fn reason_code(self) -> &'static str {
        match self {
            Self::InvalidHost => "invalid_allowed_host",
            Self::InvalidOrigin => "invalid_allowed_origin",
        }
    }
}

impl fmt::Display for TransportSecurityPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.reason_code())
    }
}

impl std::error::Error for TransportSecurityPolicyError {}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransportSecurityPolicy {
    allowed_hosts: BTreeSet<String>,
    allowed_origins: BTreeSet<String>,
    allow_missing_origin: bool,
}

impl TransportSecurityPolicy {
    pub fn new(
        allowed_hosts: impl IntoIterator<Item = impl Into<String>>,
        allowed_origins: impl IntoIterator<Item = impl Into<String>>,
        allow_missing_origin: bool,
    ) -> Result<Self, TransportSecurityPolicyError> {
        let allowed_hosts = allowed_hosts
            .into_iter()
            .map(|host| {
                let host = host.into();
                normalize_host(&host).ok_or(TransportSecurityPolicyError::InvalidHost)
            })
            .collect::<Result<_, _>>()?;
        let allowed_origins = allowed_origins
            .into_iter()
            .map(|origin| {
                let origin = origin.into();
                normalize_origin(&origin).ok_or(TransportSecurityPolicyError::InvalidOrigin)
            })
            .collect::<Result<_, _>>()?;

        Ok(Self {
            allowed_hosts,
            allowed_origins,
            allow_missing_origin,
        })
    }

    pub fn localhost(port: u16, allow_missing_origin: bool) -> Self {
        let port = port.to_string();
        Self::new(
            [
                format!("localhost:{port}"),
                format!("127.0.0.1:{port}"),
                format!("[::1]:{port}"),
            ],
            [
                format!("http://localhost:{port}"),
                format!("http://127.0.0.1:{port}"),
                format!("http://[::1]:{port}"),
            ],
            allow_missing_origin,
        )
        .expect("built-in localhost transport authorities must be valid")
    }

    pub fn validate_request(
        &self,
        host: Option<&str>,
        origin: Option<&str>,
    ) -> Result<(), TransportSecurityError> {
        let host = host
            .and_then(normalize_host)
            .ok_or(TransportSecurityError::MissingHost)?;

        if !self.allowed_hosts.contains(&host) {
            return Err(TransportSecurityError::HostNotAllowed { received: host });
        }

        match origin {
            Some(raw_origin) => {
                let normalized_origin = normalize_origin(raw_origin).ok_or_else(|| {
                    TransportSecurityError::InvalidOrigin {
                        received: raw_origin.to_string(),
                    }
                })?;

                if self.allowed_origins.contains(&normalized_origin) {
                    Ok(())
                } else {
                    Err(TransportSecurityError::OriginNotAllowed {
                        received: normalized_origin,
                    })
                }
            }
            None if self.allow_missing_origin => Ok(()),
            None => Err(TransportSecurityError::OriginRequired),
        }
    }

    pub fn allowed_hosts(&self) -> &BTreeSet<String> {
        &self.allowed_hosts
    }

    pub fn allowed_origins(&self) -> &BTreeSet<String> {
        &self.allowed_origins
    }
}

pub(crate) fn normalize_host(host: &str) -> Option<String> {
    normalize_authority(host)
}

pub(crate) fn normalize_origin(origin: &str) -> Option<String> {
    if contains_ascii_whitespace_or_control(origin) {
        return None;
    }

    let lower = origin.to_ascii_lowercase();
    let (scheme, authority) = if let Some(authority) = lower.strip_prefix("http://") {
        ("http://", authority)
    } else if let Some(authority) = lower.strip_prefix("https://") {
        ("https://", authority)
    } else {
        return None;
    };

    normalize_authority(authority).map(|authority| format!("{scheme}{authority}"))
}

fn normalize_authority(authority: &str) -> Option<String> {
    if authority.is_empty()
        || contains_ascii_whitespace_or_control(authority)
        || authority
            .bytes()
            .any(|byte| matches!(byte, b'*' | b'/' | b'?' | b'#' | b'@' | b'\\'))
    {
        return None;
    }

    if let Some(bracketed) = authority.strip_prefix('[') {
        let close = bracketed.find(']')?;
        let address = &bracketed[..close];
        let remainder = &bracketed[close + 1..];
        let ipv6 = address.parse::<Ipv6Addr>().ok()?;
        let port = parse_port_suffix(remainder)?;
        return Some(format!("[{ipv6}]{port}"));
    }

    if authority.contains(['[', ']']) || authority.matches(':').count() > 1 {
        return None;
    }

    let (raw_host, port) = match authority.split_once(':') {
        Some((host, port)) => (host, format!(":{}", parse_port(port)?)),
        None => (authority, String::new()),
    };

    let raw_host = raw_host.strip_suffix('.').unwrap_or(raw_host);
    if raw_host.is_empty() || raw_host.ends_with('.') {
        return None;
    }

    let normalized_host = match raw_host.parse::<IpAddr>() {
        Ok(IpAddr::V4(ipv4)) => ipv4.to_string(),
        Ok(IpAddr::V6(_)) => return None,
        Err(_) if valid_dns_name(raw_host) => raw_host.to_ascii_lowercase(),
        Err(_) => return None,
    };

    Some(format!("{normalized_host}{port}"))
}

fn parse_port_suffix(remainder: &str) -> Option<String> {
    if remainder.is_empty() {
        return Some(String::new());
    }
    let port = remainder.strip_prefix(':')?;
    Some(format!(":{}", parse_port(port)?))
}

fn parse_port(port: &str) -> Option<u16> {
    if port.is_empty() || !port.bytes().all(|byte| byte.is_ascii_digit()) {
        return None;
    }
    let port = port.parse::<u16>().ok()?;
    (port != 0).then_some(port)
}

fn valid_dns_name(host: &str) -> bool {
    if host.len() > 253 {
        return false;
    }

    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && label
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
            && !label.starts_with('-')
            && !label.ends_with('-')
    })
}

fn contains_ascii_whitespace_or_control(value: &str) -> bool {
    value
        .bytes()
        .any(|byte| byte.is_ascii_whitespace() || byte.is_ascii_control())
}

#[cfg(test)]
mod tests {
    use super::*;

    const ACCEPTED_AUTHORITIES: &[(&str, &str)] = &[
        ("localhost", "localhost"),
        ("LOCALHOST:8000", "localhost:8000"),
        ("example.com.", "example.com"),
        ("127.0.0.1:8000", "127.0.0.1:8000"),
        ("[::1]", "[::1]"),
        ("[0:0:0:0:0:0:0:1]:8000", "[::1]:8000"),
    ];

    const REJECTED_AUTHORITIES: &[&str] = &[
        "",
        " ",
        "localhost\t:8000",
        "localhost\n:8000",
        "local\u{7f}host:8000",
        "*",
        "*.example.com",
        "user@example.com",
        "example.com/path",
        "example.com?query",
        "example.com#fragment",
        "example.com:",
        "example.com:0",
        "example.com:65536",
        "example.com:not-a-port",
        ":8000",
        "-example.com",
        "example-.com",
        "example..com",
        "::1",
        "2001:db8::1",
        "[::1",
        "::1]",
        "[]:8000",
        "[127.0.0.1]:8000",
        "[::1]junk",
        "[::1]:",
        "[::1]:65536",
    ];

    #[test]
    fn allows_expected_localhost_host_and_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        policy
            .validate_request(Some("LOCALHOST:8000"), Some("http://localhost:8000"))
            .unwrap();
    }

    #[test]
    fn uses_identical_authority_normalization_for_hosts_and_origins() {
        for (input, expected) in ACCEPTED_AUTHORITIES {
            assert_eq!(normalize_host(input).as_deref(), Some(*expected));
            assert_eq!(
                normalize_origin(&format!("HTTP://{input}")),
                Some(format!("http://{expected}"))
            );
        }

        for input in REJECTED_AUTHORITIES {
            assert_eq!(normalize_host(input), None, "host accepted: {input:?}");
            assert_eq!(
                normalize_origin(&format!("http://{input}")),
                None,
                "origin accepted: {input:?}"
            );
        }
    }

    #[test]
    fn constructor_fails_closed_for_invalid_allowlist_entries() {
        assert_eq!(
            TransportSecurityPolicy::new(
                ["localhost:8000", "bad host"],
                ["http://localhost:8000"],
                false,
            ),
            Err(TransportSecurityPolicyError::InvalidHost)
        );
        assert_eq!(
            TransportSecurityPolicy::new(
                ["localhost:8000"],
                ["http://localhost:8000", "file://localhost"],
                false,
            ),
            Err(TransportSecurityPolicyError::InvalidOrigin)
        );
    }

    #[test]
    fn allows_case_insensitive_origin_scheme_and_authority() {
        let policy =
            TransportSecurityPolicy::new(["localhost:8000"], ["HTTP://LOCALHOST:8000"], false)
                .unwrap();
        assert!(policy.allowed_origins().contains("http://localhost:8000"));

        policy
            .validate_request(Some("localhost:8000"), Some("HTTP://LOCALHOST:8000"))
            .unwrap();
    }

    #[test]
    fn rejects_unlisted_host() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        let error = policy
            .validate_request(Some("example.com:8000"), Some("http://localhost:8000"))
            .unwrap_err();
        assert!(matches!(
            error,
            TransportSecurityError::HostNotAllowed { .. }
        ));
    }

    #[test]
    fn rejects_unlisted_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        let error = policy
            .validate_request(Some("localhost:8000"), Some("https://example.com"))
            .unwrap_err();
        assert!(matches!(
            error,
            TransportSecurityError::OriginNotAllowed { .. }
        ));
    }

    #[test]
    fn rejects_origin_url_components_beyond_exact_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, false);

        for origin in [
            "http://localhost:8000/",
            "http://localhost:8000/path",
            "http://localhost:8000?debug=true",
            "http://localhost:8000#fragment",
            "https://identity@localhost:8000",
            "https://*.localhost:8000",
        ] {
            let error = policy
                .validate_request(Some("localhost:8000"), Some(origin))
                .unwrap_err();
            assert!(matches!(
                error,
                TransportSecurityError::InvalidOrigin { .. }
            ));
        }
    }

    #[test]
    fn rejects_ascii_whitespace_and_controls_without_trimming() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        for host in [" localhost:8000", "localhost:8000 ", "localhost\t:8000"] {
            assert_eq!(
                policy.validate_request(Some(host), Some("http://localhost:8000")),
                Err(TransportSecurityError::MissingHost)
            );
        }
        for origin in [
            " http://localhost:8000",
            "http://localhost:8000 ",
            "http://localhost\n:8000",
        ] {
            assert!(matches!(
                policy
                    .validate_request(Some("localhost:8000"), Some(origin))
                    .unwrap_err(),
                TransportSecurityError::InvalidOrigin { .. }
            ));
        }
    }

    #[test]
    fn requires_http_origin_scheme() {
        assert_eq!(
            TransportSecurityPolicy::new(
                ["localhost:8000"],
                ["chrome-extension://example", "file://localhost"],
                false,
            ),
            Err(TransportSecurityPolicyError::InvalidOrigin)
        );
    }

    #[test]
    fn requires_origin_when_configured() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        assert_eq!(
            policy
                .validate_request(Some("localhost:8000"), None)
                .unwrap_err(),
            TransportSecurityError::OriginRequired
        );
    }

    #[test]
    fn can_allow_missing_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, true);
        policy
            .validate_request(Some("127.0.0.1:8000"), None)
            .unwrap();
    }

    #[test]
    fn rejects_missing_host() {
        let policy = TransportSecurityPolicy::localhost(8000, true);
        assert_eq!(
            policy.validate_request(None, None).unwrap_err(),
            TransportSecurityError::MissingHost
        );
    }

    #[test]
    fn transport_security_errors_render_stable_reason_codes() {
        let cases = [
            (TransportSecurityError::MissingHost, "missing_host"),
            (
                TransportSecurityError::HostNotAllowed {
                    received: "attacker.example:8000".to_string(),
                },
                "host_not_allowed",
            ),
            (
                TransportSecurityError::OriginNotAllowed {
                    received: "https://attacker.example".to_string(),
                },
                "origin_not_allowed",
            ),
            (TransportSecurityError::OriginRequired, "origin_required"),
            (
                TransportSecurityError::InvalidOrigin {
                    received: "https://identity@localhost:8000".to_string(),
                },
                "invalid_origin",
            ),
        ];

        for (error, reason_code) in cases {
            assert_eq!(error.reason_code(), reason_code);
            assert_eq!(error.to_string(), reason_code);
        }
    }

    #[test]
    fn transport_security_policy_errors_render_stable_reason_codes() {
        let cases = [
            (
                TransportSecurityPolicyError::InvalidHost,
                "invalid_allowed_host",
            ),
            (
                TransportSecurityPolicyError::InvalidOrigin,
                "invalid_allowed_origin",
            ),
        ];

        for (error, reason_code) in cases {
            assert_eq!(error.reason_code(), reason_code);
            assert_eq!(error.to_string(), reason_code);
        }
    }

    #[test]
    fn transport_security_display_does_not_reflect_received_header_values() {
        let attacker_values = [
            (
                TransportSecurityError::HostNotAllowed {
                    received: "attacker.example:8000".to_string(),
                },
                "attacker.example",
            ),
            (
                TransportSecurityError::OriginNotAllowed {
                    received: "https://attacker.example".to_string(),
                },
                "attacker.example",
            ),
            (
                TransportSecurityError::InvalidOrigin {
                    received: "https://identity@localhost:8000".to_string(),
                },
                "identity@localhost",
            ),
        ];

        for (error, attacker_value) in attacker_values {
            assert!(
                !error.to_string().contains(attacker_value),
                "transport rejection Display reflected attacker-controlled input"
            );
        }
    }
}
