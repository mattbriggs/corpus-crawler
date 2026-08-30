//! Protocol-level failures (REQ-086, REQ-087).

use std::fmt;

use crawl_domain::ids::RequestId;

/// The maximum number of characters of an offending line reproduced in an
/// error message.
///
/// A worker can emit a very long line; quoting all of it would flood the log
/// while adding nothing diagnostic.
const EXCERPT_LIMIT: usize = 200;

/// A message that cannot be interpreted according to the supported protocol.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProtocolError {
    /// The line was not valid JSON (REQ-086).
    #[error("worker emitted malformed JSON on protocol stdout: {reason} (line: {excerpt})")]
    MalformedJson {
        /// The parser's description of the failure.
        reason: String,
        /// A truncated excerpt of the offending line.
        excerpt: String,
    },
    /// The line was valid JSON but not a JSON object.
    #[error("worker message must be a JSON object, found {found} (line: {excerpt})")]
    NotAnObject {
        /// The JSON type actually received.
        found: &'static str,
        /// A truncated excerpt of the offending line.
        excerpt: String,
    },
    /// A required protocol field was absent.
    #[error("worker message is missing required field {field:?}")]
    MissingField {
        /// The absent field.
        field: &'static str,
    },
    /// A protocol field had the wrong JSON type.
    #[error("worker message field {field:?} must be {expected}, found {found}")]
    InvalidFieldType {
        /// The offending field.
        field: &'static str,
        /// The type the protocol requires.
        expected: &'static str,
        /// The type actually received.
        found: &'static str,
    },
    /// The message carried a field the protocol does not define.
    ///
    /// Rejecting unknown top-level protocol fields keeps the contract
    /// unambiguous and makes a version mismatch visible immediately rather
    /// than as silently discarded data.
    #[error("worker message contains undefined protocol field {field:?}")]
    UndefinedField {
        /// The offending field.
        field: String,
    },
    /// The `status` value is not part of this protocol version.
    #[error("worker message declares unknown status {status:?}")]
    UnknownStatus {
        /// The unrecognised status value.
        status: String,
    },
    /// A well-formed message arrived where a different kind was expected.
    #[error("expected a {expected} message but received a {actual} message")]
    UnexpectedMessage {
        /// The message kind the host was waiting for.
        expected: &'static str,
        /// The message kind actually received.
        actual: &'static str,
    },
    /// The response identifier did not match the outstanding request
    /// (REQ-163, VAL-040).
    #[error("worker responded to request {actual} while request {expected} was outstanding")]
    CorrelationMismatch {
        /// The outstanding request identifier.
        expected: RequestId,
        /// The identifier the worker replied with.
        actual: RequestId,
    },
    /// An element of `rows` was not a JSON object (VAL-043).
    #[error("row {index} of the response is a {found}, but every row must be a JSON object")]
    RowNotAnObject {
        /// Zero-based index of the offending row.
        index: usize,
        /// The JSON type actually received.
        found: &'static str,
    },
    /// The worker declared an unsupported protocol version at handshake.
    #[error(
        "worker declares protocol api_version {declared:?}; this host implements {supported:?}"
    )]
    UnsupportedVersion {
        /// The version the worker declared.
        declared: String,
        /// The version this host implements.
        supported: &'static str,
    },
}

impl ProtocolError {
    /// Returns a short, stable kind label.
    ///
    /// Cross-language contract fixtures assert on this label, so the strings
    /// are part of the test contract and must not change casually.
    #[must_use]
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MalformedJson { .. } => "malformed_json",
            Self::NotAnObject { .. } => "not_an_object",
            Self::MissingField { .. } => "missing_field",
            Self::InvalidFieldType { .. } => "invalid_field_type",
            Self::UndefinedField { .. } => "undefined_field",
            Self::UnknownStatus { .. } => "unknown_status",
            Self::UnexpectedMessage { .. } => "unexpected_message",
            Self::CorrelationMismatch { .. } => "correlation_mismatch",
            Self::RowNotAnObject { .. } => "row_not_an_object",
            Self::UnsupportedVersion { .. } => "unsupported_version",
        }
    }
}

