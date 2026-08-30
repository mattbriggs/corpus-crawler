//! The plugin output-schema language and the record validator.
//!
//! # Purpose
//!
//! A plugin declares an ordered map of output fields in its manifest. The host
//! uses that declaration twice: to derive the CSV column layout before any file
//! is processed (REQ-098, REQ-103), and to validate every candidate record
//! before it reaches the report writer (REQ-095, REQ-099).
//!
//! # Design decisions
//!
//! * `required: true` means "present and non-null"; `required: false` (the
//!   default) means "may be absent or null". That is the single nullability
//!   convention required by REQ-156.
//! * No lossy coercion is ever performed (VAL-026). The one widening rule is
//!   documented on [`FieldType::accept`].
//! * Nested values are rejected outright (REQ-093, REQ-094, AC-028).

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

use crate::record::{RawRecord, RawValue, ScalarValue, ValidatedRecord};

/// A scalar type supported by the v1 schema language (REQ-091).
///
/// `datetime` (REQ-092, OQ-022) is deliberately absent: the SRS makes it
/// optional and does not define its canonical lexical or CSV representation.
/// See `site/adr/0005-schema-type-system.md`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    /// UTF-8 text.
    String,
    /// 64-bit signed integer.
    Integer,
    /// Double-precision floating point.
    Number,
    /// Boolean.
    Boolean,
}

impl FieldType {
    /// Returns the manifest spelling of the type.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::String => "string",
            Self::Integer => "integer",
            Self::Number => "number",
            Self::Boolean => "boolean",
        }
    }

    /// Converts a non-null raw value to a scalar of this type, or reports why
    /// it is incompatible.
    ///
    /// # Coercion rules
    ///
    /// * `integer` accepts a JSON integer. It also accepts a JSON float whose
    ///   fractional part is zero and which is exactly representable as `i64`,
    ///   because JSON encoders legitimately render `7` as `7.0`. Every other
    ///   float, and every string, is rejected (VAL-025).
    /// * `number` accepts JSON integers and finite floats; integer-to-float
    ///   widening is the only permitted conversion, and non-finite floats are
    ///   rejected because they have no CSV representation.
    /// * `boolean` and `string` accept only their own JSON type. Strings are
    ///   never parsed into other types (VAL-026).
    ///
    /// # Errors
    ///
    /// Returns [`TypeMismatch`] describing the expected and actual types.
    pub fn accept(self, value: &RawValue) -> Result<ScalarValue, TypeMismatch> {
        let mismatch = || TypeMismatch {
            expected: self,
            actual: value.type_name(),
        };
        match (self, value) {
            (Self::String, RawValue::String(text)) => Ok(ScalarValue::String(text.clone())),
            (Self::Boolean, RawValue::Bool(flag)) => Ok(ScalarValue::Bool(*flag)),
            (Self::Integer, RawValue::Integer(number)) => Ok(ScalarValue::Integer(*number)),
            (Self::Integer, RawValue::Number(number)) => {
                if number.fract() == 0.0
                    && number.is_finite()
                    && *number >= i64::MIN as f64
                    && *number <= i64::MAX as f64
                {
                    Ok(ScalarValue::Integer(*number as i64))
                } else {
                    Err(mismatch())
                }
            }
            (Self::Number, RawValue::Integer(number)) => Ok(ScalarValue::Number(*number as f64)),
            (Self::Number, RawValue::Number(number)) => {
                if number.is_finite() {
                    Ok(ScalarValue::Number(*number))
                } else {
                    Err(mismatch())
                }
            }
            _ => Err(mismatch()),
        }
    }
}

impl fmt::Display for FieldType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

/// A declared type incompatibility between schema and record value.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TypeMismatch {
    /// The type declared by the schema.
    pub expected: FieldType,
    /// The JSON type actually supplied by the plugin.
    pub actual: &'static str,
}

/// One declared output field (SRS 8.2).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SchemaField {
    /// Field name, used verbatim as the CSV column header.
    pub name: String,
    /// Declared scalar type.
    pub r#type: FieldType,
    /// When `true` the field must be present and non-null in every record.
    pub required: bool,
}

/// Reasons a declared schema is itself invalid.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaDefinitionError {
    /// The schema declared no fields.
    #[error("output schema must declare at least one field")]
    Empty,
    /// The same field name was declared more than once (VAL-020).
    #[error("output schema declares duplicate field {0:?}")]
    DuplicateField(String),
    /// A field name was empty or contained control characters.
    #[error("output schema field name {0:?} is invalid: {1}")]
    InvalidFieldName(String, &'static str),
}

