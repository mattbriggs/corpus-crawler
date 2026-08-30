//! The `plugin.yaml` contract: parsing, validation, and normalization.
//!
//! # Purpose
//!
//! This module is the boundary between untrusted YAML bytes and the validated
//! [`Plugin`] aggregate. It implements the four-stage pipeline the design
//! requires: YAML bytes -> syntactic YAML -> manifest DTO -> validated domain
//! model. Each stage has its own error variant so a plugin author is told
//! which stage rejected the manifest (NFR-050).
//!
//! # Strictness
//!
//! Every DTO denies unknown fields. A misspelled key is a rejected manifest
//! rather than a silently ignored setting, which is what makes the manifest a
//! contract rather than a suggestion (REQ-012).

use std::collections::BTreeSet;
use std::fmt;
use std::path::{Path, PathBuf};

use serde::de::{self, MapAccess, Visitor};
use serde::{Deserialize, Deserializer};

use crate::ids::{ApiVersion, IdentityError, PluginName, PluginVersion};
use crate::plugin::{ExecutionDefaults, InputSpec, Plugin, RuntimeSpec, RuntimeType};
use crate::policy::{PolicyError, QueueCapacity, RetryLimit, Timeout, WorkerCount};
use crate::schema::{
    ExtraFieldPolicy, FieldType, OutputSchema, SchemaDefinitionError, SchemaField,
};

/// Conventional file name of a plugin manifest.
pub const MANIFEST_FILE_NAME: &str = "plugin.yaml";

/// Errors produced while turning manifest bytes into a [`Plugin`].
#[derive(Debug, thiserror::Error)]
pub enum ManifestError {
    /// The manifest file could not be read.
    #[error("cannot read plugin manifest at {path}: {source}")]
    Io {
        /// The path that could not be read.
        path: PathBuf,
        /// The underlying I/O error.
        #[source]
        source: std::io::Error,
    },
    /// The manifest was not valid YAML or did not match the contract (VAL-001).
    #[error("invalid plugin manifest at {path}: {source}")]
    Syntax {
        /// The offending manifest path.
        path: PathBuf,
        /// The parse or shape error.
        #[source]
        source: serde_yaml::Error,
    },
    /// An identity value object rejected its input.
    #[error("invalid plugin manifest: {0}")]
    Identity(#[from] IdentityError),
    /// An execution value object rejected its input.
    #[error("invalid plugin manifest: {0}")]
    Policy(#[from] PolicyError),
    /// The declared output schema was itself invalid.
    #[error("invalid plugin manifest: {0}")]
    Schema(#[from] SchemaDefinitionError),
    /// The declared API version is well formed but not implemented (REQ-013).
    #[error(
        "plugin declares unsupported api_version {declared:?}; this host supports {supported:?}"
    )]
    UnsupportedApiVersion {
        /// The version the plugin declared.
        declared: String,
        /// The version this host implements.
        supported: &'static str,
    },
    /// An `input.extensions` entry was malformed (VAL-007).
    #[error("invalid file extension {value:?}: {reason}")]
    InvalidExtension {
        /// The rejected entry.
        value: String,
        /// Why it was rejected.
        reason: &'static str,
    },
    /// `input.extensions` was empty.
    #[error("input.extensions must declare at least one extension")]
    NoExtensions,
    /// `runtime.command` was empty (VAL-006).
    #[error("runtime.command must contain at least the program to execute")]
    EmptyCommand,
    /// A field type name was not part of the v1 type system (VAL-021).
    #[error("field {field:?} declares unsupported type {declared:?}; supported types are string, integer, number, boolean")]
    UnsupportedFieldType {
        /// The offending field.
        field: String,
        /// The unsupported type name.
        declared: String,
    },
}

/// Normalizes and validates one declared file extension (VAL-007).
///
/// # Caller contract
///
/// Accepts entries with or without a leading dot and normalizes to a
/// lowercase, dot-prefixed form so that `TXT`, `.txt` and `txt` all become
/// `.txt`. Rejects entries containing path separators, whitespace, interior
/// dots, or control characters.
///
/// # Errors
///
/// Returns [`ManifestError::InvalidExtension`] describing the rejection.
pub fn normalize_extension(value: &str) -> Result<String, ManifestError> {
    let trimmed = value.trim();
    let reject = |reason: &'static str| ManifestError::InvalidExtension {
        value: value.to_owned(),
        reason,
    };
    if trimmed.is_empty() {
        return Err(reject("must not be empty"));
    }
    let bare = trimmed.strip_prefix('.').unwrap_or(trimmed);
    if bare.is_empty() {
        return Err(reject("must contain characters after the leading dot"));
    }
    if bare.contains('.') {
        return Err(reject("must name a single extension without interior dots"));
    }
    if bare.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return Err(reject("must not contain whitespace or control characters"));
    }
    if bare.contains(['/', '\\']) {
        return Err(reject("must not contain path separators"));
    }
    Ok(format!(".{}", bare.to_ascii_lowercase()))
}

