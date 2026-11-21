//! All of this crate's integration tests in one binary: each top-level
//! tests/*.rs file would be its own binary, linked against the whole crate
//! (see docs/reference/testing.md). Add a suite as tests/it/<name>.rs plus
//! a mod line here. Only a suite that needs a process of its own (process-wide
//! state that plain `cargo test` would share) stays a top-level tests/<name>.rs.

mod android_gates;
mod archived_list;
mod chat_drop;
mod chat_paste;
mod chat_window;
mod cli;
mod composer_tray;
mod design_lint;
mod dictation;
mod full_screen_titlebar;
mod hover_cards;
mod leftovers;
mod onboarding_continue_ink;
mod pointer_cursor;
mod reload;
mod sidebar_row_ink;
mod streaming_cost;
mod thought_streaming;
