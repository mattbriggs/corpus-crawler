//! Identity value objects.
//!
//! Every identifier validates its own representation at construction so that
//! an invalid identity cannot be represented once it has entered the domain
//! (SRS 6.3).

use std::fmt;

use serde::{Deserialize, Serialize};
use uuid::Uuid;

/// Error produced when an identity value object rejects its input.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum IdentityError {
    /// The supplied value was empty or contained only whitespace.
    #[error("{kind} must not be empty")]
    Empty {
        /// Human-readable name of the rejected value object.
        kind: &'static str,
    },
    /// The supplied value contained characters outside the permitted set.
    #[error("{kind} {value:?} is invalid: {reason}")]
    Invalid {
        /// Human-readable name of the rejected value object.
        kind: &'static str,
        /// The rejected input.
        value: String,
        /// Why the input was rejected.
        reason: &'static str,
    },
}

/// Registry-visible plugin name.
///
/// # Caller contract
///
/// A name is a lowercase ASCII token starting with a letter or digit and
/// containing only letters, digits, `-`, `_` and `.` (VAL-003). The restricted
/// character set keeps names usable as filesystem and registry keys on every
/// supported platform.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginName(String);

impl PluginName {
    /// Maximum permitted length in bytes.
    pub const MAX_LEN: usize = 64;

    /// Validates and constructs a plugin name.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when the value is empty, too long, or contains
    /// characters outside the permitted set.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdentityError::Empty {
                kind: "plugin name",
            });
        }
        if value.len() > Self::MAX_LEN {
            return Err(IdentityError::Invalid {
                kind: "plugin name",
                value,
                reason: "must be at most 64 bytes",
            });
        }
        let mut chars = value.chars();
        let first = chars.next().expect("non-empty");
        if !first.is_ascii_alphanumeric() {
            return Err(IdentityError::Invalid {
                kind: "plugin name",
                value,
                reason: "must start with an ASCII letter or digit",
            });
        }
        if !value.chars().all(|c| {
            c.is_ascii_alphanumeric() && !c.is_ascii_uppercase() || matches!(c, '-' | '_' | '.')
        }) {
            return Err(IdentityError::Invalid {
                kind: "plugin name",
                value,
                reason: "may contain only lowercase letters, digits, '-', '_' and '.'",
            });
        }
        Ok(Self(value))
    }

    /// Returns the name as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PluginName {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PluginName> for String {
    fn from(value: PluginName) -> Self {
        value.0
    }
}

/// Opaque, non-empty plugin version string.
///
/// Semantic versioning is *not* mandated: OQ-014 is unresolved, so the domain
/// only guarantees that the version is a non-empty token without whitespace
/// (VAL-004).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct PluginVersion(String);

impl PluginVersion {
    /// Validates and constructs a plugin version.
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when the value is empty or contains
    /// whitespace or control characters.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdentityError::Empty {
                kind: "plugin version",
            });
        }
        if value.chars().any(|c| c.is_whitespace() || c.is_control()) {
            return Err(IdentityError::Invalid {
                kind: "plugin version",
                value,
                reason: "must not contain whitespace or control characters",
            });
        }
        Ok(Self(value))
    }

    /// Returns the version as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for PluginVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for PluginVersion {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<PluginVersion> for String {
    fn from(value: PluginVersion) -> Self {
        value.0
    }
}

/// Declared host-plugin API version (REQ-150, VAL-002).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ApiVersion(String);

impl ApiVersion {
    /// Validates and constructs an API version token.
    ///
    /// Support is checked separately by [`ApiVersion::is_supported`] so that an
    /// unsupported-but-well-formed version can be reported precisely (REQ-013).
    ///
    /// # Errors
    ///
    /// Returns [`IdentityError`] when the token is empty or malformed.
    pub fn new(value: impl Into<String>) -> Result<Self, IdentityError> {
        let value = value.into();
        if value.trim().is_empty() {
            return Err(IdentityError::Empty {
                kind: "api_version",
            });
        }
        if !value.chars().all(|c| c.is_ascii_digit() || c == '.') {
            return Err(IdentityError::Invalid {
                kind: "api_version",
                value,
                reason: "must contain only digits and '.'",
            });
        }
        Ok(Self(value))
    }

    /// Returns `true` when this host implements the declared API version.
    #[must_use]
    pub fn is_supported(&self) -> bool {
        self.0 == crate::SUPPORTED_API_VERSION
    }

    /// Returns the version as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ApiVersion {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.0)
    }
}

impl TryFrom<String> for ApiVersion {
    type Error = IdentityError;

    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<ApiVersion> for String {
    fn from(value: ApiVersion) -> Self {
        value.0
    }
}

/// Registry key for an installed plugin.
///
/// The registry is keyed by name: installing a plugin whose name is already
/// registered updates the existing entry (REQ-011).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct PluginId(PluginName);

impl PluginId {
    /// Wraps a validated plugin name as a registry identity.
    #[must_use]
    pub fn new(name: PluginName) -> Self {
        Self(name)
    }

    /// Returns the underlying plugin name.
    #[must_use]
    pub fn name(&self) -> &PluginName {
        &self.0
    }

    /// Returns the identity as a string slice.
    #[must_use]
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}

impl fmt::Display for PluginId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<PluginName> for PluginId {
    fn from(value: PluginName) -> Self {
        Self(value)
    }
}

/// Globally unique identity of one `crawl run` execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CrawlId(Uuid);

impl CrawlId {
    /// Generates a fresh crawl identity.
    #[must_use]
    pub fn generate() -> Self {
        Self(Uuid::new_v4())
    }