/// A parsed but not yet validated manifest.
///
/// Keeping the DTO separate from [`Plugin`] is what allows `crawl plugin
/// inspect` to report exactly what the author wrote alongside what the host
/// resolved.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginManifest {
    /// Declared host-plugin API version (REQ-150).
    #[serde(deserialize_with = "scalar_as_string")]
    pub api_version: String,
    /// Plugin identity (REQ-151).
    pub plugin: PluginIdentityDto,
    /// Worker runtime declaration (REQ-152).
    pub runtime: RuntimeDto,
    /// Input selection declaration (REQ-153).
    pub input: InputDto,
    /// Output format and schema declaration (REQ-154, REQ-155).
    pub output: OutputDto,
    /// Optional execution defaults (REQ-157).
    #[serde(default)]
    pub execution: ExecutionDto,
}

/// Manifest `plugin:` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PluginIdentityDto {
    /// Registry name.
    pub name: String,
    /// Plugin version.
    #[serde(deserialize_with = "scalar_as_string")]
    pub version: String,
    /// Optional human description.
    #[serde(default)]
    pub description: Option<String>,
}

/// Manifest `runtime:` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeDto {
    /// Runtime family; only `python` is supported in v1.
    #[serde(rename = "type")]
    pub runtime_type: RuntimeType,
    /// Argument vector for the worker process.
    pub command: Vec<String>,
    /// Optional working directory, relative to the plugin root.
    #[serde(default)]
    pub working_directory: Option<PathBuf>,
    /// Optional extra environment variables.
    #[serde(default)]
    pub env: Vec<EnvEntryDto>,
    /// Whether the plugin implements the optional `selftest` control request.
    #[serde(default)]
    pub selftest: bool,
}

/// One `runtime.env` entry.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvEntryDto {
    /// Variable name.
    pub name: String,
    /// Variable value.
    #[serde(deserialize_with = "scalar_as_string")]
    pub value: String,
}

/// Manifest `input:` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InputDto {
    /// Declared file extensions.
    pub extensions: Vec<String>,
    /// Whether traversal follows symbolic links (REQ-044). Defaults to `false`.
    #[serde(default)]
    pub follow_symlinks: bool,
}

/// Manifest `output:` block.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OutputDto {
    /// Output format; only `records` is supported in v1 (REQ-154).
    #[serde(default = "default_output_format")]
    pub format: OutputFormatDto,
    /// Declared output fields, in CSV column order.
    pub schema: OrderedSchemaDto,
    /// Policy for undeclared record fields (OQ-012).
    #[serde(default)]
    pub extra_fields: ExtraFieldPolicy,
}

/// The output format assumed when a manifest omits it.
fn default_output_format() -> OutputFormatDto {
    OutputFormatDto::Records
}

/// Supported output formats.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum OutputFormatDto {
    /// Zero or more flat records per file.
    Records,
}

