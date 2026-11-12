//! Comment threads: reviewing a text instead of rewriting it.

use crate::error::Result;
use crate::library::{Library, unfinished};
use crate::types::{
    AddCommentRequest, CommentChanged, CommentList, CommentRequest, DeleteCommentRequest,
    ListCommentsRequest, ReplyRequest,
};

impl Library {
    /// The comment threads of a document, each with the text it is anchored to.
    pub fn comments(&self, request: &ListCommentsRequest) -> Result<CommentList> {
        let _ = request;
        unfinished("comments")
    }

    /// Starts a comment thread on an exact quote of the text. See [`AddCommentRequest`].
    pub fn add_comment(&self, request: &AddCommentRequest) -> Result<CommentChanged> {
        let _ = request;
        unfinished("add_comment")
    }

    /// Adds a reply to a comment thread.
    pub fn reply(&self, request: &ReplyRequest) -> Result<CommentChanged> {
        let _ = request;
        unfinished("reply")
    }

    /// Marks a comment thread as resolved.
    pub fn resolve_comment(&self, request: &CommentRequest) -> Result<CommentChanged> {
        let _ = request;
        unfinished("resolve_comment")
    }

    /// Opens a resolved comment thread again.
    pub fn reopen_comment(&self, request: &CommentRequest) -> Result<CommentChanged> {
        let _ = request;
        unfinished("reopen_comment")
    }

    /// Deletes a comment thread, or one reply of it. See [`DeleteCommentRequest`].
    pub fn delete_comment(&self, request: &DeleteCommentRequest) -> Result<CommentChanged> {
        let _ = request;
        unfinished("delete_comment")
    }
}
