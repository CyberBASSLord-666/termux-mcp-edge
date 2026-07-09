use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSecurityError {
    MissingHost,
    HostNotAllowed { received: String },
    OriginNotAllowed { received: String },
    OriginRequired,
    InvalidOrigin { received: String },
}

impl TransportSecurityError {
    pub fn client_message(&self) -> &'static str {
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
        match self {
            Self::MissingHost => write!(formatter, "missing Host header"),
            Self::HostNotAllowed { received } => {
                write!(formatter, "Host is not allowed: {received}")
            }
            Self::OriginNotAllowed { received } => {
                write!(formatter, "Origin is not allowed: {received}")
            }
            Self::OriginRequired => write!(formatter, "Origin header is required"),
            Self::InvalidOrigin { received } => {
                write!(formatter, "Origin is malformed or unsupported: {received}")
            }
        }
    }
}

impl std::error::Error for TransportSecurityError {}

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
    ) -> Self {
        Self {
            allowed_hosts: allowed_hosts
                .into_iter()
                .filter_map(|host| normalize_host(&host.into()))
                .collect(),
            allowed_origins: allowed_origins
                .into_iter()
                .filter_map(|origin| normalize_origin(&origin.into()))
                .collect(),
            allow_missing_origin,
        }
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
                        received: raw_origin.trim().to_string(),
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

fn normalize_host(host: &str) -> Option<String> {
    let trimmed = host.trim().trim_end_matches('.');
    if trimmed.is_empty() || trimmed.contains('/') || trimmed.contains(' ') {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

fn normalize_origin(origin: &str) -> Option<String> {
    let trimmed = origin.trim();
    let lower_trimmed = trimmed.to_ascii_lowercase();
    let (scheme, authority) = if let Some(authority) = lower_trimmed.strip_prefix("http://") {
        ("http://", authority)
    } else if let Some(authority) = lower_trimmed.strip_prefix("https://") {
        ("https://", authority)
    } else {
        return None;
    };

    if authority.is_empty()
        || authority.contains('*')
        || authority.contains(' ')
        || authority.contains('/')
        || authority.contains('?')
        || authority.contains('#')
        || authority.contains('@')
    {
        return None;
    }

    Some(format!("{scheme}{authority}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_expected_localhost_host_and_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        policy
            .validate_request(Some("LOCALHOST:8000"), Some("http://localhost:8000"))
            .unwrap();
    }

    #[test]
    fn allows_case_insensitive_origin_scheme_and_authority() {
        let policy =
            TransportSecurityPolicy::new(["localhost:8000"], ["HTTP://LOCALHOST:8000"], false);
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
    fn exposes_stable_client_messages_without_received_values() {
        assert_eq!(
            TransportSecurityError::HostNotAllowed {
                received: "attacker.example".to_string(),
            }
            .client_message(),
            "host_not_allowed"
        );
        assert_eq!(
            TransportSecurityError::OriginNotAllowed {
                received: "https://attacker.example".to_string(),
            }
            .client_message(),
            "origin_not_allowed"
        );
        assert_eq!(
            TransportSecurityError::InvalidOrigin {
                received: "javascript:alert(1)".to_string(),
            }
            .client_message(),
            "invalid_origin"
        );
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
    fn requires_http_origin_scheme() {
        let policy = TransportSecurityPolicy::new(
            ["localhost:8000"],
            ["chrome-extension://example", "file://localhost"],
            false,
        );

        assert!(policy.allowed_origins().is_empty());
        assert!(matches!(
            policy
                .validate_request(Some("localhost:8000"), Some("chrome-extension://example"))
                .unwrap_err(),
            TransportSecurityError::InvalidOrigin { .. }
        ));
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
}
