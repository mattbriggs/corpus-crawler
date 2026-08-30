//! Host-to-worker requests (REQ-081, VAL-030, VAL-031).

use crawl_domain::ids::RequestId;
use serde::{Deserialize, Serialize};

/// The operation a request asks the worker to perform.
///
/// `process` is the default so that the minimal request documented in the SRS,
/// `{"id": 42, "filepath": "/data/a.txt"}`, remains valid on the wire. The
/// control operations are additive and carry no `filepath`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum RequestOp {
    /// Process one file and return records.
    #[default]
    Process,
    /// Confirm the worker started, loaded the plugin, and speaks this protocol
    /// version. Sent once per worker before any file is dispatched.
    Handshake,
    /// Run the plugin's optional registration-time self-test (REQ-019).
    Selftest,
    /// Ask the worker to exit cleanly. No response is required.
    Shutdown,
}

impl RequestOp {
    /// Returns the wire spelling of the operation.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Process => "process",
            Self::Handshake => "handshake",
            Self::Selftest => "selftest",
            Self::Shutdown => "shutdown",
        }
    }

    /// Returns `true` when the operation requires a `filepath`.
    #[must_use]
    pub const fn requires_filepath(self) -> bool {
        matches!(self, Self::Process)
    }

    /// Returns `true` when the host must wait for a response.
    #[must_use]
    pub const fn expects_response(self) -> bool {
        !matches!(self, Self::Shutdown)
    }
}

/// One host-to-worker request.
///
/// Field order is part of the canonical encoding used by cross-language
/// contract fixtures: `id`, then `op`, then `filepath`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Request {
    /// Correlation identifier (REQ-063).
    pub id: RequestId,
    /// Requested operation. Omitted on the wire when it is `process`, which
    /// keeps the documented minimal request shape valid.
    #[serde(default, skip_serializing_if = "is_process")]
    pub op: RequestOp,
    /// Target file path, present exactly for `process` requests (REQ-064).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub filepath: Option<String>,
}

fn is_process(op: &RequestOp) -> bool {
    matches!(op, RequestOp::Process)
}

impl Request {
    /// Creates a file-processing request.
    ///
    /// # Caller contract
    ///
    /// `filepath` must be the path the host discovered, encoded as UTF-8. The
    /// host is responsible for having decided that the file matches the
    /// plugin's input rules before calling this (PRE-001).
    #[must_use]
    pub fn process(id: RequestId, filepath: impl Into<String>) -> Self {
        Self {
            id,
            op: RequestOp::Process,
            filepath: Some(filepath.into()),
        }
    }

    /// Creates a control request that carries no file path.
    #[must_use]
    pub fn control(id: RequestId, op: RequestOp) -> Self {
        Self {
            id,
            op,
            filepath: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn process_requests_encode_without_an_op_field() {
        let request = Request::process(RequestId::new(42), "/data/a.txt");
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(encoded, r#"{"id":42,"filepath":"/data/a.txt"}"#);
    }

    #[test]
    fn control_requests_encode_their_op_and_omit_filepath() {
        let request = Request::control(RequestId::new(0), RequestOp::Handshake);
        let encoded = serde_json::to_string(&request).unwrap();
        assert_eq!(encoded, r#"{"id":0,"op":"handshake"}"#);
    }

    #[test]
    fn requests_round_trip_and_default_to_process() {
        let decoded: Request = serde_json::from_str(r#"{"id":7,"filepath":"/a"}"#).unwrap();
        assert_eq!(decoded.op, RequestOp::Process);
        assert_eq!(decoded, Request::process(RequestId::new(7), "/a"));
    }

    #[test]
    fn unknown_request_fields_are_rejected() {
        // A worker must not silently accept a field from a newer host: that is
        // how a version mismatch turns into wrong output instead of an error.
        let error = serde_json::from_str::<Request>(r#"{"id":1,"filepath":"/a","extra":true}"#)
            .expect_err("an undefined field must be rejected");
        assert!(
            error.to_string().contains("extra"),
            "the rejection must name the offending field: {error}"
        );
    }

    #[test]
    fn operation_properties_are_explicit() {
        assert!(RequestOp::Process.requires_filepath());
        assert!(!RequestOp::Handshake.requires_filepath());
        assert!(RequestOp::Handshake.expects_response());
        assert!(!RequestOp::Shutdown.expects_response());
        assert_eq!(RequestOp::Selftest.as_str(), "selftest");
    }
}
