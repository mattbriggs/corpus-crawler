//! Infrastructure adapter tests against the real filesystem.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use crawl_application::ports::registry::{PluginRegistry, RegistryError};
use crawl_application::ports::report::ReportWriter;
use crawl_application::ports::walker::{DirectoryWalker, Discovery};
use crawl_domain::manifest::PluginManifest;
use crawl_domain::plugin::{InputSpec, Plugin};
use crawl_domain::policy::PartialOutputPolicy;
use crawl_domain::record::{ScalarValue, ValidatedRecord};
use crawl_domain::schema::{FieldType, OutputSchema, SchemaField};
use crawl_infrastructure::reports::csv_writer::temporary_path;
use crawl_infrastructure::{CsvReportWriter, FileRegistry, FilesystemWalker};
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;

fn plugin(name: &str) -> Plugin {
    let manifest = format!(
        r#"
api_version: "1"
plugin:
  name: {name}
  version: "1.0.0"
runtime:
  type: python
  command: ["python3", "w.py"]
input:
  extensions: [".txt"]
output:
  format: records
  schema:
    a:
      type: string
      required: true
"#
    );
    PluginManifest::parse(&manifest, Path::new("plugin.yaml"))
        .expect("parses")
        .validate(Path::new("/plugins/x"))
        .expect("validates")
}

fn schema() -> OutputSchema {
    OutputSchema::new(vec![
        SchemaField {
            name: "text".into(),
            r#type: FieldType::String,
            required: true,
        },
        SchemaField {
            name: "count".into(),
            r#type: FieldType::Integer,
            required: false,
        },
    ])
    .expect("schema")
}

// --- registry ------------------------------------------------------------

#[test]
fn registry_round_trips_a_plugin() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = FileRegistry::at(home.path().join("registry.json"));
    let plugin = plugin("alpha");

    assert!(registry.list().expect("list").is_empty());
    registry.install(&plugin, false).expect("install");

    let found = registry.find(&plugin.id()).expect("find").expect("present");
    assert_eq!(found.name, plugin.name);
    assert_eq!(found.schema.header(), vec!["a"]);
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn a_missing_registry_file_reads_as_empty_not_as_an_error() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = FileRegistry::at(home.path().join("nested/deeper/registry.json"));
    assert!(registry.list().expect("list").is_empty());
}

#[test]
fn reinstalling_requires_force() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = FileRegistry::at(home.path().join("registry.json"));
    let plugin = plugin("alpha");

    registry.install(&plugin, false).expect("first install");
    let error = registry
        .install(&plugin, false)
        .expect_err("second install must be refused");
    assert!(matches!(error, RegistryError::AlreadyInstalled(_)));

    registry
        .install(&plugin, true)
        .expect("forced install replaces");
    assert_eq!(registry.list().expect("list").len(), 1);
}

#[test]
fn removing_an_unregistered_plugin_is_an_error() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = FileRegistry::at(home.path().join("registry.json"));
    let plugin = plugin("alpha");

    let error = registry
        .remove(&plugin.id())
        .expect_err("nothing to remove");
    assert!(matches!(error, RegistryError::NotFound(_)));

    registry.install(&plugin, false).expect("install");
    registry.remove(&plugin.id()).expect("remove");
    assert!(registry.find(&plugin.id()).expect("find").is_none());
}

#[test]
fn a_corrupt_registry_is_reported_as_corrupt() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("registry.json");
    std::fs::write(&path, "{ this is not json").expect("seed");
    let registry = FileRegistry::at(&path);
    assert!(matches!(
        registry.list().expect_err("must fail"),
        RegistryError::Corrupt(_)
    ));
}

