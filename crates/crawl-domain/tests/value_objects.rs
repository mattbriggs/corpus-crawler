//! Round-trip, rendering, and conversion tests for the domain value objects.
//!
//! These cover the `Display`, `serde`, and conversion surfaces that the
//! in-module unit tests exercise only incidentally. They matter because those
//! surfaces are what logs, the registry file, and `plugin inspect` depend on.

use std::path::{Path, PathBuf};

use crawl_domain::crawl::{CrawlOutcome, CrawlState};
use crawl_domain::errors::{ErrorCategory, ErrorEvent, SchemaViolationEvent};
use crawl_domain::ids::{
    ApiVersion, CrawlId, PluginId, PluginName, PluginVersion, RequestId, WorkerId,
};
use crawl_domain::manifest::{ManifestError, PluginManifest, MANIFEST_FILE_NAME};
use crawl_domain::policy::{
    PartialOutputPolicy, QueueCapacity, RetryLimit, RetryPolicy, Timeout, WorkerCount,
};
use crawl_domain::record::{RawRecord, RawValue, ScalarValue};
use crawl_domain::schema::{ExtraFieldPolicy, FieldType};

// --- identities ----------------------------------------------------------

#[test]
fn identities_render_and_round_trip_through_serde() {
    let name = PluginName::new("entity-lines").expect("name");
    let json = serde_json::to_string(&name).expect("serialize");
    assert_eq!(json, "\"entity-lines\"");
    assert_eq!(
        serde_json::from_str::<PluginName>(&json).expect("deserialize"),
        name
    );
    assert_eq!(name.to_string(), "entity-lines");

    // An invalid name must not survive deserialization either: a hand-edited
    // or corrupted registry file is not a trusted source, and the validation
    // has to run on the way in, not only at construction.
    let error = serde_json::from_str::<PluginName>("\"Not Valid\"")
        .expect_err("an invalid name must not deserialize");
    assert!(
        error.to_string().contains("lowercase"),
        "deserialization must report the character-set rule: {error}"
    );

    let version = PluginVersion::new("1.0.0").expect("version");
    assert_eq!(version.to_string(), "1.0.0");
    assert_eq!(String::from(version.clone()), "1.0.0");

    let api = ApiVersion::new("1").expect("api version");
    assert_eq!(api.to_string(), "1");
    assert_eq!(String::from(api), "1");

    let id = PluginId::from(name.clone());
    assert_eq!(id.to_string(), "entity-lines");
    assert_eq!(id.name(), &name);
    assert_eq!(id.as_str(), "entity-lines");
    assert_eq!(String::from(name), "entity-lines");
}

#[test]
fn crawl_and_request_ids_render_stably() {
    let uuid = uuid::Uuid::nil();
    let crawl = CrawlId::from_uuid(uuid);
    assert_eq!(crawl.to_string(), "00000000-0000-0000-0000-000000000000");
    assert_eq!(CrawlId::generate().to_string().len(), 36);

    let request = RequestId::new(99);
    assert_eq!(request.to_string(), "99");
    assert_eq!(request.get(), 99);
}

#[test]
fn worker_ids_are_ordered_by_slot_then_generation() {
    let mut ids = [
        WorkerId::new(1).next_generation(),
        WorkerId::new(0),
        WorkerId::new(1),
    ];
    ids.sort();
    assert_eq!(
        ids.iter().map(WorkerId::to_string).collect::<Vec<_>>(),
        vec!["w0#1", "w1#1", "w1#2"]
    );
}

// --- policies ------------------------------------------------------------

#[test]
fn policy_values_render_for_operators() {
    assert_eq!(WorkerCount::AUTO.to_string(), "auto");
    assert_eq!(WorkerCount::fixed(6).expect("count").to_string(), "6");
    assert_eq!(QueueCapacity::new(32).expect("capacity").to_string(), "32");
    assert_eq!(
        Timeout::from_seconds(2.5).expect("timeout").to_string(),
        "2.5s"
    );
    assert_eq!(RetryLimit::new(3).to_string(), "3");
    assert_eq!(RetryLimit::new(3).max_attempts(), 4);
}

#[test]
fn worker_count_survives_serde_in_both_forms() {
    for value in [WorkerCount::AUTO, WorkerCount::fixed(4).expect("count")] {
        let json = serde_json::to_string(&value).expect("serialize");
        assert_eq!(
            serde_json::from_str::<WorkerCount>(&json).expect("deserialize"),
            value
        );
    }
    assert_eq!(
        serde_json::to_string(&WorkerCount::AUTO).unwrap(),
        "\"auto\""
    );
}

