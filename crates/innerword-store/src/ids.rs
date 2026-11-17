//! Typed row ids.

use std::fmt;

use rusqlite::ToSql;
use rusqlite::types::{FromSql, FromSqlResult, ToSqlOutput, ValueRef};
use serde::{Deserialize, Serialize};

macro_rules! id_type {
    ($(#[$doc:meta])* $name:ident) => {
        $(#[$doc])*
        ///
        /// Wraps the SQLite rowid. Ids are never reused while the row exists; serialises as a
        /// plain integer.
        #[derive(
            Copy, Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub i64);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }

        impl ToSql for $name {
            fn to_sql(&self) -> rusqlite::Result<ToSqlOutput<'_>> {
                Ok(ToSqlOutput::from(self.0))
            }
        }

        impl FromSql for $name {
            fn column_result(value: ValueRef<'_>) -> FromSqlResult<Self> {
                i64::column_result(value).map($name)
            }
        }
    };
}

id_type!(
    /// Identifies a folder.
    FolderId
);
id_type!(
    /// Identifies a document.
    DocumentId
);
id_type!(
    /// Identifies a version snapshot.
    SnapshotId
);

/// Converts a count to the SQLite integer type, saturating.
pub(crate) fn count_to_sql(n: u64) -> i64 {
    i64::try_from(n).unwrap_or(i64::MAX)
}

/// Converts a stored count back, clamping corrupt negative values to zero.
pub(crate) fn count_from_sql(n: i64) -> u64 {
    u64::try_from(n).unwrap_or(0)
}
