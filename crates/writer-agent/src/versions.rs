//! History: the versions the library keeps of a document.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{
    ListVersionsRequest, ReadVersionRequest, RestoreVersionRequest, VersionList, VersionRestored,
    VersionText,
};

impl Library {
    /// The versions of a document, newest first.
    pub fn versions(&self, request: &ListVersionsRequest) -> Result<VersionList> {
        let _ = request;
        unfinished("versions")
    }

    /// The text of one version.
    pub fn read_version(&self, request: &ReadVersionRequest) -> Result<VersionText> {
        let _ = request;
        unfinished("read_version")
    }

    /// Makes a version the text of its document again, after keeping the text as it is
    /// now as a version. See [`RestoreVersionRequest`].
    pub fn restore_version(&self, request: &RestoreVersionRequest) -> Result<VersionRestored> {
        let _ = request;
        unfinished("restore_version")
    }
}
