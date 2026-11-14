//! The built `writer` binary, run the way an agent runs it: as a process, against a
//! library and a home directory that exist only for the test.

mod suite {
    pub mod harness;

    mod agents;
    mod documents;
    mod guard;
    mod images;
    mod items;
    mod live;
    mod mcp;
    mod output;
    mod pages;
    mod review;
}