/// An ordered, validated set of output fields.
///
/// Field order is the CSV column order for the whole crawl (REQ-098, REQ-103).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "Vec<SchemaField>", into = "Vec<SchemaField>")]
pub struct OutputSchema {
    fields: Vec<SchemaField>,
}

impl OutputSchema {
    /// Validates and constructs a schema from declaration order.
    ///
    /// # Errors
    ///
    /// Returns [`SchemaDefinitionError`] when the schema is empty, declares a
    /// duplicate field name, or declares an unusable field name.
    pub fn new(fields: Vec<SchemaField>) -> Result<Self, SchemaDefinitionError> {
        if fields.is_empty() {
            return Err(SchemaDefinitionError::Empty);
        }
        let mut seen: IndexMap<&str, ()> = IndexMap::new();
        for field in &fields {
            if field.name.trim().is_empty() {
                return Err(SchemaDefinitionError::InvalidFieldName(
                    field.name.clone(),
                    "must not be empty",
                ));
            }
            if field.name.chars().any(char::is_control) {
                return Err(SchemaDefinitionError::InvalidFieldName(
                    field.name.clone(),
                    "must not contain control characters",
                ));
            }
            if seen.insert(field.name.as_str(), ()).is_some() {
                return Err(SchemaDefinitionError::DuplicateField(field.name.clone()));
            }
        }
        Ok(Self { fields })
    }

    /// Returns the declared fields in schema order.
    #[must_use]
    pub fn fields(&self) -> &[SchemaField] {
        &self.fields
    }

    /// Returns the CSV header row derived from the schema (REQ-180).
    #[must_use]
    pub fn header(&self) -> Vec<String> {
        self.fields.iter().map(|field| field.name.clone()).collect()
    }

    /// Returns the number of declared columns.
    #[must_use]
    pub fn len(&self) -> usize {
        self.fields.len()
    }

    /// Returns `true` when the schema declares no fields. Always `false` for a
    /// constructed schema; provided for lint symmetry with [`Self::len`].
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.fields.is_empty()
    }

    /// Validates one candidate record against this schema.
    ///
    /// # Caller contract
    ///
    /// Called once per row of a protocol-valid success response, before the
    /// row is handed to the report writer (REQ-099). Never call it on protocol
    /// error payloads: protocol errors and schema errors are distinct
    /// categories (SRS 8.6).
    ///
    /// # Errors
    ///
    /// Returns the first [`SchemaViolation`] found. Validation is fail-fast per
    /// record because a record is accepted or rejected as a whole (REQ-096).
    pub fn validate(
        &self,
        record: &RawRecord,
        extra_fields: ExtraFieldPolicy,
    ) -> Result<ValidatedRecord, SchemaViolation> {
        if extra_fields == ExtraFieldPolicy::Reject {
            for name in record.field_names() {
                if !self.fields.iter().any(|field| field.name == name) {
                    return Err(SchemaViolation::UndeclaredField {
                        field: name.to_owned(),
                    });
                }
            }
        }

        let mut values = Vec::with_capacity(self.fields.len());
        for field in &self.fields {
            let value = match record.get(&field.name) {
                None | Some(RawValue::Null) => {
                    if field.required {
                        return Err(if record.get(&field.name).is_none() {
                            SchemaViolation::MissingRequiredField {
                                field: field.name.clone(),
                            }
                        } else {
                            SchemaViolation::NullRequiredField {
                                field: field.name.clone(),
                            }
                        });
                    }
                    ScalarValue::Null
                }
                Some(value) if value.is_nested() => {
                    return Err(SchemaViolation::NestedValue {
                        field: field.name.clone(),
                        actual: value.type_name(),
                    });
                }
                Some(value) => field.r#type.accept(value).map_err(|mismatch| {
                    SchemaViolation::TypeMismatch {
                        field: field.name.clone(),
                        expected: mismatch.expected,
                        actual: mismatch.actual,
                    }
                })?,
            };
            values.push(value);
        }
        Ok(ValidatedRecord::from_ordered_values(values))
    }
}

impl TryFrom<Vec<SchemaField>> for OutputSchema {
    type Error = SchemaDefinitionError;

