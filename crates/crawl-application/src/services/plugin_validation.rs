//! Plugin validation use case (REQ-012 - REQ-019).

use crawl_domain::ids::WorkerId;
use crawl_domain::plugin::Plugin;
use crawl_protocol::{Request, RequestOp, Response};

use crate::ports::worker::{WorkerFactory, WorkerStartupError};

/// Outcome of validating one installed or candidate plugin.
#[derive(Debug, Clone, PartialEq)]
pub struct ValidationReport {
    /// Manifest parsed and satisfied the domain contract (REQ-012, REQ-017).
    pub manifest_valid: bool,
    /// The runtime command started and completed the handshake (REQ-014).
    pub runtime_started: bool,
    /// The plugin module and callable loaded (REQ-015, REQ-016).
    ///
    /// A successful handshake is the evidence: the SDK performs the import and
    /// callable check before replying `ready`.
    pub entrypoint_loaded: bool,
    /// Result of the optional self-test (REQ-019), `None` when not declared.
    pub selftest: Option<Result<String, String>>,
}

/// Validates a plugin by actually starting its worker.
///
/// # Errors
/// Returns [`WorkerStartupError`] when the runtime cannot start or the plugin
/// cannot be loaded; these are the failures that must block registration.
pub async fn validate_plugin(
    plugin: &Plugin,
    factory: &dyn WorkerFactory,
) -> Result<ValidationReport, WorkerStartupError> {
    let mut worker = factory.spawn(plugin, WorkerId::new(0)).await?;

    let selftest = if plugin.runtime.selftest {
        let request = Request::control(crawl_domain::ids::RequestId::new(1), RequestOp::Selftest);
        match worker.dispatch(request, None).await {
            Ok(Response::Ok { .. }) => Some(Ok("self-test passed".to_owned())),
            Ok(Response::Error { error, .. }) => {
                Some(Err(format!("{}: {}", error.code, error.message)))
            }
            Ok(other) => Some(Err(format!(
                "self-test returned an unexpected {} message",
                other.status()
            ))),
            Err(failure) => Some(Err(failure.to_string())),
        }
    } else {
        None
    };

    worker.shutdown().await;

    Ok(ValidationReport {
        manifest_valid: true,
        runtime_started: true,
        entrypoint_loaded: true,
        selftest,
    })
}
