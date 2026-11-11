//! Tests for the Vim engine: one binary, a table-driven harness and one
//! module per area.

mod harness;

mod motions;
mod operators;
mod textobjects;
mod insert;
mod visual;
mod registers;
mod search;
mod ex;
mod undo;
mod repeat;
mod rich;
mod void;
mod code;
mod scroll;
mod api;
mod fuzz;