#[test]
fn plugins_are_listed_in_a_stable_order() {
    let home = tempfile::tempdir().expect("tempdir");
    let registry = FileRegistry::at(home.path().join("registry.json"));
    for name in ["zulu", "alpha", "mike"] {
        registry.install(&plugin(name), false).expect("install");
    }
    let names: Vec<String> = registry
        .list()
        .expect("list")
        .iter()
        .map(|plugin| plugin.name.to_string())
        .collect();
    assert_eq!(names, vec!["alpha", "mike", "zulu"]);
}

// --- CSV writer ----------------------------------------------------------

#[test]
fn the_header_is_written_at_open_time() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");

    // Before finalization the report lives at the temporary path only.
    assert!(!destination.exists());
    assert!(writer.temporary_path().exists());

    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");
    assert_eq!(
        std::fs::read_to_string(&destination).expect("report"),
        "text,count\n"
    );
}

#[test]
fn values_are_escaped_according_to_csv_rules() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");

    for text in ["plain", "has,comma", "has\"quote", "has\nnewline"] {
        writer
            .write(&ValidatedRecord::from_ordered_values(vec![
                ScalarValue::String(text.to_owned()),
                ScalarValue::Integer(1),
            ]))
            .expect("write");
    }
    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");

    // Re-parsing is the real assertion: the values must survive a round trip.
    let text = std::fs::read_to_string(&destination).expect("report");
    let mut reader = csv::Reader::from_reader(text.as_bytes());
    let values: Vec<String> = reader
        .records()
        .map(|record| record.expect("row")[0].to_owned())
        .collect();
    assert_eq!(
        values,
        vec!["plain", "has,comma", "has\"quote", "has\nnewline"]
    );
}

#[test]
fn null_optional_values_render_as_empty_fields() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");
    writer
        .write(&ValidatedRecord::from_ordered_values(vec![
            ScalarValue::String("a".into()),
            ScalarValue::Null,
        ]))
        .expect("write");
    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");
    assert_eq!(
        std::fs::read_to_string(&destination).expect("report"),
        "text,count\na,\n"
    );
}

#[test]
fn retaining_the_temporary_file_leaves_the_destination_absent() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");
    writer
        .write(&ValidatedRecord::from_ordered_values(vec![
            ScalarValue::String("a".into()),
            ScalarValue::Null,
        ]))
        .expect("write");
    writer
        .finalize(PartialOutputPolicy::RetainTemporary)
        .expect("finalize");

    assert!(
        !destination.exists(),
        "an unvouched-for report was published"
    );
    assert!(
        temporary_path(&destination).exists(),
        "the partial file was lost"
    );
}

#[test]
fn writing_after_finalization_is_refused() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");
    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");
    assert!(writer
        .write(&ValidatedRecord::from_ordered_values(vec![
            ScalarValue::String("a".into()),
            ScalarValue::Null,
        ]))
        .is_err());
}

#[test]
fn missing_parent_directories_are_created() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("a/b/c/report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");
    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");
    assert!(destination.exists());
}

#[test]
fn row_counts_are_tracked() {
    let home = tempfile::tempdir().expect("tempdir");
    let mut writer =
        CsvReportWriter::create(&home.path().join("report.csv"), &schema()).expect("create");
    assert_eq!(writer.rows_written(), 0);
    writer
        .write(&ValidatedRecord::from_ordered_values(vec![
            ScalarValue::String("a".into()),
            ScalarValue::Null,
        ]))
        .expect("write");
    assert_eq!(writer.rows_written(), 1);
}

// --- walker --------------------------------------------------------------

/// Collects every discovery the walker emits for a tree.
async fn walk(root: &Path, follow_symlinks: bool) -> Vec<Discovery> {
    let (tx, mut rx) = mpsc::channel(256);
    let walker = Arc::new(FilesystemWalker);
    let input = InputSpec::new(std::collections::BTreeSet::new(), follow_symlinks);
    let handle = {
        let walker = Arc::clone(&walker);
        let root = root.to_path_buf();
        tokio::spawn(async move {
            walker.walk(root, input, tx, CancellationToken::new()).await;
        })
    };
    let mut discoveries = Vec::new();
    while let Some(discovery) = rx.recv().await {
        discoveries.push(discovery);
    }
    handle.await.expect("walk finished");
    discoveries
}