/// Manifest `execution:` block.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExecutionDto {
    /// Preferred worker count, an integer or `auto`.
    #[serde(default)]
    pub workers: Option<WorkersDto>,
    /// Preferred per-file timeout in seconds.
    #[serde(default)]
    pub timeout_seconds: Option<f64>,
    /// Preferred retry allowance.
    #[serde(default)]
    pub max_retries: Option<u32>,
    /// Preferred bounded queue depth.
    #[serde(default)]
    pub queue_capacity: Option<u32>,
}

/// `execution.workers` as written in the manifest.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(untagged)]
pub enum WorkersDto {
    /// An explicit count.
    Fixed(u32),
    /// The literal `auto`.
    Auto(AutoDto),
}

/// The literal string `auto`.
#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum AutoDto {
    /// `auto`
    Auto,
}

/// One declared schema field, as written in the manifest.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SchemaFieldDto {
    /// Declared type name.
    #[serde(rename = "type")]
    pub field_type: String,
    /// Whether the field must be present and non-null. Defaults to `false`.
    #[serde(default)]
    pub required: bool,
    /// Optional human description, carried through to `plugin inspect`.
    #[serde(default)]
    pub description: Option<String>,
}

/// A YAML mapping of field name to declaration, preserving order and rejecting
/// duplicate keys.
///
/// YAML mappings deserialized into an ordinary map silently drop duplicate
/// keys, which would make VAL-020 unenforceable. This wrapper visits the map
/// directly so a duplicate is a parse error at the exact offending key.
#[derive(Debug, Clone)]
pub struct OrderedSchemaDto(pub Vec<(String, SchemaFieldDto)>);

impl<'de> Deserialize<'de> for OrderedSchemaDto {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        struct OrderedSchemaVisitor;

        impl<'de> Visitor<'de> for OrderedSchemaVisitor {
            type Value = OrderedSchemaDto;

            fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str("a mapping of output field names to declarations")
            }

            fn visit_map<A>(self, mut access: A) -> Result<Self::Value, A::Error>
            where
                A: MapAccess<'de>,
            {
                let mut fields: Vec<(String, SchemaFieldDto)> = Vec::new();
                while let Some((name, declaration)) =
                    access.next_entry::<String, SchemaFieldDto>()?
                {
                    if fields.iter().any(|(existing, _)| *existing == name) {
                        return Err(de::Error::custom(format!(
                            "duplicate output schema field {name:?}"
                        )));
                    }
                    fields.push((name, declaration));
                }
                Ok(OrderedSchemaDto(fields))
            }
        }

        deserializer.deserialize_map(OrderedSchemaVisitor)
    }
}

/// Deserializes a YAML scalar as a string, accepting unquoted numbers.
///
/// `api_version: 1` and `version: 1.0` are natural things to write and YAML
/// types them as numbers. Rejecting them would be a usability trap, so scalars
/// are rendered to their lexical form instead.
fn scalar_as_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = serde_yaml::Value::deserialize(deserializer)?;
    match value {
        serde_yaml::Value::String(text) => Ok(text),
        serde_yaml::Value::Number(number) => Ok(number.to_string()),
        serde_yaml::Value::Bool(flag) => Ok(flag.to_string()),
        other => Err(de::Error::custom(format!(
            "expected a string scalar, found {other:?}"
        ))),
    }
}

impl PluginManifest {
    /// Parses manifest bytes without validating the domain contract.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Syntax`] when the bytes are not valid YAML or
    /// do not match the manifest shape (VAL-001).
    pub fn parse(source: &str, path: &Path) -> Result<Self, ManifestError> {
        serde_yaml::from_str(source).map_err(|source| ManifestError::Syntax {
            path: path.to_path_buf(),
            source,
        })
    }

