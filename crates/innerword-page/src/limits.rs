//! How much a page may be, and how long making one may take (docs/pages-design.md,
//! section 2, "Limits").

use std::time::Duration;

/// One file: an input file, a piece fetched from the web, a file after rewriting.
pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
/// One page: its distinct bytes, served and original files together.
pub const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;
/// Files of one page (its served files).
pub const MAX_FILES: usize = 2_000;
/// Distinct addresses on the web one page may need.
pub const MAX_EXTERNALS: usize = 500;
/// The rendered text a page keeps; the rest is cut, with a warning.
pub const MAX_TEXT_CHARS: usize = 2_000_000;
/// One address, as `curl --max-time` takes it.
pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
/// Every fetch of one page together.
pub const FETCH_BUDGET: Duration = Duration::from_secs(180);
/// Addresses fetched at the same time.
pub const FETCH_PARALLEL: usize = 6;
/// Nesting of `@import` chains and module graphs on the web.
pub const MAX_DEPTH: usize = 8;
/// Other HTML files of a page the check loads besides the entry; more is refused.
pub const ALSO_LOADS: usize = 100;
/// The check waits at least this long after `load` of the entry.
pub const SETTLE_MIN_MS: u64 = 1_000;
/// And at most this long.
pub const SETTLE_MAX_MS: u64 = 3_000;
/// The whole check.
pub const CHECK_LIMIT_MS: u64 = 120_000;
/// Where `ProcessChecker` kills the child that runs the check.
pub const CHECK_KILL_MS: u64 = 135_000;
/// Redirects the fetcher follows for one address.
pub const MAX_REDIRECTS: usize = 5;
/// The longest title a page takes from its HTML, in characters.
pub const MAX_TITLE_CHARS: usize = 200;
