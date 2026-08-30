//! Encoding and strict decoding of protocol lines.
//!
//! Decoding is written by hand against `serde_json::Value` rather than derived,
//! because the host must be able to say *precisely* which part of the contract
//! a worker violated (NFR-050) and must distinguish a malformed line from a
//! well-formed line that breaks the contract (REQ-086 vs REQ-087).

use crawl_domain::ids::RequestId;
use crawl_domain::record::{RawRecord, RawValue};
use indexmap::IndexMap;
use serde_json::{Map, Value};

use crate::error::{excerpt, json_type_name, ProtocolError};
use crate::request::Request;
use crate::response::{PluginError, Response};

/// Top-level fields defined for a `ready` message.
const READY_FIELDS: [&str; 5] = ["id", "status", "api_version", "plugin", "sdk"];
/// Top-level fields defined for an `ok` message.
const OK_FIELDS: [&str; 3] = ["id", "status", "rows"];
/// Top-level fields defined for an `error` message.
const ERROR_FIELDS: [&str; 3] = ["id", "status", "error"];

/// Serializes a request as one newline-terminated protocol line.
///
/// # Errors
///
/// Returns [`serde_json::Error`] only if the request contains a string that
/// cannot be represented in JSON, which cannot happen for validated paths.
pub fn encode_request(request: &Request) -> Result<String, serde_json::Error> {
    let mut line = serde_json::to_string(request)?;
    line.push('\n');
    Ok(line)
}

/// Serializes a response as one newline-terminated protocol line.
///
/// Used by tests and by the reference worker fixtures; the host never produces
/// responses in production.
///
/// # Errors
///
/// Returns [`serde_json::Error`] when the response cannot be serialized.
pub fn encode_response(response: &Response) -> Result<String, serde_json::Error> {
    let value = match response {
        Response::Ready {
            id,
            api_version,
            plugin,
            sdk,
        } => {
            let mut map = Map::new();
            map.insert("id".into(), Value::from(id.get()));
            map.insert("status".into(), Value::from("ready"));
            map.insert("api_version".into(), Value::from(api_version.clone()));
            if let Some(plugin) = plugin {
                map.insert("plugin".into(), Value::from(plugin.clone()));
            }
            if let Some(sdk) = sdk {
                map.insert("sdk".into(), Value::from(sdk.clone()));
            }
            Value::Object(map)
        }
        Response::Ok { id, rows } => {
            let mut map = Map::new();
            map.insert("id".into(), Value::from(id.get()));
            map.insert("status".into(), Value::from("ok"));
            map.insert(
                "rows".into(),
                Value::Array(rows.iter().map(raw_record_to_json).collect()),
            );
            Value::Object(map)
        }
        Response::Error { id, error } => {
            let mut map = Map::new();
            map.insert("id".into(), Value::from(id.get()));
            map.insert("status".into(), Value::from("error"));
            map.insert("error".into(), serde_json::to_value(error)?);
            Value::Object(map)
        }
    };
    let mut line = serde_json::to_string(&value)?;
    line.push('\n');
    Ok(line)
}

/// Decodes one protocol line into a [`Response`].
///
/// # Caller contract
///
/// `line` is a single complete line read from a worker's protocol `stdout`,
/// without its trailing newline (REQ-161). Blank lines are a protocol
/// violation rather than a no-op: a worker that emits stray blank output on
/// its protocol stream has desynchronised the stream.
///
/// # Errors
///
/// Returns a [`ProtocolError`] naming the exact contract violation.
pub fn decode_response(line: &str) -> Result<Response, ProtocolError> {
    let value: Value =
        serde_json::from_str(line).map_err(|error| ProtocolError::MalformedJson {
            reason: error.to_string(),
            excerpt: excerpt(line),
        })?;

    let Value::Object(object) = value else {
        return Err(ProtocolError::NotAnObject {
            found: json_type_name(&value),
            excerpt: excerpt(line),
        });
    };

    let id = read_id(&object)?;
    let status = read_status(&object)?;

    match status.as_str() {
        "ready" => {
            reject_undefined_fields(&object, &READY_FIELDS)?;
            let api_version = read_required_string(&object, "api_version")?;
            Ok(Response::Ready {
                id,
                api_version,
                plugin: read_optional_string(&object, "plugin")?,
                sdk: read_optional_string(&object, "sdk")?,
            })
        }
        "ok" => {
            reject_undefined_fields(&object, &OK_FIELDS)?;
            let rows_value = object
                .get("rows")
                .ok_or(ProtocolError::MissingField { field: "rows" })?;
            let Value::Array(rows) = rows_value else {
                return Err(ProtocolError::InvalidFieldType {
                    field: "rows",
                    expected: "an array",
                    found: json_type_name(rows_value),
                });
            };
            let mut records = Vec::with_capacity(rows.len());
            for (index, row) in rows.iter().enumerate() {
                let Value::Object(row) = row else {
                    return Err(ProtocolError::RowNotAnObject {
                        index,
                        found: json_type_name(row),
                    });
                };
                records.push(json_object_to_raw_record(row));
            }
            Ok(Response::Ok { id, rows: records })
        }
        "error" => {
            reject_undefined_fields(&object, &ERROR_FIELDS)?;
            let error_value = object
                .get("error")
                .ok_or(ProtocolError::MissingField { field: "error" })?;
            let Value::Object(error_object) = error_value else {
                return Err(ProtocolError::InvalidFieldType {
                    field: "error",
                    expected: "an object",
                    found: json_type_name(error_value),
                });
            };
            let code = read_required_string(error_object, "code")?;
            let message = read_required_string(error_object, "message")?;
            Ok(Response::Error {
                id,
                error: PluginError {
                    code,
                    message,
                    detail: error_object.get("detail").cloned(),
                },
            })
        }
        other => Err(ProtocolError::UnknownStatus {
            status: other.to_owned(),
        }),
    }
}

