use std::collections::BTreeSet;
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransportSecurityError {
    MissingHost,
    HostNotAllowed { received: String },
    OriginNotAllowed { received: String },
    OriginRequired,
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

        match origin.and_then(normalize_origin) {
            Some(origin) if self.allowed_origins.contains(&origin) => Ok(()),
            Some(origin) => Err(TransportSecurityError::OriginNotAllowed { received: origin }),
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
    let trimmed = origin.trim().trim_end_matches('/');
    if trimmed.is_empty() || trimmed.contains(' ') {
        return None;
    }
    Some(trimmed.to_ascii_lowercase())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allows_expected_localhost_host_and_origin() {
        let policy = TransportSecurityPolicy::localhost(8000, false);
        policy
            .validate_request(Some("LOCALHOST:8000"), Some("http://localhost:8000/"))
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