    /// Reads and parses a manifest from a plugin directory or manifest file.
    ///
    /// # Caller contract
    ///
    /// `source` may be either the directory containing `plugin.yaml` or the
    /// manifest file itself, because both are natural things for a user to
    /// pass to `crawl plugin install`.
    ///
    /// # Errors
    ///
    /// Returns [`ManifestError::Io`] when the file cannot be read and
    /// [`ManifestError::Syntax`] when it cannot be parsed.
    pub fn load(source: &Path) -> Result<(Self, PathBuf), ManifestError> {
        let manifest_path = if source.is_dir() {
            source.join(MANIFEST_FILE_NAME)
        } else {
            source.to_path_buf()
        };
        let text = std::fs::read_to_string(&manifest_path).map_err(|error| ManifestError::Io {
            path: manifest_path.clone(),
            source: error,
        })?;
        let manifest = Self::parse(&text, &manifest_path)?;
        Ok((manifest, manifest_path))
    }

    /// Validates the manifest and produces the [`Plugin`] aggregate.
    ///
    /// # Caller contract
    ///
    /// `root` is the directory containing the manifest. It becomes the default
    /// working directory of worker processes, so a manifest can refer to its
    /// entry point with a path relative to the plugin.
    ///
    /// # Errors
    ///
    /// Returns the first [`ManifestError`] encountered. Validation order is
    /// deliberate: identity, then API support, then runtime, then input, then
    /// schema, so the message names the earliest problem an author should fix.
    pub fn validate(self, root: &Path) -> Result<Plugin, ManifestError> {
        let name = PluginName::new(self.plugin.name)?;
        let version = PluginVersion::new(self.plugin.version)?;

        let api_version = ApiVersion::new(self.api_version)?;
        if !api_version.is_supported() {
            return Err(ManifestError::UnsupportedApiVersion {
                declared: api_version.as_str().to_owned(),
                supported: crate::SUPPORTED_API_VERSION,
            });
        }

        if self.runtime.command.is_empty() || self.runtime.command[0].trim().is_empty() {
            return Err(ManifestError::EmptyCommand);
        }

        if self.input.extensions.is_empty() {
            return Err(ManifestError::NoExtensions);
        }
        let mut extensions = BTreeSet::new();
        for declared in &self.input.extensions {
            extensions.insert(normalize_extension(declared)?);
        }

        let mut fields = Vec::with_capacity(self.output.schema.0.len());
        for (field_name, declaration) in self.output.schema.0 {
            let field_type = parse_field_type(&field_name, &declaration.field_type)?;
            fields.push(SchemaField {
                name: field_name,
                r#type: field_type,
                required: declaration.required,
            });
        }
        let schema = OutputSchema::new(fields)?;

        let execution = ExecutionDefaults {
            workers: match self.execution.workers {
                None => None,
                Some(WorkersDto::Auto(_)) => Some(WorkerCount::AUTO),
                Some(WorkersDto::Fixed(count)) => Some(WorkerCount::fixed(count)?),
            },
            timeout: self
                .execution
                .timeout_seconds
                .map(Timeout::from_seconds)
                .transpose()?,
            max_retries: self.execution.max_retries.map(RetryLimit::new),
            queue_capacity: self
                .execution
                .queue_capacity
                .map(QueueCapacity::new)
                .transpose()?,
        };

        Ok(Plugin {
            api_version,
            name,
            version,
            description: self.plugin.description,
            runtime: RuntimeSpec {
                runtime_type: self.runtime.runtime_type,
                command: self.runtime.command,
                working_directory: self.runtime.working_directory,
                environment: self
                    .runtime
                    .env
                    .into_iter()
                    .map(|entry| (entry.name, entry.value))
                    .collect(),
                selftest: self.runtime.selftest,
            },
            input: InputSpec::new(extensions, self.input.follow_symlinks),
            schema,
            extra_fields: self.output.extra_fields,
            execution,
            root: root.to_path_buf(),
        })
    }
}

