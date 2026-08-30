//! Streaming CSV report writer (REQ-100 - REQ-107, REQ-180 - REQ-183).
//!
//! # Temp-then-promote
//!
//! Rows stream into a sibling temporary file and the file is renamed onto the
//! requested path only at finalization. That is what makes OQ-017 answerable:
//! an orderly end promotes the partial report, a host-fatal end leaves the
//! temporary file untouched and names it in the error.

use std::fs::{File, OpenOptions};
use std::io::BufWriter;
use std::path::{Path, PathBuf};

use crawl_application::ports::report::{ReportError, ReportWriter};
use crawl_domain::policy::PartialOutputPolicy;
use crawl_domain::record::ValidatedRecord;
use crawl_domain::schema::OutputSchema;

/// Writes one CSV report for a crawl.
pub struct CsvReportWriter {
    writer: Option<csv::Writer<BufWriter<File>>>,
    temporary: PathBuf,
    destination: PathBuf,
    rows_written: u64,
}

impl CsvReportWriter {
    /// Creates the report and writes its header immediately.
    ///
    /// Writing the header at open time is what makes a zero-record crawl
    /// produce a header-only CSV (OQ-021, REQ-104) and proves the destination
    /// is writable during preflight rather than after processing (REQ-032).
    ///
    /// # Errors
    /// Returns [`ReportError`] when the destination cannot be created.
    pub fn create(destination: &Path, schema: &OutputSchema) -> Result<Self, ReportError> {
        if let Some(parent) = destination.parent() {
            if !parent.as_os_str().is_empty() {
                std::fs::create_dir_all(parent)
                    .map_err(|error| ReportError(format!("{}: {error}", parent.display())))?;
            }
        }
        let temporary = temporary_path(destination);
        let file = OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .open(&temporary)
            .map_err(|error| ReportError(format!("{}: {error}", temporary.display())))?;

        let mut writer = csv::Writer::from_writer(BufWriter::new(file));
        writer
            .write_record(schema.header())
            .map_err(|error| ReportError(error.to_string()))?;
        writer
            .flush()
            .map_err(|error| ReportError(error.to_string()))?;

        Ok(Self {
            writer: Some(writer),
            temporary,
            destination: destination.to_path_buf(),
            rows_written: 0,
        })
    }

    /// Returns the temporary path rows are streamed into.
    #[must_use]
    pub fn temporary_path(&self) -> &Path {
        &self.temporary
    }

    /// Returns the number of data rows written so far.
    #[must_use]
    pub const fn rows_written(&self) -> u64 {
        self.rows_written
    }
}

/// Returns the sibling temporary path used while a report is being written.
#[must_use]
pub fn temporary_path(destination: &Path) -> PathBuf {
    let mut name = destination
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_else(|| "report.csv".to_owned());
    name.push_str(".partial");
    destination.with_file_name(name)
}

impl ReportWriter for CsvReportWriter {
    fn write(&mut self, record: &ValidatedRecord) -> Result<(), ReportError> {
        let Some(writer) = self.writer.as_mut() else {
            return Err(ReportError("report is already finalized".to_owned()));
        };
        writer
            .write_record(record.to_csv_fields())
            .map_err(|error| ReportError(error.to_string()))?;
        self.rows_written += 1;
        Ok(())
    }

    fn finalize(&mut self, policy: PartialOutputPolicy) -> Result<(), ReportError> {
        if let Some(mut writer) = self.writer.take() {
            writer
                .flush()
                .map_err(|error| ReportError(error.to_string()))?;
            drop(writer);
        }
        match policy {
            PartialOutputPolicy::Promote => std::fs::rename(&self.temporary, &self.destination)
                .map_err(|error| {
                    ReportError(format!(
                        "cannot promote {} to {}: {error}",
                        self.temporary.display(),
                        self.destination.display()
                    ))
                }),
            PartialOutputPolicy::RetainTemporary => Ok(()),
        }
    }
}
