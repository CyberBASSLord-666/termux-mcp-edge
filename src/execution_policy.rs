//! Inert execution-profile policy primitives.
//!
//! This module defines validation types for future allowlisted execution
//! profiles. It does not spawn processes, expose MCP tools, or enable runtime
//! execution.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecutionProfile {
    pub profile_name: String,
    pub program: String,
    pub argv: Vec<ArgumentTemplate>,
    pub placeholders: BTreeMap<String, PlaceholderPolicy>,
    pub working_directory: WorkingDirectoryPolicy,
    pub environment: EnvironmentPolicy,
    pub limits: ExecutionLimits,
    pub audit_gate_name: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ArgumentTemplate {
    Literal(String),
    Placeholder(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PlaceholderPolicy {
    pub name: String,
    pub validator: PlaceholderValidator,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PlaceholderValidator {
    Utf8String { max_bytes: usize },
    SafeRelativePath { max_bytes: usize },
    Enum { allowed_values: BTreeSet<String> },
    UnsignedInteger { min: u64, max: u64 },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkingDirectoryPolicy {
    Fixed(String),
    SafeRootRequired,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct EnvironmentPolicy {
    pub allowed: BTreeMap<String, String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExecutionLimits {
    pub timeout_ms: u64,
    pub stdout_max_bytes: usize,
    pub stderr_max_bytes: usize,
    pub max_argv_count: usize,
    pub max_argument_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExecutionPolicyError {
    EmptyProfileName,
    EmptyProgram,
    EmptyAuditGateName,
    EmptyLiteralArgument,
    ShellLikeArgument { argument: String },
    MissingPlaceholder { name: String },
    EmptyPlaceholderName,
    InvalidPlaceholderLimit { name: String },
    EmptyEnumValidator { name: String },
    InvalidIntegerRange { name: String },
    InvalidWorkingDirectory,
    DisallowedEnvironmentKey { key: String },
    InvalidLimit { field: &'static str },
    TooManyArguments { count: usize, max: usize },
    ArgumentTooLong { bytes: usize, max: usize },
}

impl ExecutionProfile {
    pub fn validate(&self) -> Result<(), ExecutionPolicyError> {
        if self.profile_name.trim().is_empty() {
            return Err(ExecutionPolicyError::EmptyProfileName);
        }
        if self.program.trim().is_empty() {
            return Err(ExecutionPolicyError::EmptyProgram);
        }
        if self.audit_gate_name.trim().is_empty() {
            return Err(ExecutionPolicyError::EmptyAuditGateName);
        }

        self.limits.validate()?;

        if self.argv.len() > self.limits.max_argv_count {
            return Err(ExecutionPolicyError::TooManyArguments {
                count: self.argv.len(),
                max: self.limits.max_argv_count,
            });
        }

        self.working_directory.validate()?;
        self.environment.validate()?;

        for placeholder in self.placeholders.values() {
            placeholder.validate()?;
        }

        for argument in &self.argv {
            match argument {
                ArgumentTemplate::Literal(value) => {
                    if value.is_empty() {
                        return Err(ExecutionPolicyError::EmptyLiteralArgument);
                    }
                    validate_argument_bytes(value, self.limits.max_argument_bytes)?;
                    if is_shell_like(value) {
                        return Err(ExecutionPolicyError::ShellLikeArgument {
                            argument: value.clone(),
                        });
                    }
                }
                ArgumentTemplate::Placeholder(name) => {
                    if name.trim().is_empty() {
                        return Err(ExecutionPolicyError::EmptyPlaceholderName);
                    }
                    if !self.placeholders.contains_key(name) {
                        return Err(ExecutionPolicyError::MissingPlaceholder { name: name.clone() });
                    }
                }
            }
        }

        Ok(())
    }
}

impl PlaceholderPolicy {
    pub fn validate(&self) -> Result<(), ExecutionPolicyError> {
        if self.name.trim().is_empty() {
            return Err(ExecutionPolicyError::EmptyPlaceholderName);
        }

        match &self.validator {
            PlaceholderValidator::Utf8String { max_bytes }
            | PlaceholderValidator::SafeRelativePath { max_bytes } => {
                if *max_bytes == 0 {
                    return Err(ExecutionPolicyError::InvalidPlaceholderLimit {
                        name: self.name.clone(),
                    });
                }
            }
            PlaceholderValidator::Enum { allowed_values } => {
                if allowed_values.is_empty() {
                    return Err(ExecutionPolicyError::EmptyEnumValidator {
                        name: self.name.clone(),
                    });
                }
            }
            PlaceholderValidator::UnsignedInteger { min, max } => {
                if min > max {
                    return Err(ExecutionPolicyError::InvalidIntegerRange {
                        name: self.name.clone(),
                    });
                }
            }
        }

        Ok(())
    }
}

impl WorkingDirectoryPolicy {
    fn validate(&self) -> Result<(), ExecutionPolicyError> {
        match self {
            Self::Fixed(path) if path.trim().is_empty() => {
                Err(ExecutionPolicyError::InvalidWorkingDirectory)
            }
            Self::Fixed(path) if !path.starts_with('/') => {
                Err(ExecutionPolicyError::InvalidWorkingDirectory)
            }
            _ => Ok(()),
        }
    }
}

impl EnvironmentPolicy {
    fn validate(&self) -> Result<(), ExecutionPolicyError> {
        for key in self.allowed.keys() {
            if !is_safe_environment_key(key) {
                return Err(ExecutionPolicyError::DisallowedEnvironmentKey { key: key.clone() });
            }
        }

        Ok(())
    }
}

impl ExecutionLimits {
    fn validate(self) -> Result<(), ExecutionPolicyError> {
        if self.timeout_ms == 0 {
            return Err(ExecutionPolicyError::InvalidLimit {
                field: "timeout_ms",
            });
        }
        if self.stdout_max_bytes == 0 {
            return Err(ExecutionPolicyError::InvalidLimit {
                field: "stdout_max_bytes",
            });
        }
        if self.stderr_max_bytes == 0 {
            return Err(ExecutionPolicyError::InvalidLimit {
                field: "stderr_max_bytes",
            });
        }
        if self.max_argv_count == 0 {
            return Err(ExecutionPolicyError::InvalidLimit {
                field: "max_argv_count",
            });
        }
        if self.max_argument_bytes == 0 {
            return Err(ExecutionPolicyError::InvalidLimit {
                field: "max_argument_bytes",
            });
        }

        Ok(())
    }
}

impl fmt::Display for ExecutionPolicyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{self:?}")
    }
}

impl std::error::Error for ExecutionPolicyError {}

fn validate_argument_bytes(value: &str, max: usize) -> Result<(), ExecutionPolicyError> {
    if value.len() > max {
        Err(ExecutionPolicyError::ArgumentTooLong {
            bytes: value.len(),
            max,
        })
    } else {
        Ok(())
    }
}

fn is_shell_like(value: &str) -> bool {
    value.contains(';')
        || value.contains('|')
        || value.contains('&')
        || value.contains('`')
        || value.contains("$(")
        || value.contains('\n')
        || value.contains('\0')
}

fn is_safe_environment_key(key: &str) -> bool {
    !key.is_empty()
        && key.len() <= 64
        && key
            .chars()
            .all(|character| character.is_ascii_uppercase() || character.is_ascii_digit() || character == '_')
        && !key.contains("TOKEN")
        && !key.contains("SECRET")
        && !key.contains("PASSWORD")
        && !key.contains("KEY")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_profile() -> ExecutionProfile {
        let mut placeholders = BTreeMap::new();
        placeholders.insert(
            "format".to_string(),
            PlaceholderPolicy {
                name: "format".to_string(),
                validator: PlaceholderValidator::Enum {
                    allowed_values: BTreeSet::from(["json".to_string()]),
                },
            },
        );

        ExecutionProfile {
            profile_name: "metadata_probe".to_string(),
            program: "cargo".to_string(),
            argv: vec![
                ArgumentTemplate::Literal("metadata".to_string()),
                ArgumentTemplate::Literal("--format-version".to_string()),
                ArgumentTemplate::Placeholder("format".to_string()),
            ],
            placeholders,
            working_directory: WorkingDirectoryPolicy::SafeRootRequired,
            environment: EnvironmentPolicy::default(),
            limits: ExecutionLimits {
                timeout_ms: 15_000,
                stdout_max_bytes: 1_048_576,
                stderr_max_bytes: 65_536,
                max_argv_count: 16,
                max_argument_bytes: 256,
            },
            audit_gate_name: "execution_profile".to_string(),
        }
    }

    #[test]
    fn valid_profile_passes_policy_validation() {
        assert_eq!(valid_profile().validate(), Ok(()));
    }

    #[test]
    fn empty_profile_name_is_rejected() {
        let mut profile = valid_profile();
        profile.profile_name = " ".to_string();

        assert_eq!(profile.validate(), Err(ExecutionPolicyError::EmptyProfileName));
    }

    #[test]
    fn empty_program_is_rejected() {
        let mut profile = valid_profile();
        profile.program.clear();

        assert_eq!(profile.validate(), Err(ExecutionPolicyError::EmptyProgram));
    }

    #[test]
    fn shell_like_literal_argument_is_rejected() {
        let mut profile = valid_profile();
        profile.argv.push(ArgumentTemplate::Literal("metadata; rm".to_string()));

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::ShellLikeArgument {
                argument: "metadata; rm".to_string(),
            })
        );
    }

    #[test]
    fn missing_placeholder_definition_is_rejected() {
        let mut profile = valid_profile();
        profile.argv.push(ArgumentTemplate::Placeholder("missing".to_string()));

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::MissingPlaceholder {
                name: "missing".to_string(),
            })
        );
    }

    #[test]
    fn disallowed_environment_key_is_rejected() {
        let mut profile = valid_profile();
        profile
            .environment
            .allowed
            .insert("API_TOKEN".to_string(), "redacted".to_string());

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::DisallowedEnvironmentKey {
                key: "API_TOKEN".to_string(),
            })
        );
    }

    #[test]
    fn invalid_limits_are_rejected() {
        let mut profile = valid_profile();
        profile.limits.timeout_ms = 0;

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::InvalidLimit {
                field: "timeout_ms",
            })
        );
    }

    #[test]
    fn too_many_arguments_are_rejected() {
        let mut profile = valid_profile();
        profile.limits.max_argv_count = 1;

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::TooManyArguments { count: 3, max: 1 })
        );
    }

    #[test]
    fn fixed_working_directory_must_be_absolute() {
        let mut profile = valid_profile();
        profile.working_directory = WorkingDirectoryPolicy::Fixed("relative".to_string());

        assert_eq!(
            profile.validate(),
            Err(ExecutionPolicyError::InvalidWorkingDirectory)
        );
    }
}
