//! The command layer against temporary libraries. No test here names the user's library:
//! every [`Library`] is made by [`suite::Fixture`], in a directory that is deleted when
//! the test ends.

mod suite;
