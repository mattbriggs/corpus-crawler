//! Logging adapter tests.
//!
//! The sink has one match arm per event type, and these tests assert on what it
//! actually writes. A formatting mistake in a rarely reached arm would
//! otherwise surface for the first time during the incident the log was meant
//! to explain.
//!
//! The invariants of `CrawlEvent` itself - name uniqueness, level
//! classification, error exposure - belong to `crawl-domain` and are tested
//! there. These tests cover only the rendering.

use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use crawl_application::ports::EventSink;
use crawl_domain::crawl::CrawlOutcome;
use crawl_domain::errors::{ErrorCategory, ErrorEvent, SchemaViolationEvent};
use crawl_domain::events::CrawlEvent;
use crawl_domain::ids::{CrawlId, PluginId, PluginName, PluginVersion, RequestId, WorkerId};
use crawl_domain::statistics::CrawlStatistics;
use crawl_infrastructure::TracingEventSink;

fn every_event() -> Vec<CrawlEvent> {
    let crawl = CrawlId::generate();
    let worker = WorkerId::new(0);
    let path = PathBuf::from("/data/a.txt");
    let error = || Box::new(ErrorEvent::new(ErrorCategory::Timeout, "deadline").with_file(&path));

    vec![
        CrawlEvent::CrawlStarted {
            crawl,
            plugin: PluginId::new(PluginName::new("p").expect("name")),
            plugin_version: PluginVersion::new("1.0.0").expect("version"),
            input: PathBuf::from("/in"),
            output: PathBuf::from("/out.csv"),
            workers: 4,
            queue_capacity: 1024,
            timeout_seconds: Some(30.0),
            max_retries: 1,
        },
        CrawlEvent::CrawlCompleted {
            crawl,
            outcome: CrawlOutcome::Success,
            statistics: Box::new(CrawlStatistics::new()),
        },
        CrawlEvent::CrawlCancelled {
            crawl,
            reason: "operator".into(),
        },
        CrawlEvent::FileDiscovered { path: path.clone() },
        CrawlEvent::FileMatched { path: path.clone() },
        CrawlEvent::FileSkipped { path: path.clone() },
        CrawlEvent::FileProcessingStarted {
            path: path.clone(),
            request: RequestId::new(1),
            worker,
            attempt: 1,
        },
        CrawlEvent::FileProcessingCompleted {
            path: path.clone(),
            request: RequestId::new(1),
            rows_accepted: 2,
            rows_rejected: 0,
            duration_seconds: 0.01,
        },
        CrawlEvent::FileProcessingFailed {
            error: error(),
            attempts: 2,
        },
        CrawlEvent::FileProcessingRetried {
            path: path.clone(),
            error: error(),
            attempt: 1,
        },
        CrawlEvent::RecordAccepted { path: path.clone() },
        CrawlEvent::RecordRejected {
            violation: Box::new(SchemaViolationEvent::new("bad", 0, Some("line".into()))),
        },
        CrawlEvent::WorkerStarted {
            worker,
            pid: Some(1234),
        },
        CrawlEvent::WorkerCrashed {
            worker,
            detail: "exit 3".into(),
        },
        CrawlEvent::WorkerTimedOut {
            worker,
            path: path.clone(),
            timeout_seconds: 30.0,
        },
        CrawlEvent::WorkerRestarted {
            worker,
            reason: "crash".into(),
        },
        CrawlEvent::WorkerStopped { worker },
        CrawlEvent::WorkerDiagnostic {
            worker,
            line: "a warning".into(),
        },
        CrawlEvent::ProtocolViolationDetected { error: error() },
        CrawlEvent::TraversalErrorDetected { error: error() },
    ]
}

