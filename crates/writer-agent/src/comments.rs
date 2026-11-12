//! Comment threads: reviewing a text instead of rewriting it.

use std::collections::BTreeMap;

use writer_core::{CommentId, CommentThread, Document, EditorState, ReplyId, TextRange};

use crate::content::{self, Form, Reason};
use crate::error::{Error, ItemKind, Result};
use crate::library::{Library, author_name, author_phrase};
use crate::matching;
use crate::resolve::Index;
use crate::sections::heading_above;
use crate::types::{
    AddCommentRequest, CommentChanged, CommentList, CommentRequest, CommentThreadInfo,
    DeleteCommentRequest, ListCommentsRequest, ReplyInfo, ReplyRequest, Target, Time, VersionInfo,
    WriteOptions,
};

/// Every comment thread of `document` as results show it, in the order of the text;
/// threads without a place last.
pub(crate) fn thread_infos(document: &Document) -> Vec<CommentThreadInfo> {
    let mut ranges: BTreeMap<CommentId, Vec<TextRange>> = BTreeMap::new();
    for (id, range) in document.all_comment_ranges() {
        ranges.entry(id).or_default().push(range);
    }
    document
        .threads_in_order()
        .into_iter()
        .filter_map(|id| document.comment(id))
        .map(|thread| thread_info(document, thread, ranges.get(&thread.id).map(Vec::as_slice)))
        .collect()
}

fn thread_info(
    document: &Document,
    thread: &CommentThread,
    ranges: Option<&[TextRange]>,
) -> CommentThreadInfo {
    let ranges = ranges.unwrap_or(&[]);
    let anchored_text = (!ranges.is_empty()).then(|| {
        ranges
            .iter()
            .map(|range| document.text_in(*range))
            .collect::<Vec<_>>()
            .join(" \u{2026} ")
    });
    let block = ranges.first().map(|range| range.start.block);
    CommentThreadInfo {
        id: thread.id.0,
        author: thread.author.clone(),
        body: thread.body.clone(),
        created_at: Time(thread.created_at),
        resolved: thread.resolved,
        anchored_text,
        heading: block.and_then(|block| heading_above(document, block)),
        block,
        replies: thread
            .replies
            .iter()
            .map(|reply| ReplyInfo {
                id: reply.id.0,
                author: reply.author.clone(),
                body: reply.body.clone(),
                created_at: Time(reply.created_at),
            })
            .collect(),
    }
}

/// The thread `id` of `document` as results show it.
fn one_thread(document: &Document, id: CommentId) -> Option<CommentThreadInfo> {
    let thread = document.comment(id)?;
    let ranges = document.comment_ranges(id);
    Some(thread_info(document, thread, Some(&ranges)))
}

fn no_such_thread(document: &Document, id: u64) -> Error {
    let ids: Vec<String> = document
        .threads_in_order()
        .iter()
        .take(20)
        .map(|id| id.0.to_string())
        .collect();
    Error::NotFound {
        kind: ItemKind::Comment,
        reference: format!("with the id {id} in this document"),
        hint: Some(if ids.is_empty() {
            "The document has no comments".to_string()
        } else {
            format!("Its comment threads have the ids {}", ids.join(", "))
        }),
    }
}

/// What a change to a thread does to the document.
enum Step {
    /// The thread `0` is there afterwards (added or changed).
    Kept(CommentId, Option<u64>),
    /// The thread was deleted; this is what it was.
    Deleted(CommentThreadInfo),
}

impl Library {
    /// The comment threads of a document, each with the text it is anchored to.
    pub fn comments(&self, request: &ListCommentsRequest) -> Result<CommentList> {
        let session = self.reader()?;
        let index = Index::load(&session.store)?;
        let meta = index.document(&session.store, &request.document)?;
        let loaded = content::load(&session.store, &meta)?;
        let mut threads = thread_infos(&loaded.document);
        if request.open_only {
            threads.retain(|thread| !thread.resolved);
        }
        Ok(CommentList {
            document: index.document_summary(&session.store, &loaded.meta),
            threads,
        })
    }

    /// Starts a comment thread on an exact quote of the text. See [`AddCommentRequest`].
    pub fn add_comment(&self, request: &AddCommentRequest) -> Result<CommentChanged> {
        if request.body.trim().is_empty() {
            return Err(Error::invalid("the comment is empty"));
        }
        if request.quote.trim().is_empty() {
            return Err(Error::invalid(
                "a comment needs the text it is about: quote it exactly",
            ));
        }
        let author = author_name(&request.write);
        let now = self.clock.now_ms();
        self.change_comments(&request.document, &request.write, None, |ed| {
            let matches = matching::find(ed.document(), &request.quote, false)?;
            let ranges = matching::choose(
                ed.document(),
                &request.quote,
                &matches,
                false,
                request.occurrence,
            )?;
            ed.select_range(ranges[0]);
            let id = ed
                .add_comment(&request.body, &author, now)
                .ok_or_else(|| Error::invalid("the quoted text is no text a comment can be on"))?;
            Ok(Step::Kept(id, None))
        })
    }