/// Reads the correlation identifier.
///
/// Negative and fractional numbers are rejected rather than truncated: an `id`
/// the host cannot match against an outstanding request is a desynchronised
/// stream, not a value to salvage.
fn read_id(object: &Map<String, Value>) -> Result<RequestId, ProtocolError> {
    let value = object
        .get("id")
        .ok_or(ProtocolError::MissingField { field: "id" })?;
    value
        .as_u64()
        .map(RequestId::new)
        .ok_or(ProtocolError::InvalidFieldType {
            field: "id",
            expected: "a non-negative integer",
            found: json_type_name(value),
        })
}

/// Reads the discriminant that selects the message shape.
///
/// Returned as an owned `String` so the caller can name an unrecognised status
/// in its error without borrowing from the parsed document.
fn read_status(object: &Map<String, Value>) -> Result<String, ProtocolError> {
    let value = object
        .get("status")
        .ok_or(ProtocolError::MissingField { field: "status" })?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(ProtocolError::InvalidFieldType {
            field: "status",
            expected: "a string",
            found: json_type_name(value),
        })
}

/// Reads a field that must be present and must be a string.
///
/// A JSON `null` fails here, unlike in [`read_optional_string`], because a
/// required field explicitly set to null is a contract violation rather than
/// an omission.
fn read_required_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<String, ProtocolError> {
    let value = object
        .get(field)
        .ok_or(ProtocolError::MissingField { field })?;
    value
        .as_str()
        .map(str::to_owned)
        .ok_or(ProtocolError::InvalidFieldType {
            field,
            expected: "a string",
            found: json_type_name(value),
        })
}

/// Reads a field that may be absent.
///
/// An explicit `null` is treated as absence, which keeps the two natural ways
/// of writing "no value" interchangeable for the optional diagnostic fields
/// this is used for. A non-string value is still an error.
fn read_optional_string(
    object: &Map<String, Value>,
    field: &'static str,
) -> Result<Option<String>, ProtocolError> {
    match object.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text.clone())),
        Some(other) => Err(ProtocolError::InvalidFieldType {
            field,
            expected: "a string",
            found: json_type_name(other),
        }),
    }
}

/// Rejects any top-level field this protocol version does not define.
///
/// Unknown fields are refused rather than ignored so that a worker built
/// against a newer protocol fails immediately and visibly, instead of having
/// the extra data silently dropped and producing quietly incomplete output.
fn reject_undefined_fields(
    object: &Map<String, Value>,
    defined: &[&str],
) -> Result<(), ProtocolError> {
    for key in object.keys() {
        if !defined.contains(&key.as_str()) {
            return Err(ProtocolError::UndefinedField { field: key.clone() });
        }
    }
    Ok(())
}

/// Converts a decoded JSON object into a domain candidate record.
///
/// Nested values are carried through unchanged rather than rejected here: the
/// flat-record rule belongs to schema validation, which can attribute the
/// rejection to a declared field (REQ-093, AC-028).
fn json_object_to_raw_record(object: &Map<String, Value>) -> RawRecord {
    RawRecord(
        object
            .iter()
            .map(|(key, value)| (key.clone(), json_to_raw_value(value)))
            .collect(),
    )
}