fn files_of(discoveries: &[Discovery]) -> Vec<PathBuf> {
    let mut paths: Vec<PathBuf> = discoveries
        .iter()
        .filter_map(|discovery| match discovery {
            Discovery::File(path) => Some(path.clone()),
            Discovery::Error(_) => None,
        })
        .collect();
    paths.sort();
    paths
}

#[tokio::test]
async fn traversal_is_recursive_and_reports_files_only() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::create_dir_all(root.path().join("a/b")).expect("dirs");
    std::fs::write(root.path().join("top.txt"), "x").expect("file");
    std::fs::write(root.path().join("a/mid.txt"), "x").expect("file");
    std::fs::write(root.path().join("a/b/deep.rs"), "x").expect("file");

    let files = files_of(&walk(root.path(), false).await);
    assert_eq!(files.len(), 3, "directories must not be reported as files");
    assert!(files.iter().all(|path| path.is_file()));
}

#[tokio::test]
async fn hidden_files_are_discovered() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join(".hidden.txt"), "x").expect("file");
    // A processing runtime must see the whole tree, not git's view of it.
    assert_eq!(files_of(&walk(root.path(), false).await).len(), 1);
}

#[tokio::test]
async fn gitignored_files_are_still_discovered() {
    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join(".gitignore"), "ignored.txt\n").expect("gitignore");
    std::fs::write(root.path().join("ignored.txt"), "x").expect("file");
    let files = files_of(&walk(root.path(), false).await);
    assert_eq!(files.len(), 2, "VCS filters must not hide input files");
}

#[cfg(unix)]
#[tokio::test]
async fn symlinks_are_not_followed_by_default() {
    let root = tempfile::tempdir().expect("tempdir");
    let outside = tempfile::tempdir().expect("tempdir");
    std::fs::write(outside.path().join("target.txt"), "x").expect("file");
    std::os::unix::fs::symlink(outside.path(), root.path().join("link")).expect("symlink");

    let not_followed = files_of(&walk(root.path(), false).await);
    let followed = files_of(&walk(root.path(), true).await);

    let names = |paths: &[PathBuf]| -> Vec<String> {
        paths
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .map(|name| name.to_string_lossy().into_owned())
            })
            .collect()
    };

    // The default policy neither descends into the linked directory nor
    // dispatches the link itself: a symlink is not a regular file.
    assert!(
        !names(&not_followed).contains(&"target.txt".to_owned()),
        "traversal descended a symlink despite follow_symlinks = false"
    );
    assert!(not_followed.is_empty(), "the symlink itself was dispatched");

    // Following resolves the link, so the target's contents are discovered.
    assert!(names(&followed).contains(&"target.txt".to_owned()));
}

#[cfg(unix)]
#[tokio::test]
async fn non_regular_files_are_never_dispatched() {
    use std::os::unix::net::UnixListener;

    let root = tempfile::tempdir().expect("tempdir");
    std::fs::write(root.path().join("real.txt"), "x").expect("file");
    // A socket is not something a plugin could open as a file.
    let _listener = UnixListener::bind(root.path().join("socket")).expect("socket");

    let files = files_of(&walk(root.path(), false).await);
    assert_eq!(files.len(), 1);
    assert!(files[0].ends_with("real.txt"));
}

#[tokio::test]
async fn an_empty_tree_yields_no_files() {
    let root = tempfile::tempdir().expect("tempdir");
    assert!(files_of(&walk(root.path(), false).await).is_empty());
}

// --- adapter failure paths -----------------------------------------------

