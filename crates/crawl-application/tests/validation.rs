//! Plugin validation use-case tests.

mod support;

use crawl_application::services::plugin_validation::validate_plugin;

use support::{FailingFactory, HandshakeOnlyFactory};

#[tokio::test]
async fn a_plugin_that_starts_and_handshakes_validates() {
    let plugin = support::test_plugin();
    let report = validate_plugin(
        &plugin,
        &HandshakeOnlyFactory {
            selftest_fails: false,
        },
    )
    .await
    .expect("validation succeeds");
    assert!(report.manifest_valid);
    assert!(report.runtime_started);
    assert!(report.entrypoint_loaded);
    // The fixture manifest does not declare a self-test.
    assert!(report.selftest.is_none());
}

#[tokio::test]
async fn a_runtime_that_cannot_start_fails_validation() {
    let plugin = support::test_plugin();
    let error = validate_plugin(&plugin, &FailingFactory)
        .await
        .expect_err("validation must fail");
    assert!(error.to_string().contains("cannot start plugin runtime"));
}

#[tokio::test]
async fn a_declared_self_test_is_executed_and_its_failure_reported() {
    let mut plugin = support::test_plugin();
    plugin.runtime.selftest = true;

    let passed = validate_plugin(
        &plugin,
        &HandshakeOnlyFactory {
            selftest_fails: false,
        },
    )
    .await
    .expect("validation runs");
    assert!(matches!(passed.selftest, Some(Ok(_))));

    let failed = validate_plugin(
        &plugin,
        &HandshakeOnlyFactory {
            selftest_fails: true,
        },
    )
    .await
    .expect("validation runs");
    // A failing self-test is reported, not raised: the caller decides.
    let Some(Err(message)) = failed.selftest else {
        panic!("expected a reported self-test failure");
    };
    assert!(message.contains("selftest_failed"));
}
