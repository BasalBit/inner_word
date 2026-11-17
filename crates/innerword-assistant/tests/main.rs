//! The process-level tests of the engine: the Claude Code backend against a stand-in
//! for the `claude` program (`tests/fake_claude`) that replays recordings of the real
//! one (`tests/fixtures`). No test here, and none in the crate, runs the real program.

mod suite;