/// Captures everything a subscriber writes during `body`, as JSON lines.
///
/// Asserting on the emitted text is the only way to catch a formatting mistake
/// in a rarely reached arm, which would otherwise surface for the first time
/// during the incident the log was meant to explain.
fn capture(body: impl FnOnce()) -> Vec<serde_json::Value> {
    #[derive(Clone, Default)]
    struct Buffer(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for Buffer {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("buffer").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buffer {
        type Writer = Self;

        fn make_writer(&'a self) -> Self::Writer {
            self.clone()
        }
    }

    let buffer = Buffer::default();
    let subscriber = tracing_subscriber::fmt()
        .json()
        .with_max_level(tracing::Level::TRACE)
        .with_writer(buffer.clone())
        .finish();

    tracing::subscriber::with_default(subscriber, body);

    let bytes = buffer.0.lock().expect("buffer").clone();
    String::from_utf8(bytes)
        .expect("valid UTF-8")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .map(|line| serde_json::from_str(line).expect("each log line is JSON"))
        .collect()
}

#[test]
fn every_event_variant_is_emitted_under_its_own_name() {
    let events = every_event();
    let expected: Vec<&str> = events.iter().map(CrawlEvent::name).collect();

    let captured = capture(|| {
        let sink = TracingEventSink;
        for event in &events {
            sink.emit(event);
        }
    });

    assert_eq!(
        captured.len(),
        events.len(),
        "the sink dropped or merged events"
    );
    let emitted: Vec<&str> = captured
        .iter()
        .map(|line| {
            line["fields"]["event"]
                .as_str()
                .unwrap_or_else(|| panic!("a log line carried no event name: {line}"))
        })
        .collect();
    assert_eq!(
        emitted, expected,
        "events were emitted under the wrong names, or out of order"
    );
}

#[test]
fn failure_events_carry_the_fields_an_operator_needs() {
    // During an incident these three fields are what turn a log line into a
    // lead: what kind of failure, which file, and what the plugin said.
    let path = PathBuf::from("/data/a.txt");
    let error = Box::new(
        ErrorEvent::new(ErrorCategory::PluginProcessing, "parser gave up")
            .with_file(&path)
            .with_code("bad_syntax"),
    );

    let captured = capture(|| {
        TracingEventSink.emit(&CrawlEvent::FileProcessingFailed { error, attempts: 2 });
    });

    let fields = &captured[0]["fields"];
    assert_eq!(fields["event"], "file_processing_failed");
    assert_eq!(fields["category"], "plugin_processing");
    assert_eq!(fields["file"], "/data/a.txt");
    assert_eq!(fields["message"], "parser gave up");
    assert_eq!(fields["code"], "bad_syntax");
    assert_eq!(fields["attempts"], 2);
}

#[test]
fn rejected_records_report_the_offending_row_and_field() {
    let captured = capture(|| {
        TracingEventSink.emit(&CrawlEvent::RecordRejected {
            violation: Box::new(
                SchemaViolationEvent::new("expected integer", 4, Some("line".into()))
                    .with_file(PathBuf::from("/data/a.txt")),
            ),
        });
    });

    let fields = &captured[0]["fields"];
    assert_eq!(fields["event"], "record_rejected");
    assert_eq!(fields["row"], 4);
    assert_eq!(fields["field"], "line");
    assert_eq!(fields["file"], "/data/a.txt");
}

#[test]
fn per_file_events_are_emitted_below_the_default_level() {
    // A million-file crawl must stay readable by default, so per-file events
    // have to sit under INFO rather than merely being short.
    let captured = capture(|| {
        TracingEventSink.emit(&CrawlEvent::FileDiscovered {
            path: PathBuf::from("/data/a.txt"),
        });
        TracingEventSink.emit(&CrawlEvent::CrawlCancelled {
            crawl: CrawlId::generate(),
            reason: "operator".into(),
        });
    });

    assert_eq!(
        captured[0]["level"], "TRACE",
        "file_discovered must be TRACE"
    );
    assert_eq!(captured[1]["level"], "WARN", "crawl_cancelled must be WARN");
}

#[test]
fn failure_events_expose_their_error_for_attribution() {
    let with_errors: Vec<&'static str> = vec![
        "file_processing_failed",
        "file_processing_retried",
        "record_rejected",
        "protocol_violation_detected",
        "traversal_error_detected",
    ];
    for event in every_event() {
        let has_error = event.error().is_some();
        assert_eq!(
            has_error,
            with_errors.contains(&event.name()),
            "{} error exposure is wrong",
            event.name()
        );
    }
}
