//! Settings → Plugins on a paired server that doesn't grant `settings`: the
//! page says so (ui/tests/plugins_devices.test.luau, on
//! ui/plugins_devices_fixture.luau) instead of asking it.

#[test]
fn a_server_without_the_settings_grant_is_said_not_asked() {
    let ui = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui");
    let name = "a paired server that doesn't grant settings";
    let report = zgx_script::test_runner::run(&ui, Some(name), |outcome| {
        if let Some(failure) = &outcome.failure {
            eprintln!("{}: {failure}", outcome.name);
        }
    })
    .expect("the plugin page tests load");
    assert_eq!(report.outcomes.len(), 1, "{report:?}");
    assert_eq!(report.failed(), 0, "{report:?}");
}