/// Converts one decoded JSON value into the domain's own value type.
///
/// Numbers are split into `Integer` and `Number` here rather than at schema
/// validation, because `serde_json` has already decided which representation
/// the literal used and that distinction is what VAL-025 turns on.
fn json_to_raw_value(value: &Value) -> RawValue {
    match value {
        Value::Null => RawValue::Null,
        Value::Bool(flag) => RawValue::Bool(*flag),
        Value::Number(number) => {
            if let Some(integer) = number.as_i64() {
                RawValue::Integer(integer)
            } else {
                RawValue::Number(number.as_f64().unwrap_or(f64::NAN))
            }
        }
        Value::String(text) => RawValue::String(text.clone()),
        Value::Array(items) => RawValue::Array(items.iter().map(json_to_raw_value).collect()),
        Value::Object(entries) => RawValue::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), json_to_raw_value(value)))
                .collect(),
        ),
    }
}

/// Converts a candidate record back into JSON, preserving field order.
fn raw_record_to_json(record: &RawRecord) -> Value {
    Value::Object(
        record
            .0
            .iter()
            .map(|(key, value)| (key.clone(), raw_value_to_json(value)))
            .collect(),
    )
}

/// Converts a domain value back into JSON.
///
/// Non-finite floats become `null`, because JSON has no representation for
/// `NaN` or infinity. Emitting `null` keeps the line parseable so a single bad
/// value degrades one field rather than desynchronising the whole stream; the
/// field is then rejected by schema validation, which can name it.
fn raw_value_to_json(value: &RawValue) -> Value {
    match value {
        RawValue::Null => Value::Null,
        RawValue::Bool(flag) => Value::Bool(*flag),
        RawValue::Integer(number) => Value::from(*number),
        RawValue::Number(number) => {
            serde_json::Number::from_f64(*number).map_or(Value::Null, Value::Number)
        }
        RawValue::String(text) => Value::String(text.clone()),
        RawValue::Array(items) => Value::Array(items.iter().map(raw_value_to_json).collect()),
        RawValue::Object(entries) => Value::Object(
            entries
                .iter()
                .map(|(key, value)| (key.clone(), raw_value_to_json(value)))
                .collect(),
        ),
    }
}

