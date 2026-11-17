//! What can go wrong.

use std::path::PathBuf;

use crate::agents::AgentId;

/// A failure of this crate. Every message says which path it is about.
#[derive(Debug, thiserror::Error)]
pub enum SetupError {
    /// The path of the `innerword` binary must be absolute: a skill or an MCP entry with a
    /// relative path works from one folder only.
    #[error("the path of the innerword tool must be absolute, and this one is not: {}", .0.display())]
    BinaryNotAbsolute(PathBuf),

    /// A folder (or file) with the skill's name is already there and was not written by Inner
    /// Word. It is left alone.
    #[error(
        "{} already exists and was not installed by Inner Word; it is left as it is. Move or rename it, then install again.",
        .0.display()
    )]
    Foreign(PathBuf),

    /// No directory is known that this agent reads skills from.
    #[error("no skills folder is known for {0}")]
    NoSkillsDir(AgentId),

    /// The home directory could not be found (`HOME` is not set).
    #[error("the home directory is not known: HOME is not set")]
    NoHome,

    /// Reading or writing failed.
    #[error("{action} {}: {source}", .path.display())]
    Io {
        /// What was being done, as the start of a sentence ("could not write").
        action: &'static str,
        path: PathBuf,
        #[source]
        source: std::io::Error,
    },
}

impl SetupError {
    pub(crate) fn io(
        action: &'static str,
        path: impl Into<PathBuf>,
        source: std::io::Error,
    ) -> Self {
        SetupError::Io {
            action,
            path: path.into(),
            source,
        }
    }
}
