#!/usr/bin/env python3
from pathlib import Path


def replace_once(path: Path, old: str, new: str) -> None:
    text = path.read_text()
    count = text.count(old)
    if count != 1:
        raise SystemExit(f"{path}: expected exactly one match, found {count}: {old[:160]!r}")
    path.write_text(text.replace(old, new, 1))


auth = Path("src/auth.rs")
replace_once(
    auth,
    '''use crate::{
    config::{AuthConfig, AuthPosture},
    directory_grant::{
        has_directory_grant_header, take_directory_grant_authorization,
        DirectoryGrantAuthorization,
    },
};
''',
    '''use crate::config::{AuthConfig, AuthPosture};
#[cfg(feature = "mcp-runtime")]
use crate::directory_grant::{
    has_directory_grant_header, take_directory_grant_authorization,
    DirectoryGrantAuthorization,
};
''',
)
replace_once(
    auth,
    '''    pub(crate) fn stable_id(&self) -> &str {
        &self.stable_id
    }
''',
    '''    #[cfg(feature = "mcp-runtime")]
    pub(crate) fn stable_id(&self) -> &str {
        &self.stable_id
    }
''',
)
replace_once(
    auth,
    '''        McpAuthPolicy::UnauthenticatedLocalhostOnly => {
            if has_directory_grant_header(request.headers()) {
''',
    '''        McpAuthPolicy::UnauthenticatedLocalhostOnly => {
            #[cfg(feature = "mcp-runtime")]
            if has_directory_grant_header(request.headers()) {
''',
)
replace_once(
    auth,
    '''                let grant = match take_directory_grant_authorization(request.headers_mut()) {
''',
    '''                #[cfg(feature = "mcp-runtime")]
                let grant: Option<DirectoryGrantAuthorization> =
                    match take_directory_grant_authorization(request.headers_mut()) {
''',
)
replace_once(
    auth,
    '''                    Err(_) => {
                        return authorization_context_response(
                            StatusCode::BAD_REQUEST,
                            "invalid_authorization_context",
                            "Capability-grant authorization context is malformed.",
                        );
                    }
                };
''',
    '''                        Err(_) => {
                            return authorization_context_response(
                                StatusCode::BAD_REQUEST,
                                "invalid_authorization_context",
                                "Capability-grant authorization context is malformed.",
                            );
                        }
                    };
''',
)
replace_once(
    auth,
    '''                if let Some(grant): Option<DirectoryGrantAuthorization> = grant {
''',
    '''                #[cfg(feature = "mcp-runtime")]
                if let Some(grant) = grant {
''',
)
replace_once(
    auth,
    '''fn authorization_context_response(
''',
    '''#[cfg(feature = "mcp-runtime")]
fn authorization_context_response(
''',
)
replace_once(
    auth,
    '''    #[tokio::test]
    async fn grant_context_requires_authentication_and_is_removed_before_dispatch() {
''',
    '''    #[cfg(feature = "mcp-runtime")]
    #[tokio::test]
    async fn grant_context_requires_authentication_and_is_removed_before_dispatch() {
''',
)
replace_once(
    auth,
    '''    #[tokio::test]
    async fn bearer_authentication_precedes_grant_context_validation() {
''',
    '''    #[cfg(feature = "mcp-runtime")]
    #[tokio::test]
    async fn bearer_authentication_precedes_grant_context_validation() {
''',
)

grant = Path("src/directory_grant.rs")
replace_once(
    grant,
    '''pub struct VerifiedDirectoryGrant {
''',
    '''#[derive(PartialEq, Eq)]
pub struct VerifiedDirectoryGrant {
''',
)
replace_once(
    grant,
    '''    fn binding(principal: &AuthenticatedPrincipal) -> DirectoryGrantBinding<'_> {
        DirectoryGrantBinding {
            principal,
            session_id: SESSION_ID,
            safe_root_id: ROOT_ID,
            target_components: &["projects".to_owned(), "alpha".to_owned()],
        }
    }
''',
    '''    fn binding<'a>(
        principal: &'a AuthenticatedPrincipal,
        target_components: &'a [String],
    ) -> DirectoryGrantBinding<'a> {
        DirectoryGrantBinding {
            principal,
            session_id: SESSION_ID,
            safe_root_id: ROOT_ID,
            target_components,
        }
    }
''',
)
replace_once(
    grant,
    '''        for (mut claims, expected) in [
''',
    '''        for (claims, expected) in [
''',
)
principal_line = '''        let principal = AuthenticatedPrincipal::configured(PRINCIPAL).unwrap();
'''
text = grant.read_text()
count = text.count(principal_line)
if count != 3:
    raise SystemExit(f"src/directory_grant.rs: expected three verifier test principals, found {count}")
text = text.replace(
    principal_line,
    principal_line
    + '''        let components = vec!["projects".to_owned(), "alpha".to_owned()];
''',
)
text = text.replace("binding(&principal)", "binding(&principal, &components)")
grant.write_text(text)
