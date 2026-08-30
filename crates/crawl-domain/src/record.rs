//! Record values crossing the plugin boundary.
//!
//! Two record shapes exist and must not be confused:
//!
//! * [`RawRecord`] is a *candidate* record. It has survived protocol decoding
//!   but not schema validation.
//! * [`ValidatedRecord`] has satisfied the plugin's declared output schema and
//!   is the only shape the report writer accepts (ARC-008, NFR-013).

use std::fmt;

use indexmap::IndexMap;
use serde::{Deserialize, Serialize};

/// A JSON-shaped value as received from a plugin worker, before validation.
///
/// The domain defines its own value type rather than re-exporting
/// `serde_json::Value` so that the domain layer stays independent of a
/// serialization library. `crawl-protocol` performs the conversion.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum RawValue {
    /// JSON `null`.
    Null,
    /// JSON boolean.
    Bool(bool),
    /// JSON number with no fractional part, representable as `i64`.
    Integer(i64),
    /// JSON number requiring floating-point representation.
    Number(f64),
    /// JSON string.
    String(String),
    /// JSON array. Rejected by schema validation in this release (REQ-094).
    Array(Vec<RawValue>),
    /// JSON object. Rejected by schema validation in this release (REQ-094).
    Object(IndexMap<String, RawValue>),
}

impl RawValue {
    /// Returns the JSON type name, for diagnostics.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) => "array",
            Self::Object(_) => "object",
        }
    }

    /// Returns `true` for arrays and objects, which the flat-record contract
    /// forbids as native values (REQ-093, REQ-094, AC-028).
    #[must_use]
    pub fn is_nested(&self) -> bool {
        matches!(self, Self::Array(_) | Self::Object(_))
    }
}

/// A candidate record: field names mapped to unvalidated values.
///
/// Insertion order is preserved for diagnostics only. CSV column order is
/// always derived from the declared schema, never from this map (REQ-098).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
#[serde(transparent)]
pub struct RawRecord(pub IndexMap<String, RawValue>);

impl RawRecord {
    /// Creates an empty candidate record.
    #[must_use]
    pub fn new() -> Self {
        Self(IndexMap::new())
    }

    /// Looks up a field by name.
    #[must_use]
    pub fn get(&self, field: &str) -> Option<&RawValue> {
        self.0.get(field)
    }

    /// Returns the field names present on this record, in arrival order.
    pub fn field_names(&self) -> impl Iterator<Item = &str> {
        self.0.keys().map(String::as_str)
    }

    /// Inserts a field, replacing any previous value.
    pub fn insert(&mut self, field: impl Into<String>, value: RawValue) -> Option<RawValue> {
        self.0.insert(field.into(), value)
    }

    /// Returns the number of fields present.
    #[must_use]
    pub fn len(&self) -> usize {
        self.0.len()
    }

    /// Returns `true` when the record carries no fields.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }
}

impl FromIterator<(String, RawValue)> for RawRecord {
    fn from_iter<T: IntoIterator<Item = (String, RawValue)>>(iter: T) -> Self {
        Self(iter.into_iter().collect())
    }
}

/// A schema-legal scalar value.
///
/// Nested structures cannot be represented, which makes "a validated record is
/// flat" a type-level guarantee rather than a runtime convention.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ScalarValue {
    /// An absent or explicitly null value for a nullable field.
    Null,
    /// A boolean value.
    Bool(bool),
    /// A 64-bit signed integer.
    Integer(i64),
    /// A double-precision number.
    Number(f64),
    /// A UTF-8 string.
    String(String),
}

impl ScalarValue {
    /// Renders the value using the canonical CSV representation for its type
    /// (REQ-182).
    ///
    /// Null renders as the empty field. Booleans render as `true`/`false`.
    /// Integers render without a decimal point. Numbers use Rust's shortest
    /// round-trippable form, with non-finite values rejected earlier by schema
    /// validation.
    #[must_use]
    pub fn to_csv_field(&self) -> String {
        match self {
            Self::Null => String::new(),
            Self::Bool(true) => "true".to_owned(),
            Self::Bool(false) => "false".to_owned(),
            Self::Integer(value) => value.to_string(),
            Self::Number(value) => {
                let mut rendered = value.to_string();
                if rendered == "-0" {
                    rendered = "0".to_owned();
                }
                rendered
            }
            Self::String(value) => value.clone(),
        }
    }

    /// Returns the schema type name of this value, for diagnostics.
    #[must_use]
    pub fn type_name(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Integer(_) => "integer",
            Self::Number(_) => "number",
            Self::String(_) => "string",
        }
    }
}

impl fmt::Display for ScalarValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.to_csv_field())
    }
}

/// A record that has satisfied its plugin's declared output schema.
///
/// Values are stored positionally in schema order, so the report writer never
/// has to re-derive column order and cannot accidentally emit a row whose
/// arity differs from the CSV header (REQ-098, REQ-103).
#[derive(Debug, Clone, PartialEq)]
pub struct ValidatedRecord {
    values: Vec<ScalarValue>,
}

impl ValidatedRecord {
    /// Creates a validated record from values already ordered by schema
    /// position.
    ///
    /// # Caller contract
    ///
    /// Only [`crate::schema::OutputSchema::validate`] should call this: it is
    /// the single place where the ordering invariant is established.
    #[must_use]
    pub fn from_ordered_values(values: Vec<ScalarValue>) -> Self {
        Self { values }
    }

    /// Returns the values in schema order.
    #[must_use]
    pub fn values(&self) -> &[ScalarValue] {
        &self.values
    }

    /// Returns the number of columns in the record.
    #[must_use]
    pub fn len(&self) -> usize {
        self.values.len()
    }

    /// Returns `true` when the record has no columns.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.values.is_empty()
    }

    /// Renders the record as CSV fields in schema order.
    #[must_use]
    pub fn to_csv_fields(&self) -> Vec<String> {
        self.values.iter().map(ScalarValue::to_csv_field).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn csv_rendering_matches_declared_types() {
        assert_eq!(ScalarValue::Null.to_csv_field(), "");
        assert_eq!(ScalarValue::Bool(true).to_csv_field(), "true");
        assert_eq!(ScalarValue::Integer(-7).to_csv_field(), "-7");
        assert_eq!(ScalarValue::Number(1.5).to_csv_field(), "1.5");
        assert_eq!(ScalarValue::Number(-0.0).to_csv_field(), "0");
        assert_eq!(ScalarValue::String("a,b".into()).to_csv_field(), "a,b");
    }

    #[test]
    fn nested_values_are_detectable() {
        assert!(RawValue::Array(vec![]).is_nested());
        assert!(RawValue::Object(IndexMap::new()).is_nested());
        assert!(!RawValue::String("x".into()).is_nested());
        assert_eq!(RawValue::Null.type_name(), "null");
    }

    #[test]
    fn validated_record_preserves_positional_order() {
        let record = ValidatedRecord::from_ordered_values(vec![
            ScalarValue::String("a".into()),
            ScalarValue::Integer(1),
        ]);
        assert_eq!(record.to_csv_fields(), vec!["a".to_owned(), "1".to_owned()]);
        assert_eq!(record.len(), 2);
        assert!(!record.is_empty());
    }
}
