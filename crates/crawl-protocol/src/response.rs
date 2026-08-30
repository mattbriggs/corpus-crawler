//! Worker-to-host responses (REQ-082 - REQ-085, VAL-040 - VAL-051).

use crawl_domain::ids::RequestId;
use crawl_domain::record::RawRecord;
use serde::{Deserialize, Serialize};

/// A plugin-declared file-processing failure (OQ-011).
///
/// # Decision
///
/// The mandatory fields are a machine-readable `code` and a human-readable
/// `message`; `detail` is an optional free-form object for plugin-specific
/// diagnostics. Correlation lives on the enclosing response rather than being
/// duplicated here (VAL-050). Error information never reaches the CSV
/// (VAL-051): it becomes a log event and a statistic.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PluginError {
    /// Stable, plugin-defined error code, for example `unreadable_file`.
    pub code: String,
    /// Human-readable description of the failure.
    pub message: String,
    /// Optional plugin-specific diagnostic payload.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub detail: Option<serde_json::Value>,
}

impl PluginError {
    /// Creates a plugin error with no detail payload.
    #[must_use]
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
            detail: None,
        }
    }
}

/// One decoded worker message.
#[derive(Debug, Clone, PartialEq)]
pub enum Response {
    /// Handshake acknowledgement, sent once per worker before file dispatch.
    Ready {
        /// Correlation identifier of the handshake request.
        id: RequestId,
        /// Protocol version the worker implements.
        api_version: String,
        /// Optional plugin identity, echoed for diagnostics.
        plugin: Option<String>,
        /// Optional SDK identity, echoed for diagnostics.
        sdk: Option<String>,
    },
    /// Successful processing, carrying zero or more candidate records
    /// (REQ-083, REQ-084).
    Ok {
        /// Correlation identifier.
        id: RequestId,
        /// Candidate records, not yet schema-validated.
        rows: Vec<RawRecord>,
    },
    /// Plugin-declared processing failure (REQ-085).
    Error {
        /// Correlation identifier.
        id: RequestId,
        /// The declared failure.
        error: PluginError,
    },
}

impl Response {
    /// Returns the correlation identifier carried by the message.
    #[must_use]
    pub const fn id(&self) -> RequestId {
        match self {
            Self::Ready { id, .. } | Self::Ok { id, .. } | Self::Error { id, .. } => *id,
        }
    }

    /// Returns the wire `status` value of the message.
    #[must_use]
    pub const fn status(&self) -> &'static str {
        match self {
            Self::Ready { .. } => "ready",
            Self::Ok { .. } => "ok",
            Self::Error { .. } => "error",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_and_id_are_readable_for_every_variant() {
        let ready = Response::Ready {
            id: RequestId::new(0),
            api_version: "1".into(),
            plugin: None,
            sdk: None,
        };
        assert_eq!(ready.status(), "ready");
        assert_eq!(ready.id(), RequestId::new(0));

        let ok = Response::Ok {
            id: RequestId::new(1),
            rows: vec![],
        };
        assert_eq!(ok.status(), "ok");

        let error = Response::Error {
            id: RequestId::new(2),
            error: PluginError::new("boom", "it broke"),
        };
        assert_eq!(error.status(), "error");
        assert_eq!(error.id(), RequestId::new(2));
    }

    #[test]
    fn plugin_error_detail_is_optional_on_the_wire() {
        let encoded = serde_json::to_string(&PluginError::new("bad_file", "cannot read")).unwrap();
        assert_eq!(encoded, r#"{"code":"bad_file","message":"cannot read"}"#);
    }
}
