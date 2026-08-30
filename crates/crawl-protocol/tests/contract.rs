//! Cross-language protocol contract tests.
//!
//! These read the same `tests/fixtures/protocol/cases.json` the Python SDK
//! tests read. Rust must encode requests to the exact bytes Python validates,
//! and decode exactly the responses Python produces.

use std::path::PathBuf;

use crawl_domain::ids::RequestId;
use crawl_protocol::{decode_response, encode_request, Request, RequestOp, Response};
use serde_json::Value;

fn cases() -> Value {
    let path =
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/protocol/cases.json");
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|error| panic!("cannot read {}: {error}", path.display()));
    serde_json::from_str(&text).expect("fixtures are valid JSON")
}

#[test]
fn rust_encodes_requests_to_the_canonical_bytes() {
    for case in cases()["requests"].as_array().expect("requests") {
        let name = case["name"].as_str().expect("name");
        let decoded = &case["decoded"];
        let id = RequestId::new(decoded["id"].as_u64().expect("id"));
        let request = match decoded["op"].as_str().expect("op") {
            "process" => Request::process(id, decoded["filepath"].as_str().expect("filepath")),
            "handshake" => Request::control(id, RequestOp::Handshake),
            "selftest" => Request::control(id, RequestOp::Selftest),
            "shutdown" => Request::control(id, RequestOp::Shutdown),
            other => panic!("{name}: unknown op {other}"),
        };
        let encoded = encode_request(&request).expect("encode");
        assert_eq!(
            encoded.trim_end(),
            case["encoded"].as_str().expect("encoded"),
            "{name}: canonical encoding drifted"
        );
    }
}

#[test]
fn rust_decodes_every_valid_response_fixture() {
    for case in cases()["valid_responses"].as_array().expect("valid") {
        let name = case["name"].as_str().expect("name");
        let line = case["encoded"].as_str().expect("encoded");
        let response = decode_response(line).unwrap_or_else(|error| panic!("{name}: {error}"));

        assert_eq!(
            response.id().get(),
            case["id"].as_u64().expect("id"),
            "{name}: correlation id"
        );
        assert_eq!(
            response.status(),
            case["kind"].as_str().expect("kind"),
            "{name}: status"
        );
        if let Response::Ok { rows, .. } = &response {
            let expected = case["rows"].as_array().expect("rows");
            assert_eq!(rows.len(), expected.len(), "{name}: row count");
        }
        if let Response::Error { error, .. } = &response {
            assert_eq!(
                error.code,
                case["error"]["code"].as_str().expect("code"),
                "{name}: error code"
            );
        }
    }
}

#[test]
fn rust_rejects_every_invalid_response_fixture_with_the_agreed_kind() {
    for case in cases()["invalid_responses"].as_array().expect("invalid") {
        let name = case["name"].as_str().expect("name");
        let line = case["encoded"].as_str().expect("encoded");
        let error = decode_response(line)
            .err()
            .unwrap_or_else(|| panic!("{name}: expected a protocol error"));
        assert_eq!(
            error.kind(),
            case["error"].as_str().expect("error"),
            "{name}: wrong error classification ({error})"
        );
    }
}

// --- encoding surfaces ---------------------------------------------------

#[test]
fn every_response_variant_encodes_and_decodes_back() {
    use crawl_protocol::{encode_response, PluginError};

    let responses = vec![
        Response::Ready {
            id: RequestId::new(0),
            api_version: "1".into(),
            plugin: Some("p".into()),
            sdk: Some("crawl-plugin-sdk/0.1.0".into()),
        },
        Response::Ready {
            id: RequestId::new(0),
            api_version: "1".into(),
            plugin: None,
            sdk: None,
        },
        Response::Ok {
            id: RequestId::new(1),
            rows: vec![],
        },
        Response::Error {
            id: RequestId::new(2),
            error: PluginError {
                code: "boom".into(),
                message: "it broke".into(),
                detail: Some(serde_json::json!({"errno": 2})),
            },
        },
    ];

    for response in responses {
        let line = encode_response(&response).expect("encode");
        assert!(
            line.ends_with('\n'),
            "protocol lines must be newline framed"
        );
        assert_eq!(
            decode_response(line.trim_end()).expect("decode"),
            response,
            "round trip changed the message"
        );
    }
}

#[test]
fn nested_and_non_finite_row_values_encode_defensively() {
    use crawl_domain::record::RawValue;
    use crawl_protocol::{encode_response, record_from_pairs};

    let row = record_from_pairs(
        [
            (
                "nested".to_owned(),
                RawValue::Object(
                    [("k".to_owned(), RawValue::Integer(1))]
                        .into_iter()
                        .collect(),
                ),
            ),
            (
                "list".to_owned(),
                RawValue::Array(vec![RawValue::Bool(true)]),
            ),
            // JSON has no representation for NaN, so it degrades to null
            // rather than producing a line no parser could read.
            ("nan".to_owned(), RawValue::Number(f64::NAN)),
        ]
        .into_iter()
        .collect(),
    );

    let line = encode_response(&Response::Ok {
        id: RequestId::new(1),
        rows: vec![row],
    })
    .expect("encode");
    assert!(line.contains("\"nan\":null"));

    // The line still parses, and the nested values survive intact so schema
    // validation can reject them by name. Degrading NaN must not corrupt
    // anything around it.
    let Response::Ok { rows, .. } = decode_response(line.trim_end()).expect("still parses") else {
        panic!("expected an ok response");
    };
    assert!(rows[0].get("nested").expect("nested").is_nested());
    assert!(rows[0].get("list").expect("list").is_nested());
    assert_eq!(
        rows[0].get("nan"),
        Some(&crawl_domain::record::RawValue::Null)
    );
}

#[test]
fn every_protocol_error_renders_a_useful_message() {
    use crawl_protocol::ProtocolError;

    let errors = vec![
        ProtocolError::MalformedJson {
            reason: "expected value".into(),
            excerpt: "junk".into(),
        },
        ProtocolError::NotAnObject {
            found: "array",
            excerpt: "[]".into(),
        },
        ProtocolError::MissingField { field: "rows" },
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
            index: 0,
            found: "string",
        },
        ProtocolError::UnsupportedVersion {
            declared: "2".into(),
            supported: "1",
        },
    ];

    for error in errors {
        let rendered = error.to_string();
        assert!(!rendered.is_empty(), "{:?} rendered empty", error.kind());
        assert!(!error.kind().is_empty());
    }
}
