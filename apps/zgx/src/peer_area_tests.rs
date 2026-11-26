//! Settings pages on a paired server, from the window's side: each page asks
//! only a server that grants its area and says so otherwise
//! (ui/tests/plugins_devices.test.luau, peer_looks.test.luau and
//! providers_stability.test.luau on their fixtures).

fn run(filter: &str, count: usize) {
    let ui = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("ui");
    let report = zgx_script::test_runner::run(&ui, Some(filter), |outcome| {
        if let Some(failure) = &outcome.failure {
            eprintln!("{}: {failure}", outcome.name);
        }
    })
    .expect("the page tests load");
    assert_eq!(report.outcomes.len(), count, "{report:?}");
    assert_eq!(report.failed(), 0, "{report:?}");
}

#[test]
fn a_server_without_the_settings_grant_is_said_not_asked() {
    run("a paired server that doesn't grant settings says so, with the command, and is asked nothing", 1);
}

#[test]
fn a_server_without_the_logins_grant_is_said_on_providers() {
    run("providers: a server that doesn't grant logins", 1);
}

/// A grant taken back while Providers is open says so; native subagents and the
/// update policy follow `settings` and `updates`.
#[test]
fn providers_follow_the_grants_live_and_per_area() {
    run("providers: a grant taken back", 1);
    run("providers: an agent's settings on a server", 1);
}

/// About & updates on a paired server: its update, asked of it where it grants
/// `updates`; the notice where it doesn't.
#[test]
fn about_updates_follow_the_device_chosen() {
    run("about:", 2);
}

#[test]
fn update_all_names_a_server_that_doesnt_grant_updates() {
    run("Update all names a paired server", 1);
}

/// Themes and Templates on a server: its list, use and delete (themes), no
/// install, and the same not-granted notice.
#[test]
fn a_servers_themes_and_templates_are_listed_used_and_deleted_there() {
    run("themes: a server that grants settings", 1);
    run("templates: a server's are listed", 1);
    run("a server that doesn't grant settings says so, with the command, and is asked nothing", 1);
}