/// Converts an [`IndexMap`] of raw values into a candidate record.
///
/// Exposed for fixture construction in tests.
#[must_use]
pub fn record_from_pairs(pairs: IndexMap<String, RawValue>) -> RawRecord {
    RawRecord(pairs)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_response_decodes_rows_in_arrival_order() {
        let line = r#"{"id":42,"status":"ok","rows":[{"filename":"/data/a.txt","line":7,"entity":"Example"}]}"#;
        let Response::Ok { id, rows } = decode_response(line).unwrap() else {
            panic!("expected ok response");
        };
        assert_eq!(id, RequestId::new(42));
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].get("line"), Some(&RawValue::Integer(7)));
        assert_eq!(
            rows[0].field_names().collect::<Vec<_>>(),
            vec!["filename", "line", "entity"]
        );
    }

    #[test]
    fn zero_and_multiple_rows_are_both_valid() {
        let Response::Ok { rows, .. } =
            decode_response(r#"{"id":1,"status":"ok","rows":[]}"#).unwrap()
        else {
            panic!("expected ok response");
        };
        assert!(rows.is_empty());

        let Response::Ok { rows, .. } =
            decode_response(r#"{"id":1,"status":"ok","rows":[{"a":1},{"a":2},{"a":3}]}"#).unwrap()
        else {
            panic!("expected ok response");
        };
        assert_eq!(rows.len(), 3);
    }

    #[test]
    fn error_response_decodes_code_message_and_detail() {
        let line = r#"{"id":3,"status":"error","error":{"code":"unreadable","message":"no such file","detail":{"errno":2}}}"#;
        let Response::Error { id, error } = decode_response(line).unwrap() else {
            panic!("expected error response");
        };
        assert_eq!(id, RequestId::new(3));
        assert_eq!(error.code, "unreadable");
        assert!(error.detail.is_some());
    }

    #[test]
    fn ready_response_decodes_version() {
        let line = r#"{"id":0,"status":"ready","api_version":"1","plugin":"echo"}"#;
        let Response::Ready {
            api_version,
            plugin,
            sdk,
            ..
        } = decode_response(line).unwrap()
        else {
            panic!("expected ready response");
        };
        assert_eq!(api_version, "1");
        assert_eq!(plugin.as_deref(), Some("echo"));
        assert!(sdk.is_none());
    }

    #[test]
    fn malformed_json_is_a_protocol_error() {
        let error = decode_response("this is not json").unwrap_err();
        assert_eq!(error.kind(), "malformed_json");
        assert!(error.to_string().contains("this is not json"));
    }

    #[test]
    fn empty_line_is_a_protocol_error() {
        assert_eq!(decode_response("").unwrap_err().kind(), "malformed_json");
    }

    #[test]
    fn non_object_messages_are_rejected() {
        assert_eq!(
            decode_response("[1,2,3]").unwrap_err().kind(),
            "not_an_object"
        );
        assert_eq!(decode_response("42").unwrap_err().kind(), "not_an_object");
    }

    #[test]
    fn missing_and_mistyped_fields_are_named() {
        assert_eq!(
            decode_response(r#"{"status":"ok","rows":[]}"#).unwrap_err(),
            ProtocolError::MissingField { field: "id" }
        );
        assert_eq!(
            decode_response(r#"{"id":1,"rows":[]}"#).unwrap_err(),
            ProtocolError::MissingField { field: "status" }
        );
        assert_eq!(
            decode_response(r#"{"id":1,"status":"ok"}"#).unwrap_err(),
            ProtocolError::MissingField { field: "rows" }
        );
        assert_eq!(
            decode_response(r#"{"id":"x","status":"ok","rows":[]}"#)
                .unwrap_err()
                .kind(),
            "invalid_field_type"
        );
        assert_eq!(
            decode_response(r#"{"id":-1,"status":"ok","rows":[]}"#)
                .unwrap_err()
                .kind(),
            "invalid_field_type"
        );
        assert_eq!(
            decode_response(r#"{"id":1,"status":"ok","rows":{}}"#)
                .unwrap_err()
                .kind(),
            "invalid_field_type"
        );
    }

    #[test]
    fn unknown_status_is_rejected() {
        let error = decode_response(r#"{"id":1,"status":"maybe","rows":[]}"#).unwrap_err();
        assert_eq!(
            error,
            ProtocolError::UnknownStatus {
                status: "maybe".into()
            }
        );
    }

    #[test]
    fn undefined_top_level_fields_are_rejected() {
        let error = decode_response(r#"{"id":1,"status":"ok","rows":[],"extra":1}"#).unwrap_err();
        assert_eq!(
            error,
            ProtocolError::UndefinedField {
                field: "extra".into()
            }
        );
    }

    #[test]
    fn non_object_rows_are_rejected_with_their_index() {
        let error =
            decode_response(r#"{"id":1,"status":"ok","rows":[{"a":1},"nope"]}"#).unwrap_err();
        assert_eq!(
            error,
            ProtocolError::RowNotAnObject {
                index: 1,
                found: "string"
            }
        );
    }

    #[test]
    fn error_payload_requires_code_and_message() {
        assert_eq!(
            decode_response(r#"{"id":1,"status":"error","error":{"message":"x"}}"#).unwrap_err(),
            ProtocolError::MissingField { field: "code" }
        );
        assert_eq!(
            decode_response(r#"{"id":1,"status":"error","error":{"code":"x"}}"#).unwrap_err(),
            ProtocolError::MissingField { field: "message" }
        );
        assert_eq!(
            decode_response(r#"{"id":1,"status":"error","error":"boom"}"#)
                .unwrap_err()
                .kind(),
            "invalid_field_type"
        );
    }

    #[test]
    fn nested_row_values_survive_decoding_for_schema_to_reject() {
        let Response::Ok { rows, .. } =
            decode_response(r#"{"id":1,"status":"ok","rows":[{"a":{"b":1},"c":[1,2]}]}"#).unwrap()
        else {
            panic!("expected ok response");
        };
        assert!(rows[0].get("a").unwrap().is_nested());
        assert!(rows[0].get("c").unwrap().is_nested());
    }

    #[test]
    fn utf8_paths_round_trip() {
        let request = Request::process(RequestId::new(1), "/data/ünïcode/文件.txt");
        let line = encode_request(&request).unwrap();
        assert!(line.ends_with('\n'));
        assert!(line.contains("文件.txt"));
        let decoded: Request = serde_json::from_str(line.trim_end()).unwrap();
        assert_eq!(decoded, request);
    }

    #[test]
    fn responses_round_trip_through_encode_and_decode() {
        let original = Response::Ok {
            id: RequestId::new(9),
            rows: vec![[
                ("filename".to_owned(), RawValue::String("/a.txt".to_owned())),
                ("line".to_owned(), RawValue::Integer(3)),
                ("ratio".to_owned(), RawValue::Number(0.5)),
                ("flag".to_owned(), RawValue::Bool(true)),
                ("note".to_owned(), RawValue::Null),
            ]
            .into_iter()
            .collect()],
        };
        let line = encode_response(&original).unwrap();
        assert_eq!(decode_response(line.trim_end()).unwrap(), original);
    }
}