/// Resolves a declared type name to a [`FieldType`].
///
/// Matched by hand rather than derived so the rejection can name both the
/// offending field and the supported alternatives. A derived `Deserialize`
/// would report only that a value was unrecognised, which is markedly less
/// useful to a plugin author reading the failure (NFR-050).
fn parse_field_type(field: &str, declared: &str) -> Result<FieldType, ManifestError> {
    match declared {
        "string" => Ok(FieldType::String),
        "integer" => Ok(FieldType::Integer),
        "number" => Ok(FieldType::Number),
        "boolean" => Ok(FieldType::Boolean),
        other => Err(ManifestError::UnsupportedFieldType {
            field: field.to_owned(),
            declared: other.to_owned(),
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const VALID: &str = r#"
api_version: "1"
plugin:
  name: entity-lines
  version: "1.0.0"
  description: Extract entities and source line numbers
runtime:
  type: python
  command:
    - python3
    - -m
    - entity_lines.worker
input:
  extensions:
    - ".txt"
    - ".md"
  follow_symlinks: false
output:
  format: records
  schema:
    filename:
      type: string
      required: true
    line:
      type: integer
      required: true
    entity:
      type: string
      required: true
execution:
  workers: auto
  timeout_seconds: 30
  max_retries: 1
"#;

    fn parse(source: &str) -> Result<Plugin, ManifestError> {
        PluginManifest::parse(source, Path::new("plugin.yaml"))?.validate(Path::new("/plugins/x"))
    }

    #[test]
    fn representative_manifest_validates() {
        let plugin = parse(VALID).unwrap();
        assert_eq!(plugin.name.as_str(), "entity-lines");
        assert_eq!(plugin.version.as_str(), "1.0.0");
        assert_eq!(plugin.schema.header(), vec!["filename", "line", "entity"]);
        assert_eq!(plugin.execution.workers, Some(WorkerCount::AUTO));
        assert_eq!(plugin.execution.max_retries, Some(RetryLimit::new(1)));
        assert_eq!(plugin.runtime.program(), "python3");
        assert_eq!(plugin.working_directory(), Path::new("/plugins/x"));
        assert!(!plugin.input.follow_symlinks);
        assert_eq!(plugin.extra_fields, ExtraFieldPolicy::Reject);
    }

    #[test]
    fn schema_order_is_declaration_order() {
        let plugin = parse(VALID).unwrap();
        let names: Vec<&str> = plugin
            .schema
            .fields()
            .iter()
            .map(|f| f.name.as_str())
            .collect();
        assert_eq!(names, vec!["filename", "line", "entity"]);
    }

    #[test]
    fn invalid_yaml_is_rejected() {
        let error = parse("api_version: \"1\"\n  bad indent:").unwrap_err();
        assert!(matches!(error, ManifestError::Syntax { .. }));
    }

    #[test]
    fn unknown_manifest_keys_are_rejected() {
        let source = VALID.replace("api_version:", "apiVersion:");
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::Syntax { .. }
        ));

        let with_extra = format!("{VALID}\nunexpected: true\n");
        assert!(matches!(
            parse(&with_extra).unwrap_err(),
            ManifestError::Syntax { .. }
        ));
    }

    #[test]
    fn unsupported_api_version_is_rejected() {
        let source = VALID.replace("api_version: \"1\"", "api_version: \"2\"");
        let error = parse(&source).unwrap_err();
        assert!(matches!(
            error,
            ManifestError::UnsupportedApiVersion { ref declared, .. } if declared == "2"
        ));
    }

    #[test]
    fn unquoted_scalar_versions_are_accepted() {
        let source = VALID
            .replace("api_version: \"1\"", "api_version: 1")
            .replace("version: \"1.0.0\"", "version: 1.0");
        let plugin = parse(&source).unwrap();
        assert_eq!(plugin.api_version.as_str(), "1");
        assert_eq!(plugin.version.as_str(), "1.0");
    }

    #[test]
    fn duplicate_schema_fields_are_rejected() {
        let source = VALID.replace(
            "    entity:\n      type: string\n      required: true",
            "    line:\n      type: string\n      required: true",
        );
        let error = parse(&source).unwrap_err();
        let message = error.to_string();
        assert!(
            message.contains("duplicate output schema field"),
            "{message}"
        );
    }

    #[test]
    fn unsupported_field_type_is_rejected() {
        let source = VALID.replace("type: integer", "type: datetime");
        let error = parse(&source).unwrap_err();
        assert!(matches!(
            error,
            ManifestError::UnsupportedFieldType { ref declared, .. } if declared == "datetime"
        ));
    }

    #[test]
    fn empty_command_is_rejected() {
        let source = VALID.replace(
            "  command:\n    - python3\n    - -m\n    - entity_lines.worker",
            "  command: []",
        );
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::EmptyCommand
        ));
    }

    #[test]
    fn unsupported_runtime_type_is_rejected() {
        let source = VALID.replace("type: python", "type: ruby");
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::Syntax { .. }
        ));
    }

    #[test]
    fn extensions_are_normalized_and_validated() {
        assert_eq!(normalize_extension(".TXT").unwrap(), ".txt");
        assert_eq!(normalize_extension("md").unwrap(), ".md");
        assert_eq!(normalize_extension("  .Log ").unwrap(), ".log");
        // Each rejection must cite the rule that actually caught it, so a
        // change to one rule cannot silently start catching another's cases.
        let reason = |value: &str| match normalize_extension(value) {
            Err(ManifestError::InvalidExtension { reason, .. }) => reason,
            other => panic!("{value:?} should be rejected, got {other:?}"),
        };
        assert_eq!(reason(""), "must not be empty");
        assert_eq!(reason("   "), "must not be empty");
        assert_eq!(reason("."), "must contain characters after the leading dot");
        assert_eq!(
            reason("tar.gz"),
            "must name a single extension without interior dots"
        );
        assert_eq!(
            reason("a b"),
            "must not contain whitespace or control characters"
        );
        assert_eq!(reason("a/b"), "must not contain path separators");
        assert_eq!(reason("a\\b"), "must not contain path separators");
    }

    #[test]
    fn empty_extension_list_is_rejected() {
        let source = VALID.replace(
            "  extensions:\n    - \".txt\"\n    - \".md\"",
            "  extensions: []",
        );
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::NoExtensions
        ));
    }

    #[test]
    fn invalid_worker_count_and_timeout_are_rejected() {
        let zero_workers = VALID.replace("workers: auto", "workers: 0");
        assert!(matches!(
            parse(&zero_workers).unwrap_err(),
            ManifestError::Policy(PolicyError::NotPositive { .. })
        ));

        let zero_timeout = VALID.replace("timeout_seconds: 30", "timeout_seconds: 0");
        assert!(matches!(
            parse(&zero_timeout).unwrap_err(),
            ManifestError::Policy(PolicyError::InvalidDuration { .. })
        ));
    }

    #[test]
    fn negative_retry_count_is_rejected() {
        let source = VALID.replace("max_retries: 1", "max_retries: -1");
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::Syntax { .. }
        ));
    }

    #[test]
    fn invalid_plugin_name_is_rejected() {
        let source = VALID.replace("name: entity-lines", "name: Entity Lines");
        assert!(matches!(
            parse(&source).unwrap_err(),
            ManifestError::Identity(_)
        ));
    }

    #[test]
    fn execution_block_is_optional() {
        let source = VALID.split("execution:").next().expect("prefix").to_owned();
        let plugin = parse(&source).unwrap();
        assert_eq!(plugin.execution, ExecutionDefaults::default());
    }

    #[test]
    fn extra_fields_policy_can_be_declared() {
        let source = VALID.replace(
            "  format: records",
            "  format: records\n  extra_fields: ignore",
        );
        assert_eq!(
            parse(&source).unwrap().extra_fields,
            ExtraFieldPolicy::Ignore
        );
    }
}