#[test]
fn the_registry_location_follows_a_documented_resolution_order() {
    // These assertions mutate process environment, so they share one test
    // rather than racing each other across parallel test threads.
    let home = tempfile::tempdir().expect("tempdir");
    let previous = (
        std::env::var_os("CRAWL_HOME"),
        std::env::var_os("XDG_DATA_HOME"),
    );

    std::env::set_var("CRAWL_HOME", home.path());
    assert_eq!(
        FileRegistry::default_location().path(),
        home.path().join("registry.json"),
        "CRAWL_HOME must win"
    );

    std::env::remove_var("CRAWL_HOME");
    std::env::set_var("XDG_DATA_HOME", home.path());
    assert_eq!(
        FileRegistry::default_location().path(),
        home.path().join("crawl/registry.json"),
        "XDG_DATA_HOME is the next fallback"
    );

    match previous.0 {
        Some(value) => std::env::set_var("CRAWL_HOME", value),
        None => std::env::remove_var("CRAWL_HOME"),
    }
    match previous.1 {
        Some(value) => std::env::set_var("XDG_DATA_HOME", value),
        None => std::env::remove_var("XDG_DATA_HOME"),
    }
}

#[test]
fn an_unwritable_registry_location_is_reported_as_unavailable() {
    let home = tempfile::tempdir().expect("tempdir");
    // A regular file where a directory must go makes the write fail.
    let blocker = home.path().join("blocked");
    std::fs::write(&blocker, "not a directory").expect("seed");

    let registry = FileRegistry::at(blocker.join("registry.json"));
    let error = registry
        .install(&plugin("alpha"), false)
        .expect_err("install must fail");
    assert!(matches!(error, RegistryError::Unavailable(_)));
}

#[test]
fn registry_writes_replace_the_previous_document_atomically() {
    let home = tempfile::tempdir().expect("tempdir");
    let path = home.path().join("registry.json");
    let registry = FileRegistry::at(&path);

    registry.install(&plugin("alpha"), false).expect("install");
    registry.install(&plugin("beta"), false).expect("install");

    // No temporary artefact is left behind beside the registry.
    let leftovers: Vec<PathBuf> = std::fs::read_dir(home.path())
        .expect("read dir")
        .filter_map(|entry| entry.ok().map(|entry| entry.path()))
        .filter(|entry| entry.extension().is_some_and(|ext| ext == "tmp"))
        .collect();
    assert!(leftovers.is_empty(), "left a temporary file: {leftovers:?}");
    assert_eq!(registry.list().expect("list").len(), 2);
}

#[test]
fn an_unwritable_report_destination_fails_at_creation() {
    let home = tempfile::tempdir().expect("tempdir");
    let blocker = home.path().join("blocked");
    std::fs::write(&blocker, "not a directory").expect("seed");

    // Preflight must catch this before any file is dispatched (REQ-032), and
    // the message must name the path so the operator can fix it (NFR-050).
    let error = CsvReportWriter::create(&blocker.join("report.csv"), &schema())
        .err()
        .expect("an unwritable destination must be rejected");
    assert!(
        error.to_string().contains("blocked"),
        "the report error did not name the offending path: {error}"
    );
}

#[test]
fn the_temporary_path_is_a_sibling_of_the_destination() {
    assert_eq!(
        temporary_path(Path::new("/reports/out.csv")),
        PathBuf::from("/reports/out.csv.partial")
    );
    // A path with no file name still yields a usable temporary name.
    assert!(temporary_path(Path::new("/"))
        .to_string_lossy()
        .ends_with(".partial"));
}

#[test]
fn finalizing_twice_is_refused_rather_than_silently_reordering_output() {
    let home = tempfile::tempdir().expect("tempdir");
    let destination = home.path().join("report.csv");
    let mut writer = CsvReportWriter::create(&destination, &schema()).expect("create");
    writer
        .finalize(PartialOutputPolicy::Promote)
        .expect("finalize");
    // The temporary file is gone, so a second promotion cannot succeed.
    assert!(writer.finalize(PartialOutputPolicy::Promote).is_err());
}