    fn try_from(value: Vec<SchemaField>) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

impl From<OutputSchema> for Vec<SchemaField> {
    fn from(value: OutputSchema) -> Self {
        value.fields
    }
}

/// Policy for record fields the schema does not declare (OQ-012, VAL-023).
///
/// The default is [`ExtraFieldPolicy::Reject`], which preserves a deterministic
/// output contract. A plugin may opt into [`ExtraFieldPolicy::Ignore`] through
/// `output.extra_fields` in its manifest, which keeps the decision reversible
/// without changing the engine.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ExtraFieldPolicy {
    /// Reject any record carrying an undeclared field.
    #[default]
    Reject,
    /// Silently drop undeclared fields.
    Ignore,
}

/// A record-level schema violation (REQ-096, REQ-097).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum SchemaViolation {
    /// A required field was absent (VAL-022).
    #[error("required field {field:?} is missing")]
    MissingRequiredField {
        /// The offending field name.
        field: String,
    },
    /// A required field was present but null (VAL-024).
    #[error("required field {field:?} is null")]
    NullRequiredField {
        /// The offending field name.
        field: String,
    },
    /// A value had a type incompatible with the declaration.
    #[error("field {field:?} expected {expected} but received {actual}")]
    TypeMismatch {
        /// The offending field name.
        field: String,
        /// The declared type.
        expected: FieldType,
        /// The JSON type actually supplied.
        actual: &'static str,
    },
    /// A value was a nested array or object (REQ-093, AC-028).
    #[error("field {field:?} contains a nested {actual}; v1 records must be flat")]
    NestedValue {
        /// The offending field name.
        field: String,
        /// The nested JSON type supplied.
        actual: &'static str,
    },
    /// The record carried a field the schema does not declare.
    #[error("record contains undeclared field {field:?}")]
    UndeclaredField {
        /// The offending field name.
        field: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    fn field(name: &str, r#type: FieldType, required: bool) -> SchemaField {
        SchemaField {
            name: name.to_owned(),
            r#type,
            required,
        }
    }

    fn schema() -> OutputSchema {
        OutputSchema::new(vec![
            field("filename", FieldType::String, true),
            field("line", FieldType::Integer, true),
            field("score", FieldType::Number, false),
            field("flag", FieldType::Boolean, false),
        ])
        .unwrap()
    }

    fn record(pairs: Vec<(&str, RawValue)>) -> RawRecord {
        pairs
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect()
    }

    #[test]
    fn duplicate_field_names_are_rejected() {
        let error = OutputSchema::new(vec![
            field("a", FieldType::String, true),
            field("a", FieldType::Integer, false),
        ])
        .unwrap_err();
        assert_eq!(error, SchemaDefinitionError::DuplicateField("a".into()));
    }

    #[test]
    fn empty_schema_is_rejected() {
        assert_eq!(
            OutputSchema::new(vec![]).unwrap_err(),
            SchemaDefinitionError::Empty
        );
    }

    #[test]
    fn invalid_field_names_are_rejected() {
        assert!(matches!(
            OutputSchema::new(vec![field("", FieldType::String, true)]).unwrap_err(),
            SchemaDefinitionError::InvalidFieldName(_, _)
        ));
        assert!(matches!(
            OutputSchema::new(vec![field("a\nb", FieldType::String, true)]).unwrap_err(),
            SchemaDefinitionError::InvalidFieldName(_, _)
        ));
    }

    #[test]
    fn header_follows_declaration_order() {
        assert_eq!(schema().header(), vec!["filename", "line", "score", "flag"]);
    }

    #[test]
    fn values_are_ordered_by_schema_not_arrival() {
        let record = record(vec![
            ("line", RawValue::Integer(7)),
            ("filename", RawValue::String("a.txt".into())),
        ]);
        let validated = schema()
            .validate(&record, ExtraFieldPolicy::Reject)
            .unwrap();
        assert_eq!(
            validated.to_csv_fields(),
            vec![
                "a.txt".to_owned(),
                "7".to_owned(),
                String::new(),
                String::new()
            ]
        );
    }

    #[test]
    fn missing_and_null_required_fields_are_distinguished() {
        let missing = schema()
            .validate(
                &record(vec![("filename", RawValue::String("a".into()))]),
                ExtraFieldPolicy::Reject,
            )
            .unwrap_err();
        assert_eq!(
            missing,
            SchemaViolation::MissingRequiredField {
                field: "line".into()
            }
        );

        let null = schema()
            .validate(
                &record(vec![
                    ("filename", RawValue::String("a".into())),
                    ("line", RawValue::Null),
                ]),
                ExtraFieldPolicy::Reject,
            )
            .unwrap_err();
        assert_eq!(
            null,
            SchemaViolation::NullRequiredField {
                field: "line".into()
            }
        );
    }

    #[test]
    fn optional_fields_accept_absent_and_null() {
        let validated = schema()
            .validate(
                &record(vec![
                    ("filename", RawValue::String("a".into())),
                    ("line", RawValue::Integer(1)),
                    ("score", RawValue::Null),
                ]),
                ExtraFieldPolicy::Reject,
            )
            .unwrap();
        assert_eq!(validated.values()[2], ScalarValue::Null);
        assert_eq!(validated.values()[3], ScalarValue::Null);
    }

    #[test]
    fn integer_accepts_integral_float_and_rejects_fraction_or_string() {
        // The one documented widening: encoders legitimately render 7 as 7.0.
        assert_eq!(
            FieldType::Integer.accept(&RawValue::Number(7.0)).unwrap(),
            ScalarValue::Integer(7)
        );
        assert_eq!(
            FieldType::Integer.accept(&RawValue::Number(-0.0)).unwrap(),
            ScalarValue::Integer(0)
        );

        // Everything else is a mismatch that names both types, because the
        // author needs to know what was expected and what arrived (VAL-025).
        for (value, actual) in [
            (RawValue::Number(7.5), "number"),
            (RawValue::String("7".into()), "string"),
            (RawValue::Bool(true), "boolean"),
            (RawValue::Number(f64::NAN), "number"),
            (RawValue::Number(1e300), "number"),
        ] {
            assert_eq!(
                FieldType::Integer.accept(&value).unwrap_err(),
                TypeMismatch {
                    expected: FieldType::Integer,
                    actual
                },
                "{value:?} was not reported as an integer mismatch"
            );
        }
    }

    #[test]
    fn number_widens_integers_and_rejects_non_finite() {
        assert_eq!(
            FieldType::Number.accept(&RawValue::Integer(3)).unwrap(),
            ScalarValue::Number(3.0)
        );
        assert!(FieldType::Number
            .accept(&RawValue::Number(f64::NAN))
            .is_err());
        assert!(FieldType::Number
            .accept(&RawValue::Number(f64::INFINITY))
            .is_err());
    }

    #[test]
    fn strings_and_booleans_are_never_coerced() {
        // A string column must not absorb numbers, and a boolean column must
        // not parse "true": silent coercion is exactly what VAL-026 forbids.
        for (field_type, value, actual) in [
            (FieldType::String, RawValue::Integer(1), "integer"),
            (FieldType::String, RawValue::Bool(true), "boolean"),
            (FieldType::String, RawValue::Number(1.5), "number"),
            (
                FieldType::Boolean,
                RawValue::String("true".into()),
                "string",
            ),
            (FieldType::Boolean, RawValue::Integer(1), "integer"),
        ] {
            assert_eq!(
                field_type.accept(&value).unwrap_err(),
                TypeMismatch {
                    expected: field_type,
                    actual
                },
                "{field_type} wrongly accepted {value:?}"
            );
        }
    }

    #[test]
    fn nested_values_are_rejected() {
        let violation = schema()
            .validate(
                &record(vec![
                    ("filename", RawValue::Array(vec![RawValue::Integer(1)])),
                    ("line", RawValue::Integer(1)),
                ]),
                ExtraFieldPolicy::Reject,
            )
            .unwrap_err();
        assert_eq!(
            violation,
            SchemaViolation::NestedValue {
                field: "filename".into(),
                actual: "array"
            }
        );
    }

    #[test]
    fn extra_field_policy_switches_between_reject_and_ignore() {
        let candidate = record(vec![
            ("filename", RawValue::String("a".into())),
            ("line", RawValue::Integer(1)),
            ("surprise", RawValue::String("x".into())),
        ]);
        assert_eq!(
            schema()
                .validate(&candidate, ExtraFieldPolicy::Reject)
                .unwrap_err(),
            SchemaViolation::UndeclaredField {
                field: "surprise".into()
            }
        );
        assert!(schema()
            .validate(&candidate, ExtraFieldPolicy::Ignore)
            .is_ok());
    }
}