    /// Adds a reply to a comment thread.
    pub fn reply(&self, request: &ReplyRequest) -> Result<CommentChanged> {
        if request.body.trim().is_empty() {
            return Err(Error::invalid("the reply is empty"));
        }
        let author = author_name(&request.write);
        let now = self.clock.now_ms();
        self.change_comments(&request.document, &request.write, None, |ed| {
            let id = CommentId(request.comment);
            if ed.document().comment(id).is_none() {
                return Err(no_such_thread(ed.document(), request.comment));
            }
            let reply = ed
                .add_reply(id, &request.body, &author, now)
                .ok_or_else(|| Error::invalid("the reply could not be added"))?;
            Ok(Step::Kept(id, Some(reply.0)))
        })
    }

    /// Marks a comment thread as resolved.
    pub fn resolve_comment(&self, request: &CommentRequest) -> Result<CommentChanged> {
        self.change_comments(&request.document, &request.write, None, |ed| {
            let id = CommentId(request.comment);
            if ed.document().comment(id).is_none() {
                return Err(no_such_thread(ed.document(), request.comment));
            }
            ed.resolve_comment(id);
            Ok(Step::Kept(id, None))
        })
    }

    /// Opens a resolved comment thread again.
    pub fn reopen_comment(&self, request: &CommentRequest) -> Result<CommentChanged> {
        self.change_comments(&request.document, &request.write, None, |ed| {
            let id = CommentId(request.comment);
            if ed.document().comment(id).is_none() {
                return Err(no_such_thread(ed.document(), request.comment));
            }
            ed.reopen_comment(id);
            Ok(Step::Kept(id, None))
        })
    }

    /// Deletes a comment thread, or one reply of it. See [`DeleteCommentRequest`].
    pub fn delete_comment(&self, request: &DeleteCommentRequest) -> Result<CommentChanged> {
        self.change_comments(
            &request.document,
            &request.write,
            Some(Reason::CommentDeleted),
            |ed| {
                let id = CommentId(request.comment);
                let Some(was) = one_thread(ed.document(), id) else {
                    return Err(no_such_thread(ed.document(), request.comment));
                };
                match request.reply {
                    Some(reply) => {
                        if !was.replies.iter().any(|other| other.id == reply) {
                            return Err(Error::NotFound {
                                kind: ItemKind::Reply,
                                reference: format!(
                                    "with the id {reply} in the comment thread {id}"
                                ),
                                hint: Some(if was.replies.is_empty() {
                                    "The thread has no replies".to_string()
                                } else {
                                    format!(
                                        "Its replies have the ids {}",
                                        was.replies
                                            .iter()
                                            .map(|reply| reply.id.to_string())
                                            .collect::<Vec<_>>()
                                            .join(", ")
                                    )
                                }),
                            });
                        }
                        ed.delete_reply(id, ReplyId(reply));
                        Ok(Step::Kept(id, Some(reply)))
                    }
                    None => {
                        ed.delete_comment(id);
                        Ok(Step::Deleted(was))
                    }
                }
            },
        )
    }

    /// Runs a change to the comment threads of a document and stores the result. With
    /// `keep` a version of the document as it was is kept first (something is deleted).
    fn change_comments(
        &self,
        target: &Target,
        options: &WriteOptions,
        keep: Option<Reason>,
        change: impl FnOnce(&mut EditorState) -> Result<Step>,
    ) -> Result<CommentChanged> {
        let mut session = self.writer(options)?;
        let index = Index::load(&session.store)?;
        let meta = index.live_document(&session.store, target)?;
        let loaded = content::load(&session.store, &meta)?;
        let mut ed = EditorState::new(loaded.document.clone());
        let step = change(&mut ed)?;
        let after = ed.into_document();
        let changed = loaded.document != after;
        let (thread, reply) = match step {
            Step::Kept(id, reply) => {
                let mut thread = one_thread(&after, id).ok_or_else(|| {
                    Error::invalid(format!("the comment thread {id} is gone after the change"))
                })?;
                if session.dry_run && changed {
                    // What would be created has no id yet.
                    if loaded.document.comment(id).is_none() {
                        thread.id = 0;
                    } else if let Some(reply) = reply
                        && let Some(new) = thread.replies.iter_mut().find(|other| other.id == reply)
                        && loaded
                            .document
                            .comment(id)
                            .is_some_and(|thread| thread.reply(ReplyId(reply)).is_none())
                    {
                        new.id = 0;
                    }
                }
                let reply = reply.map(|reply| {
                    let is_new = loaded
                        .document
                        .comment(id)
                        .is_some_and(|thread| thread.reply(ReplyId(reply)).is_none());
                    if session.dry_run && is_new { 0 } else { reply }
                });
                (thread, reply)
            }
            Step::Deleted(was) => (was, None),
        };
        let mut snapshot: Option<VersionInfo> = None;
        let mut meta = loaded.meta.clone();
        if changed && !session.dry_run {
            let kept_title = loaded.custom_title.then(|| loaded.meta.title.clone());
            if let Some(reason) = keep {
                snapshot = Some(content::keep_version(
                    &mut session.store,
                    loaded.meta.id,
                    &loaded.raw,
                    &reason.label(&author_phrase(options)),
                )?);
            }
            let form = Form::of(&after, kept_title.as_deref());
            meta = session
                .store
                .save_document(loaded.meta.id, &form.content())?;
        }
        let result = CommentChanged {
            document: index.document_summary(&session.store, &meta),
            thread,
            reply,
            snapshot,
            written: session.written(changed),
        };
        session.finish()?;
        Ok(result)
    }
}