#[test]
fn partial_output_policies_are_distinct() {
    assert_ne!(
        PartialOutputPolicy::Promote,
        PartialOutputPolicy::RetainTemporary
    );
}

#[test]
fn retry_policy_exposes_its_limit() {
    let policy = RetryPolicy::new(RetryLimit::new(2));
    assert_eq!(policy.limit().max_retries(), 2);
    assert_eq!(RetryPolicy::default().limit().max_retries(), 0);
}

// --- records -------------------------------------------------------------

#[test]
fn raw_records_behave_as_ordered_maps() {
    let mut record = RawRecord::new();
    assert!(record.is_empty());
    assert_eq!(record.len(), 0);

    assert!(record.insert("b", RawValue::Integer(2)).is_none());
    assert!(record.insert("a", RawValue::Bool(true)).is_none());
    let replaced = record.insert("b", RawValue::Integer(3));
    assert_eq!(replaced, Some(RawValue::Integer(2)));

    assert_eq!(record.len(), 2);
    assert!(!record.is_empty());
    // Insertion order is preserved for diagnostics, not for CSV ordering.
    assert_eq!(record.field_names().collect::<Vec<_>>(), vec!["b", "a"]);
    assert_eq!(record.get("b"), Some(&RawValue::Integer(3)));
    assert_eq!(record.get("missing"), None);
    assert_eq!(RawRecord::default().len(), 0);
}

#[test]
fn every_raw_value_type_names_itself() {
    let cases: Vec<(RawValue, &str)> = vec![
        (RawValue::Null, "null"),
        (RawValue::Bool(true), "boolean"),
        (RawValue::Integer(1), "integer"),
        (RawValue::Number(1.5), "number"),
        (RawValue::String(String::new()), "string"),
        (RawValue::Array(vec![]), "array"),
        (RawValue::Object(Default::default()), "object"),
    ];
    for (value, expected) in cases {
        assert_eq!(value.type_name(), expected);
    }
}

#[test]
fn scalar_values_render_and_name_themselves() {
    let cases: Vec<(ScalarValue, &str, &str)> = vec![
        (ScalarValue::Null, "", "null"),
        (ScalarValue::Bool(false), "false", "boolean"),
        (ScalarValue::Integer(42), "42", "integer"),
        (ScalarValue::Number(2.25), "2.25", "number"),
        (ScalarValue::String("hi".into()), "hi", "string"),
    ];
    for (value, rendered, name) in cases {
        assert_eq!(value.to_csv_field(), rendered);
        assert_eq!(
            value.to_string(),
            rendered,
            "Display must match CSV rendering"
        );
        assert_eq!(value.type_name(), name);
    }
}

// --- schema --------------------------------------------------------------

#[test]
fn field_types_render_their_manifest_spelling() {
    for (field_type, spelling) in [
        (FieldType::String, "string"),
        (FieldType::Integer, "integer"),
        (FieldType::Number, "number"),
        (FieldType::Boolean, "boolean"),
    ] {
        assert_eq!(field_type.as_str(), spelling);
        assert_eq!(field_type.to_string(), spelling);
    }
}

#[test]
fn extra_field_policy_defaults_to_reject() {
    assert_eq!(ExtraFieldPolicy::default(), ExtraFieldPolicy::Reject);
}

// --- errors and events ---------------------------------------------------

#[test]
fn every_error_category_renders_its_stable_name() {
    for category in ErrorCategory::ALL {
        assert_eq!(category.to_string(), category.as_str());
        assert!(!category.as_str().is_empty());
    }
}

#[test]
fn error_events_serialize_without_unknown_attribution() {
    let event = ErrorEvent::new(ErrorCategory::Protocol, "bad line")
        .with_crawl(CrawlId::from_uuid(uuid::Uuid::nil()))
        .with_plugin(PluginId::new(PluginName::new("p").expect("name")))
        .with_worker(WorkerId::new(2))
        .with_code("boom");
    let json = serde_json::to_value(&event).expect("serialize");

    assert_eq!(json["category"], "protocol");
    assert_eq!(json["code"], "boom");
    // Absent attribution is omitted rather than rendered as null, so a log
    // consumer can tell "not applicable" from "unknown".
    assert!(json.get("file").is_none());
    assert_eq!(event.to_string(), "[protocol] bad line");
}

