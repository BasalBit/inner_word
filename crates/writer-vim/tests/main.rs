//! Tests for the Vim engine: one binary, a table-driven harness and one
//! module per area.

mod harness;

mod api;
mod code;
mod ex;
mod fuzz;
mod insert;
mod motions;
mod operators;
mod registers;
mod repeat;
mod rich;
mod scroll;
mod search;
mod textobjects;
mod undo;
mod visual;
mod void;