/// Truncates a line for inclusion in an error message.
#[must_use]
pub fn excerpt(line: &str) -> String {
    let cleaned: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    if cleaned.chars().count() <= EXCERPT_LIMIT {
        return cleaned;
    }
    let truncated: String = cleaned.chars().take(EXCERPT_LIMIT).collect();
    format!("{truncated}...")
}

/// Names the JSON type of a value, for diagnostics.
#[must_use]
pub fn json_type_name(value: &serde_json::Value) -> &'static str {
    match value {
        serde_json::Value::Null => "null",
        serde_json::Value::Bool(_) => "boolean",
        serde_json::Value::Number(number) => {
            if number.is_f64() {
                "number"
            } else {
                "integer"
            }
        }
        serde_json::Value::String(_) => "string",
        serde_json::Value::Array(_) => "array",
        serde_json::Value::Object(_) => "object",
    }
}

impl fmt::Debug for ExcerptOf<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&excerpt(self.0))
    }
}

/// Helper wrapper that renders a truncated excerpt in `Debug` position.
pub struct ExcerptOf<'a>(pub &'a str);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn excerpts_are_truncated_and_control_free() {
        let long = "x".repeat(500);
        let rendered = excerpt(&long);
        assert!(rendered.ends_with("..."));
        assert_eq!(rendered.chars().count(), EXCERPT_LIMIT + 3);
        assert_eq!(excerpt("a\nb"), "a b");
    }

    /// One instance of every `ProtocolError` variant.
    fn every_variant() -> Vec<ProtocolError> {
        vec![
            ProtocolError::MalformedJson {
                reason: "expected value".into(),
                excerpt: "junk".into(),
            },
            ProtocolError::NotAnObject {
                found: "array",
                excerpt: "[]".into(),
            },
            ProtocolError::MissingField { field: "id" },
            ProtocolError::InvalidFieldType {
                field: "id",
                expected: "an integer",
                found: "string",
            },
            ProtocolError::UndefinedField {
                field: "extra".into(),
            },
            ProtocolError::UnknownStatus {
                status: "maybe".into(),
            },
            ProtocolError::UnexpectedMessage {
                expected: "ready",
                actual: "ok",
            },
            ProtocolError::CorrelationMismatch {
                expected: RequestId::new(1),
                actual: RequestId::new(2),
            },
            ProtocolError::RowNotAnObject {
                index: 3,
                found: "string",
            },
            ProtocolError::UnsupportedVersion {
                declared: "2".into(),
                supported: "1",
            },
        ]
    }

    #[test]
    fn every_variant_is_represented() {
        assert_eq!(
            every_variant().len(),
            10,
            "every_variant() must construct one of each ProtocolError variant"
        );
    }

    #[test]
    fn kind_labels_are_unique_across_every_variant() {
        // Cross-language contract fixtures assert on these labels, so a
        // collision would let a Python-side divergence pass unnoticed.
        let mut kinds: Vec<&str> = every_variant().iter().map(ProtocolError::kind).collect();
        let total = kinds.len();
        kinds.sort_unstable();
        kinds.dedup();
        assert_eq!(kinds.len(), total, "two protocol errors share a kind label");
    }

    #[test]
    fn every_variant_renders_a_message_naming_its_subject() {
        // An operator reading a log needs the offending field, status, or id in
        // the text; a bare "protocol error" would be undiagnosable.
        let expectations = [
            ("malformed_json", "junk"),
            ("not_an_object", "array"),
            ("missing_field", "id"),
            ("invalid_field_type", "id"),
            ("undefined_field", "extra"),
            ("unknown_status", "maybe"),
            ("unexpected_message", "ready"),
            ("correlation_mismatch", "2"),
            ("row_not_an_object", "3"),
            ("unsupported_version", "2"),
        ];
        for error in every_variant() {
            let rendered = error.to_string();
            let (_, needle) = expectations
                .iter()
                .find(|(kind, _)| *kind == error.kind())
                .unwrap_or_else(|| panic!("no expectation for {}", error.kind()));
            assert!(
                rendered.contains(needle),
                "{} rendered {rendered:?}, which omits {needle:?}",
                error.kind()
            );
        }
    }
}
