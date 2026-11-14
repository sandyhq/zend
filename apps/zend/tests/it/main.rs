//! All of this crate's integration tests in one binary: each top-level
//! tests/*.rs file would be its own binary, linked against the whole crate
//! (see docs/reference/testing.md). Add a suite as tests/it/<name>.rs plus
//! a mod line here. Only a suite that needs a process of its own (process-wide
//! state that plain `cargo test` would share) stays a top-level tests/<name>.rs.

mod background_engine;
mod cli;
mod noop_browser;
