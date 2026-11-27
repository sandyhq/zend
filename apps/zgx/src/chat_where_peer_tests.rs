//! A new chat on a paired server, from the composer's side (step 6g):
//! under `full` it's worked on as this computer is (a new worktree made
//! there), and a server's refusal to run it is said in words with the
//! command to run on the server (ui/tests/chat_where.test.luau on
//! ui/chat_fixture.luau).

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
fn under_full_a_new_worktree_is_made_on_the_server() {
    run("on a server granting full, a send in a new worktree", 1);
    run("on a server granting less than full, a worktree picked", 1);
}

#[test]
fn a_refused_chat_says_why_and_the_command() {
    run("a chat a server refuses to run says why", 1);
}