    /// Wraps an existing UUID, primarily for deterministic tests.
    #[must_use]
    pub fn from_uuid(uuid: Uuid) -> Self {
        Self(uuid)
    }
}

impl fmt::Display for CrawlId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

/// Identity of one worker slot in the pool.
///
/// The identity belongs to the *slot*, not to the OS process: when a worker is
/// replaced after a crash the replacement reuses the slot index and increments
/// its generation, which makes restart counting unambiguous in logs.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct WorkerId {
    /// Zero-based worker slot index, bounded by the effective worker count.
    pub slot: u32,
    /// Number of times this slot has been (re)started, starting at 1.
    pub generation: u32,
}

impl WorkerId {
    /// Creates the first generation of a worker slot.
    #[must_use]
    pub fn new(slot: u32) -> Self {
        Self {
            slot,
            generation: 1,
        }
    }

    /// Returns the identity of the replacement worker for this slot.
    #[must_use]
    pub fn next_generation(self) -> Self {
        Self {
            slot: self.slot,
            generation: self.generation + 1,
        }
    }
}

impl fmt::Display for WorkerId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "w{}#{}", self.slot, self.generation)
    }
}

/// Correlation identifier for one host-to-worker request (REQ-063, VAL-030).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RequestId(u64);

impl RequestId {
    /// Wraps a raw protocol identifier.
    #[must_use]
    pub const fn new(value: u64) -> Self {
        Self(value)
    }

    /// Returns the raw protocol identifier.
    #[must_use]
    pub const fn get(self) -> u64 {
        self.0
    }
}

impl fmt::Display for RequestId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Returns why a value object rejected its input.
    ///
    /// Asserting on the reason rather than on `is_err` is what stops a test
    /// from passing when a value is rejected for the wrong cause - a rename
    /// that accidentally makes every name fail the length check would
    /// otherwise still look correct.
    fn rejection<T: std::fmt::Debug>(result: Result<T, IdentityError>) -> String {
        match result {
            Err(IdentityError::Empty { kind }) => format!("{kind} empty"),
            Err(IdentityError::Invalid { reason, .. }) => reason.to_owned(),
            Ok(value) => panic!("expected a rejection, got {value:?}"),
        }
    }

    #[test]
    fn plugin_names_accept_the_documented_character_set() {
        for accepted in ["entity-lines", "a1_b.c", "x", "0", "a".repeat(64).as_str()] {
            let name = PluginName::new(accepted)
                .unwrap_or_else(|error| panic!("{accepted:?} should be valid: {error}"));
            assert_eq!(name.as_str(), accepted, "the name was altered in transit");
        }
    }

    #[test]
    fn plugin_names_are_rejected_for_the_documented_reason() {
        assert_eq!(rejection(PluginName::new("  ")), "plugin name empty");
        assert_eq!(
            rejection(PluginName::new("x".repeat(65))),
            "must be at most 64 bytes"
        );
        assert_eq!(
            rejection(PluginName::new("-lead")),
            "must start with an ASCII letter or digit"
        );
        // Everything below is rejected by the character-set rule, not by the
        // length or leading-character rules that precede it.
        for rejected in ["Entity", "has space", "a/b", "a\\b", "a:b"] {
            assert_eq!(
                rejection(PluginName::new(rejected)),
                "may contain only lowercase letters, digits, '-', '_' and '.'",
                "{rejected:?} was rejected for the wrong reason"
            );
        }
    }

    #[test]
    fn plugin_versions_are_opaque_tokens_preserved_verbatim() {
        // The domain deliberately does not require semantic versioning
        // (OQ-014), so anything non-empty and whitespace-free round-trips
        // unchanged.
        for accepted in ["1.0.0", "2024-01-rc1", "v7", "1", "\u{7ffa}\u{672c}"] {
            let version = PluginVersion::new(accepted)
                .unwrap_or_else(|error| panic!("{accepted:?} should be valid: {error}"));
            assert_eq!(version.as_str(), accepted);
            assert_eq!(version.to_string(), accepted);
        }
    }

    #[test]
    fn plugin_versions_are_rejected_for_the_documented_reason() {
        assert_eq!(rejection(PluginVersion::new("")), "plugin version empty");
        assert_eq!(rejection(PluginVersion::new("   ")), "plugin version empty");
        for rejected in ["1 0", "1\t0", "1\n0"] {
            assert_eq!(
                rejection(PluginVersion::new(rejected)),
                "must not contain whitespace or control characters",
                "{rejected:?} was rejected for the wrong reason"
            );
        }
    }

    #[test]
    fn api_version_distinguishes_malformed_from_merely_unsupported() {
        // The distinction matters: a malformed token is an author's typo,
        // while a well-formed unsupported one is a compatibility problem that
        // REQ-013 requires the host to report precisely.
        let supported = ApiVersion::new("1").expect("well formed");
        assert!(supported.is_supported());
        assert_eq!(supported.as_str(), "1");

        let unsupported = ApiVersion::new("2").expect("well formed but unimplemented");
        assert!(!unsupported.is_supported());

        assert_eq!(
            rejection(ApiVersion::new("v1")),
            "must contain only digits and '.'"
        );
        assert_eq!(rejection(ApiVersion::new("")), "api_version empty");
    }

    #[test]
    fn worker_slots_keep_their_index_across_generations() {
        // The slot identifies the pool position and the generation counts
        // restarts, which is what makes "w3#2" readable as "slot 3, second
        // process" in a crash log.
        let first = WorkerId::new(3);
        assert_eq!((first.slot, first.generation), (3, 1));
        assert_eq!(first.to_string(), "w3#1");

        let second = first.next_generation();
        assert_eq!((second.slot, second.generation), (3, 2));
        assert_eq!(second.to_string(), "w3#2");

        let third = second.next_generation();
        assert_eq!((third.slot, third.generation), (3, 3));
        assert_ne!(first, second, "generations must be distinguishable");
    }
}