#[test]
fn schema_violation_events_carry_their_row_and_field() {
    let violation = SchemaViolationEvent::new("bad type", 3, Some("line".into()))
        .with_file(PathBuf::from("/a.txt"))
        .with_request(RequestId::new(8));
    assert_eq!(violation.error.category, ErrorCategory::Schema);
    assert_eq!(violation.row_index, 3);
    assert_eq!(violation.error.request, Some(RequestId::new(8)));
}

// --- state and outcomes --------------------------------------------------

#[test]
fn every_crawl_state_renders_a_stable_label() {
    for (state, label) in [
        (CrawlState::Created, "created"),
        (CrawlState::Validating, "validating"),
        (CrawlState::Running, "running"),
        (CrawlState::Cancelling, "cancelling"),
        (CrawlState::Stopping, "stopping"),
        (CrawlState::Finalizing, "finalizing"),
        (CrawlState::Completed, "completed"),
        (CrawlState::CompletedWithErrors, "completed_with_errors"),
        (CrawlState::Cancelled, "cancelled"),
        (CrawlState::Failed, "failed"),
    ] {
        assert_eq!(state.to_string(), label);
    }
}

#[test]
fn every_outcome_renders_a_stable_label() {
    for (outcome, label) in [
        (CrawlOutcome::Success, "success"),
        (CrawlOutcome::PartialSuccess, "partial_success"),
        (CrawlOutcome::ConfigurationFailure, "configuration_failure"),
        (CrawlOutcome::Cancelled, "cancelled"),
        (CrawlOutcome::PluginFailure, "plugin_failure"),
        (CrawlOutcome::HostFailure, "host_failure"),
    ] {
        assert_eq!(outcome.to_string(), label);
        assert_eq!(outcome.as_str(), label);
    }
}

// --- manifest loading ----------------------------------------------------

const MANIFEST: &str = r#"
api_version: "1"
plugin:
  name: loader
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "worker.py"]
  working_directory: src
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    a:
      type: string
      required: true
"#;

#[test]
fn a_manifest_loads_from_a_directory_or_from_the_file_itself() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join(MANIFEST_FILE_NAME), MANIFEST).expect("write");

    let (from_dir, path) = PluginManifest::load(root.path()).expect("load from directory");
    assert_eq!(path.file_name().expect("name"), MANIFEST_FILE_NAME);

    let (from_file, _) = PluginManifest::load(&path).expect("load from file");
    assert_eq!(from_dir.plugin.name, from_file.plugin.name);

    let plugin = from_dir.validate(root.path()).expect("validates");
    // A relative working directory resolves against the plugin root.
    assert_eq!(plugin.working_directory(), root.path().join("src"));
    assert_eq!(plugin.display_identity(), "loader@1.0.0");
    assert_eq!(plugin.id().as_str(), "loader");
}

#[test]
fn an_absolute_working_directory_is_used_verbatim() {
    let manifest = MANIFEST.replace("working_directory: src", "working_directory: /opt/plugin");
    let plugin = PluginManifest::parse(&manifest, Path::new("plugin.yaml"))
        .expect("parses")
        .validate(Path::new("/plugins/loader"))
        .expect("validates");
    assert_eq!(plugin.working_directory(), Path::new("/opt/plugin"));
}

#[test]
fn a_missing_working_directory_defaults_to_the_plugin_root() {
    let manifest = MANIFEST.replace("  working_directory: src\n", "");
    let plugin = PluginManifest::parse(&manifest, Path::new("plugin.yaml"))
        .expect("parses")
        .validate(Path::new("/plugins/loader"))
        .expect("validates");
    assert_eq!(plugin.working_directory(), Path::new("/plugins/loader"));
}

#[test]
fn a_missing_manifest_file_is_an_io_error_naming_the_path() {
    let root = tempfile::tempdir().expect("tempdir");
    let error = PluginManifest::load(root.path()).expect_err("no manifest present");
    assert!(matches!(error, ManifestError::Io { .. }));
    assert!(error.to_string().contains(MANIFEST_FILE_NAME));
}

#[test]
fn declared_environment_variables_are_carried_into_the_runtime_spec() {
    let manifest = MANIFEST.replace(
        "  working_directory: src",
        "  env:\n    - { name: MY_VAR, value: 7 }",
    );
    let plugin = PluginManifest::parse(&manifest, Path::new("plugin.yaml"))
        .expect("parses")
        .validate(Path::new("/plugins/loader"))
        .expect("validates");
    assert_eq!(
        plugin.runtime.environment,
        vec![("MY_VAR".to_owned(), "7".to_owned())]
    );
    assert_eq!(plugin.runtime.args(), ["worker.py"]);
}
