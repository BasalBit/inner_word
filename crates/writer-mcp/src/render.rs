//! The results of the command layer as text.
//!
//! One function per result type. The text is what `writer` prints and what a tool call
//! returns beside the structured data, so it is written for an agent reading it: complete
//! (every id needed for the next step is in it), compact, plain ASCII punctuation, no
//! decoration. Ids are written the way they can be passed back: `doc:12`, `folder:3`.

use writer_agent::{
    CommentChanged, CommentList, CommentThreadInfo, ContentChanged, DocumentChanged,
    DocumentCreated, DocumentInfo, DocumentList, DocumentSummary, DocumentText, ExportFormat,
    Exported, FindResults, FolderChanged, FolderCreated, FolderInfo, FolderSummary, LibraryStatus,
    MatchedAs, SearchResults, Time, TrashEmptied, TrashItem, TrashList, Tree, TreeFolder,
    VersionInfo, VersionList, VersionRestored, VersionText, Written,
};

/// A number with thin grouping: `12,345`.
pub fn count(n: u64) -> String {
    let digits = n.to_string();
    let mut out = String::with_capacity(digits.len() + digits.len() / 3);
    for (index, digit) in digits.chars().enumerate() {
        if index > 0 && (digits.len() - index).is_multiple_of(3) {
            out.push(',');
        }
        out.push(digit);
    }
    out
}

/// `1 word`, `2 words`.
pub fn plural(n: u64, one: &str, many: &str) -> String {
    format!("{} {}", count(n), if n == 1 { one } else { many })
}

/// A time to the minute, in UTC: `2026-11-12T09:30Z`.
pub fn time(time: Time) -> String {
    let full = time.to_rfc3339();
    match full.get(..16) {
        Some(minute) => format!("{minute}Z"),
        None => full,
    }
}

/// How a document is named in a sentence: `doc:12 "Book/Chapter 1"`. One that does not
/// exist yet (a dry run reports it with the id 0) is `a new document "Book/Chapter 1"`.
pub fn document_name(document: &DocumentSummary) -> String {
    match document.id.0 {
        0 => format!("a new document \"{}\"", document.path),
        id => format!("doc:{id} \"{}\"", document.path),
    }
}

/// How a folder is named in a sentence: `folder:3 "Book/Part one"`, or for one that does
/// not exist yet `a new folder "Book/Part one"`.
pub fn folder_name(folder: &FolderSummary) -> String {
    match folder.id.0 {
        0 => format!("a new folder \"{}\"", folder.path),
        id => format!("folder:{id} \"{}\"", folder.path),
    }
}

/// One line for a document in a list: id, path, size, last change.
pub fn document_line(document: &DocumentSummary) -> String {
    format!(
        "doc:{}  {}  ({}, changed {})",
        document.id.0,
        document.path,
        plural(document.words, "word", "words"),
        time(document.updated_at)
    )
}

/// What a dry run says before the report of what the real run would do.
const DRY_RUN: &str = "Dry run: nothing was changed. The real run would report:\n";

/// What a write says about itself, in front of its report.
fn dry_run_prefix(written: &Written) -> &'static str {
    if written.dry_run { DRY_RUN } else { "" }
}

/// What a write says about itself, after everything else.
fn written_suffix(written: &Written, out: &mut String) {
    if written.dry_run && written.app_is_open {
        out.push_str(
            "\nThe Writer is open: the real run will be refused until the user quits the app.",
        );
    }
}

/// `writer status`.
pub fn status(status: &LibraryStatus) -> String {
    let mut out = String::new();
    out.push_str(&format!("Library: {}\n", status.data_dir.display()));
    if !status.exists {
        out.push_str("There is no library there yet (The Writer creates it on its first run; so does the first write).\n");
    } else {
        out.push_str(&format!(
            "{}, {}, {} in the trash\n",
            plural(status.documents, "document", "documents"),
            plural(status.folders, "folder", "folders"),
            plural(status.trashed, "item", "items")
        ));
    }
    if status.app_is_open {
        let pid = status
            .app_pid
            .map(|pid| format!(" (process {pid})"))
            .unwrap_or_default();
        out.push_str(&format!(
            "The Writer is open{pid}: reading works, every change is refused until the user quits the app.\n"
        ));
    } else {
        out.push_str("The Writer is not running: reading and writing both work.\n");
    }
    out.push_str(&format!("writer {}\n", status.version));
    out
}

/// `writer list`: the tree.
pub fn tree(tree: &Tree) -> String {
    let mut out = String::new();
    match &tree.folder {
        Some(folder) => out.push_str(&format!("{}/  folder:{}\n", folder.path, folder.id.0)),
        None => out.push_str("Library\n"),
    }
    tree_level(&tree.folders, &tree.documents, 1, &mut out);
    if tree.folders.is_empty()
        && tree.documents.is_empty()
        && tree.total_documents == 0
        && tree.total_folders == 0
    {
        out.push_str("  (empty)\n");
    }
    out.push_str(&format!(
        "{}, {}, {}\n",
        plural(tree.total_folders, "folder", "folders"),
        plural(tree.total_documents, "document", "documents"),
        plural(tree.total_words, "word", "words")
    ));
    out
}

fn tree_level(
    folders: &[TreeFolder],
    documents: &[DocumentSummary],
    depth: usize,
    out: &mut String,
) {
    let indent = "  ".repeat(depth);
    for folder in folders {
        let shown = !folder.folders.is_empty() || !folder.documents.is_empty();
        let holds = folder.folder_count + folder.document_count;
        out.push_str(&format!(
            "{indent}{}/  folder:{}",
            folder.folder.name, folder.folder.id.0
        ));
        if holds == 0 {
            out.push_str("  (empty)");
        } else if !shown {
            out.push_str(&format!(
                "  ({}, {} inside)",
                plural(folder.folder_count, "folder", "folders"),
                plural(folder.document_count, "document", "documents")
            ));
        }
        out.push('\n');
        tree_level(&folder.folders, &folder.documents, depth + 1, out);
    }
    for document in documents {
        out.push_str(&format!(
            "{indent}{}  doc:{}  ({}, changed {})\n",
            document.title,
            document.id.0,
            plural(document.words, "word", "words"),
            time(document.updated_at)
        ));
    }
}

/// `writer search`.
pub fn search(results: &SearchResults) -> String {
    if results.hits.is_empty() {
        return format!("No document matches \"{}\".\n", results.query);
    }
    let mut out = match results.hits.len() {
        1 => format!("1 document matches \"{}\":\n", results.query),
        n => format!("{n} documents match \"{}\":\n", results.query),
    };
    for hit in &results.hits {
        out.push_str(&document_line(&hit.document));
        out.push('\n');
        let mut snippet = String::new();
        for part in &hit.snippet_parts {
            if part.is_match {
                snippet.push_str("**");
                snippet.push_str(&part.text);
                snippet.push_str("**");
            } else {
                snippet.push_str(&part.text);
            }
        }
        let snippet = if hit.snippet_parts.is_empty() {
            hit.snippet.clone()
        } else {
            snippet
        };
        let snippet = one_line(&snippet);
        if !snippet.is_empty() {
            out.push_str(&format!("    {snippet}\n"));
        }
    }
    out
}

/// `writer recent`.
pub fn documents(list: &DocumentList) -> String {
    if list.documents.is_empty() {
        return "No documents.\n".to_string();
    }
    let mut out = String::new();
    for document in &list.documents {
        out.push_str(&document_line(document));
        out.push('\n');
    }
    out
}

/// `writer info` for a document.
pub fn info(info: &DocumentInfo) -> String {
    let document = &info.document;
    let settings = &info.settings;
    let mut out = String::new();
    out.push_str(&format!("doc:{}  {}\n", document.id.0, document.path));
    out.push_str(&format!(
        "title:     {} ({})\n",
        document.title,
        if info.custom_title {
            "chosen by hand"
        } else {
            "follows the text"
        }
    ));
    match document.folder_id {
        Some(folder) => out.push_str(&format!(
            "folder:    {}  folder:{}\n",
            document.folder, folder.0
        )),
        None => out.push_str("folder:    (the top level of the library)\n"),
    }
    out.push_str(&format!(
        "size:      {}, {} ({} without spaces), {}, about {} min to read\n",
        plural(document.words, "word", "words"),
        plural(document.characters, "character", "characters"),
        count(info.characters_no_spaces),
        plural(info.paragraphs, "paragraph", "paragraphs"),
        info.reading_minutes
    ));
    out.push_str(&format!("created:   {}\n", time(document.created_at)));
    out.push_str(&format!("changed:   {}\n", time(document.updated_at)));
    if let Some(trashed) = document.trashed_at {
        out.push_str(&format!("trashed:   {} (in the trash)\n", time(trashed)));
    }
    out.push_str(&format!(
        "comments:  {} open, {} resolved\n",
        info.open_comments, info.resolved_comments
    ));
    out.push_str(&format!("versions:  {}\n", info.versions));
    out.push_str(&format!(
        "page:      {} {}, {} x {} pt, margins {} {} {} {} pt (top right bottom left), page view {}\n",
        settings.paper,
        settings.orientation,
        number(settings.page_width),
        number(settings.page_height),
        number(settings.margins[0]),
        number(settings.margins[1]),
        number(settings.margins[2]),
        number(settings.margins[3]),
        if settings.page_view { "on" } else { "off" }
    ));
    out.push_str(&format!(
        "type:      {} {} pt, line height {}, paragraph spacing {} pt\n",
        settings.font,
        number(settings.text_size),
        number(settings.line_height),
        number(settings.paragraph_spacing)
    ));
    out.push_str(&format!(
        "spelling:  {}\n",
        match settings.spelling_language.as_deref() {
            None => "the app's language",
            Some("auto") => "detected for each paragraph",
            Some(language) => language,
        }
    ));
    if info.headings.is_empty() {
        out.push_str("headings:  none\n");
    } else {
        out.push_str("headings:\n");
        for heading in &info.headings {
            let twin = if heading.occurrence > 1 {
                format!(", occurrence {}", heading.occurrence)
            } else {
                String::new()
            };
            out.push_str(&format!(
                "  {} {}  ({}{twin})\n",
                "#".repeat(usize::from(heading.level.clamp(1, 6))),
                heading.text,
                plural(heading.words, "word", "words")
            ));
        }
    }
    out
}

/// `writer info` for a folder: the folder and what is directly in it.
pub fn folder_info(info: &FolderInfo) -> String {
    let mut out = String::new();
    match &info.folder {
        Some(folder) => {
            out.push_str(&format!("folder:{}  {}\n", folder.id.0, folder.path));
            out.push_str(&format!("name:      {}\n", folder.name));
            match folder.parent_id {
                Some(parent) => out.push_str(&format!("inside:    folder:{}\n", parent.0)),
                None => out.push_str("inside:    (the top level of the library)\n"),
            }
            out.push_str(&format!("created:   {}\n", time(folder.created_at)));
            out.push_str(&format!("changed:   {}\n", time(folder.updated_at)));
            if let Some(trashed) = folder.trashed_at {
                out.push_str(&format!("trashed:   {} (in the trash)\n", time(trashed)));
            }
        }
        None => out.push_str("The top level of the library\n"),
    }
    out.push_str(&format!(
        "holds:     {}, {}, {}\n",
        plural(info.total_folders, "folder", "folders"),
        plural(info.total_documents, "document", "documents"),
        plural(info.total_words, "word", "words")
    ));
    if !info.folders.is_empty() || !info.documents.is_empty() {
        out.push_str("directly in it:\n");
        for folder in &info.folders {
            out.push_str(&format!("  {}/  folder:{}\n", folder.name, folder.id.0));
        }
        for document in &info.documents {
            out.push_str(&format!(
                "  {}  doc:{}  ({}, changed {})\n",
                document.title,
                document.id.0,
                plural(document.words, "word", "words"),
                time(document.updated_at)
            ));
        }
    }
    out
}

/// `writer get`: the text and nothing else, unless the comments were asked for.
pub fn text(text: &DocumentText) -> String {
    let mut out = text.content.clone();
    if let Some(threads) = &text.comments {
        if !out.is_empty() && !out.ends_with('\n') {
            out.push('\n');
        }
        out.push_str("\n---\n");
        if threads.is_empty() {
            out.push_str("No comments.\n");
        } else {
            out.push_str(&format!(
                "{}:\n",
                plural(threads.len() as u64, "comment thread", "comment threads")
            ));
            for thread in threads {
                thread_lines(thread, &mut out);
            }
        }
    }
    out
}

/// `writer find`.
pub fn found(results: &FindResults) -> String {
    let name = document_name(&results.document);
    if results.count == 0 {
        return format!(
            "The text does not occur in {name}. It has to be given exactly as it stands in the document.\n"
        );
    }
    let mut out = format!(
        "The text occurs {} in {name}{}.\n",
        match results.count {
            1 => "once".to_string(),
            n => format!("{n} times"),
        },
        match results.matched_as {
            Some(MatchedAs::Markdown) => " (found after reading it as Markdown)",
            _ => "",
        }
    );
    for occurrence in &results.occurrences {
        out.push_str(&format!("  {}  ", occurrence.occurrence));
        if let Some(heading) = &occurrence.heading {
            out.push_str(&format!("[{heading}] "));
        }
        out.push_str(&one_line(&occurrence.context));
        out.push('\n');
    }
    if results.count > results.occurrences.len() {
        out.push_str(&format!(
            "  and {} more\n",
            results.count - results.occurrences.len()
        ));
    }
    out
}

/// `writer create`.
pub fn created(created: &DocumentCreated) -> String {
    let mut out = String::from(dry_run_prefix(&created.written));
    let document = &created.document;
    out.push_str(&format!(
        "Created {} ({}).\n",
        document_name(document),
        plural(document.words, "word", "words")
    ));
    for folder in &created.created_folders {
        out.push_str(&format!("Folder created: {}\n", folder_name(folder)));
    }
    warnings(&created.warnings, &mut out);
    written_suffix(&created.written, &mut out);
    finish(out)
}

/// A change to the text of a document. `did` is what was done to it, as the start of a
/// sentence: "Edited", "Appended to".
pub fn content_changed(did: &str, changed: &ContentChanged) -> String {
    let written = &changed.written;
    let name = document_name(&changed.document);
    let mut out = String::new();
    if !written.changed && !written.dry_run {
        out.push_str(&format!(
            "Nothing changed: {name} already reads that way.\n"
        ));
        warnings(&changed.warnings, &mut out);
        return out;
    }
    out.push_str(dry_run_prefix(written));
    out.push_str(&format!("{did} {name}"));
    let mut facts: Vec<String> = Vec::new();
    if changed.replacements > 0 {
        let mut fact = plural(changed.replacements as u64, "replacement", "replacements");
        if changed.matched_as == Some(MatchedAs::Markdown) {
            fact.push_str(" (the old text was found after reading it as Markdown)");
        }
        facts.push(fact);
    }
    let mut blocks: Vec<String> = Vec::new();
    if changed.blocks_added > 0 {
        blocks.push(format!("{} added", changed.blocks_added));
    }
    if changed.blocks_removed > 0 {
        blocks.push(format!("{} removed", changed.blocks_removed));
    }
    if changed.blocks_changed > 0 {
        blocks.push(format!("{} changed", changed.blocks_changed));
    }
    if !blocks.is_empty() {
        facts.push(format!("blocks: {}", blocks.join(", ")));
    }
    facts.push(format!(
        "{} -> {} words",
        count(changed.words_before),
        count(changed.words_after)
    ));
    out.push_str(&format!(": {}.\n", facts.join("; ")));
    if !changed.headings.is_empty() {
        let headings: Vec<String> = changed
            .headings
            .iter()
            .map(|heading| format!("\"{heading}\""))
            .collect();
        out.push_str(&format!("Under: {}\n", headings.join(", ")));
    }
    if let Some(snapshot) = &changed.snapshot {
        out.push_str(&format!(
            "The text as it was is kept as version {} ({}); \"writer history restore {}\" brings it back.\n",
            snapshot.id.0,
            snapshot.label.as_deref().unwrap_or("no label"),
            snapshot.id.0
        ));
    }
    warnings(&changed.warnings, &mut out);
    if !changed.comments_detached.is_empty() {
        out.push_str(&format!(
            "{} lost the text {} anchored to and {} kept without a place:\n",
            plural(
                changed.comments_detached.len() as u64,
                "comment thread",
                "comment threads"
            ),
            if changed.comments_detached.len() == 1 {
                "it was"
            } else {
                "they were"
            },
            if changed.comments_detached.len() == 1 {
                "is"
            } else {
                "are"
            },
        ));
        for thread in &changed.comments_detached {
            out.push_str(&format!(
                "  thread {}: {} ({})\n",
                thread.id,
                one_line(&thread.body),
                thread.author
            ));
        }
    }
    written_suffix(written, &mut out);
    finish(out)
}

/// A change to a document's name, place or state. `did` is the start of the sentence:
/// "Renamed", "Moved", "Moved to the trash:".
pub fn document_changed(did: &str, changed: &DocumentChanged) -> String {
    let written = &changed.written;
    let mut out = String::new();
    if !written.changed && !written.dry_run {
        out.push_str(&format!(
            "Nothing changed: {} is already so.\n",
            document_name(&changed.document)
        ));
        return out;
    }
    out.push_str(dry_run_prefix(written));
    match &changed.before {
        Some(before) if before.path != changed.document.path => out.push_str(&format!(
            "{did} doc:{} \"{}\" -> \"{}\".\n",
            changed.document.id.0, before.path, changed.document.path
        )),
        Some(_) if did == "Moved" => out.push_str(&format!(
            "Moved {} to another place among the documents of its folder.\n",
            document_name(&changed.document)
        )),
        _ => out.push_str(&format!("{did} {}.\n", document_name(&changed.document))),
    }
    for folder in &changed.created_folders {
        out.push_str(&format!("Folder created: {}\n", folder_name(folder)));
    }
    written_suffix(written, &mut out);
    finish(out)
}

/// `writer duplicate`: the copy, and what it is a copy of.
pub fn document_copied(changed: &DocumentChanged) -> String {
    let written = &changed.written;
    let mut out = String::from(dry_run_prefix(written));
    out.push_str(&format!("Copied to {}", document_name(&changed.document)));
    if let Some(original) = &changed.before {
        out.push_str(&format!(" from {}", document_name(original)));
    }
    out.push_str(&format!(
        " ({}).\n",
        plural(changed.document.words, "word", "words")
    ));
    written_suffix(written, &mut out);
    finish(out)
}

/// `writer mkdir`.
pub fn folder_created(created: &FolderCreated) -> String {
    let written = &created.written;
    let mut out = String::from(dry_run_prefix(written));
    if created.created.is_empty() {
        out.push_str(&format!(
            "The folder is there already: {}.\n",
            folder_name(&created.folder)
        ));
    } else {
        out.push_str(&format!("Created {}.\n", folder_name(&created.folder)));
        for folder in created
            .created
            .iter()
            .filter(|folder| folder.path != created.folder.path)
        {
            out.push_str(&format!(
                "Folder created on the way: {}\n",
                folder_name(folder)
            ));
        }
    }
    written_suffix(written, &mut out);
    finish(out)
}

/// A change to a folder's name, place or state.
pub fn folder_changed(did: &str, changed: &FolderChanged) -> String {
    let written = &changed.written;
    let mut out = String::new();
    if !written.changed && !written.dry_run {
        out.push_str(&format!(
            "Nothing changed: {} is already so.\n",
            folder_name(&changed.folder)
        ));
        return out;
    }
    out.push_str(dry_run_prefix(written));
    match &changed.before {
        Some(before) if before.path != changed.folder.path => out.push_str(&format!(
            "{did} folder:{} \"{}\" -> \"{}\"",
            changed.folder.id.0, before.path, changed.folder.path
        )),
        Some(_) if did == "Moved" => out.push_str(&format!(
            "Moved {} to another place among the folders beside it",
            folder_name(&changed.folder)
        )),
        _ => out.push_str(&format!("{did} {}", folder_name(&changed.folder))),
    }
    if changed.folders > 0 || changed.documents > 0 {
        out.push_str(&format!(
            ", with {} and {} in it",
            plural(changed.folders, "folder", "folders"),
            plural(changed.documents, "document", "documents")
        ));
    }
    out.push_str(".\n");
    written_suffix(written, &mut out);
    finish(out)
}

/// `writer list --trash`.
pub fn trash(list: &TrashList) -> String {
    if list.entries.is_empty() {
        return "The trash is empty.\n".to_string();
    }
    let mut out = format!(
        "{} in the trash:\n",
        plural(list.entries.len() as u64, "item", "items")
    );
    for entry in &list.entries {
        match entry {
            TrashItem::Document { document } => out.push_str(&format!(
                "doc:{}  {}  ({}, trashed {})\n",
                document.id.0,
                document.path,
                plural(document.words, "word", "words"),
                document.trashed_at.map(time).unwrap_or_default()
            )),
            TrashItem::Folder {
                folder,
                folders,
                documents,
            } => out.push_str(&format!(
                "folder:{}  {}/  ({}, {}, trashed {})\n",
                folder.id.0,
                folder.path,
                plural(*folders, "folder", "folders"),
                plural(*documents, "document", "documents"),
                folder.trashed_at.map(time).unwrap_or_default()
            )),
        }
    }
    out.push_str("\"writer restore <id>\" brings one back.\n");
    out
}

/// `writer empty-trash`.
pub fn trash_emptied(emptied: &TrashEmptied) -> String {
    let written = &emptied.written;
    let mut out = String::from(dry_run_prefix(written));
    if emptied.folders == 0 && emptied.documents == 0 {
        out.push_str("The trash was empty.\n");
    } else {
        out.push_str(&format!(
            "Emptied the trash: erased {} and {} for good.\n",
            plural(emptied.folders, "folder", "folders"),
            plural(emptied.documents, "document", "documents")
        ));
    }
    written_suffix(written, &mut out);
    finish(out)
}

fn thread_lines(thread: &CommentThreadInfo, out: &mut String) {
    out.push_str(&format!(
        "thread {}  {}",
        thread.id,
        if thread.resolved { "resolved" } else { "open" }
    ));
    if let Some(heading) = &thread.heading {
        out.push_str(&format!("  under \"{heading}\""));
    }
    out.push('\n');
    match &thread.anchored_text {
        Some(anchored) => out.push_str(&format!("  on: \"{}\"\n", one_line(anchored))),
        None => out.push_str("  on: (the text it was anchored to is gone)\n"),
    }
    out.push_str(&format!(
        "  {} ({}): {}\n",
        thread.author,
        time(thread.created_at),
        indent_rest(&thread.body, 4)
    ));
    for reply in &thread.replies {
        out.push_str(&format!(
            "    reply {}  {} ({}): {}\n",
            reply.id,
            reply.author,
            time(reply.created_at),
            indent_rest(&reply.body, 6)
        ));
    }
}

/// `writer comments`.
pub fn comments(list: &CommentList) -> String {
    let name = document_name(&list.document);
    if list.threads.is_empty() {
        return format!("No comments in {name}.\n");
    }
    let mut out = format!(
        "{} in {name}:\n",
        plural(
            list.threads.len() as u64,
            "comment thread",
            "comment threads"
        )
    );
    for thread in &list.threads {
        thread_lines(thread, &mut out);
    }
    out
}

/// A change to a comment thread. `did` is the start of the sentence: "Added", "Resolved".
pub fn comment_changed(did: &str, changed: &CommentChanged) -> String {
    let written = &changed.written;
    let name = document_name(&changed.document);
    let mut out = String::new();
    if !written.changed && !written.dry_run {
        out.push_str(&format!(
            "Nothing changed: thread {} in {name} is already so.\n",
            changed.thread.id
        ));
        return out;
    }
    out.push_str(dry_run_prefix(written));
    let thread = if changed.thread.id == 0 {
        "a new thread".to_string()
    } else {
        format!("thread {}", changed.thread.id)
    };
    out.push_str(&format!("{did} {thread} in {name}"));
    if let Some(reply) = changed.reply.filter(|reply| *reply != 0) {
        out.push_str(&format!(" (reply {reply})"));
    }
    out.push_str(".\n");
    if let Some(anchored) = &changed.thread.anchored_text {
        out.push_str(&format!("  on: \"{}\"\n", one_line(anchored)));
    }
    out.push_str(&format!(
        "  {}: {}\n",
        changed.thread.author,
        indent_rest(&changed.thread.body, 4)
    ));
    for reply in &changed.thread.replies {
        out.push_str(&format!(
            "    reply {}  {}: {}\n",
            reply.id,
            reply.author,
            indent_rest(&reply.body, 6)
        ));
    }
    if let Some(snapshot) = &changed.snapshot {
        out.push_str(&format!(
            "The document as it was is kept as version {}; \"writer history restore {}\" brings it back.\n",
            snapshot.id.0, snapshot.id.0
        ));
    }
    written_suffix(written, &mut out);
    finish(out)
}

fn version_line(version: &VersionInfo) -> String {
    format!(
        "version {}  {}  {}  {}",
        version.id.0,
        time(version.created_at),
        plural(version.words, "word", "words"),
        match &version.label {
            Some(label) => label.clone(),
            None => "kept by the app".to_string(),
        }
    )
}

/// `writer history`.
pub fn versions(list: &VersionList) -> String {
    let name = document_name(&list.document);
    if list.versions.is_empty() {
        return format!("No versions are kept of {name}.\n");
    }
    let mut out = format!(
        "{} of {name}, newest first:\n",
        plural(list.versions.len() as u64, "version", "versions")
    );
    for version in &list.versions {
        out.push_str(&version_line(version));
        out.push('\n');
    }
    out
}

/// `writer history show`: the text and nothing else.
pub fn version_text(text: &VersionText) -> String {
    text.content.clone()
}

/// `writer history restore`.
pub fn version_restored(restored: &VersionRestored) -> String {
    let written = &restored.written;
    let name = document_name(&restored.document);
    let mut out = String::new();
    if !written.changed && !written.dry_run {
        out.push_str(&format!(
            "Nothing changed: {name} already has the text of version {}.\n",
            restored.restored.id.0
        ));
        return out;
    }
    out.push_str(dry_run_prefix(written));
    out.push_str(&format!(
        "Restored version {} ({}) of {name}: {} -> {} words.\n",
        restored.restored.id.0,
        time(restored.restored.created_at),
        count(restored.words_before),
        count(restored.words_after)
    ));
    if let Some(snapshot) = &restored.snapshot {
        out.push_str(&format!(
            "The text as it was is kept as version {}; \"writer history restore {}\" brings it back.\n",
            snapshot.id.0, snapshot.id.0
        ));
    }
    written_suffix(written, &mut out);
    finish(out)
}

/// `writer export`.
pub fn exported(exported: &Exported) -> String {
    let mut out = String::new();
    if exported.dry_run {
        out.push_str("Dry run: nothing was written. The real run would report:\n");
    }
    let format = match exported.format {
        ExportFormat::Markdown => "Markdown",
        ExportFormat::Docx => "DOCX",
        ExportFormat::PlainText => "plain text",
    };
    out.push_str(&format!(
        "Exported \"{}\" ({}) to {} ({format}, {} bytes).\n",
        exported.title,
        plural(exported.documents.len() as u64, "document", "documents"),
        exported.path.display(),
        count(exported.bytes)
    ));
    out
}

fn warnings(warnings: &[String], out: &mut String) {
    for warning in warnings {
        out.push_str(&format!("Note: {warning}\n"));
    }
}

/// Ends with exactly one line break.
fn finish(mut out: String) -> String {
    while out.ends_with('\n') {
        out.pop();
    }
    out.push('\n');
    out
}

/// A text on one line: line breaks become spaces, long text is cut.
fn one_line(text: &str) -> String {
    const MAX: usize = 200;
    let line = text.split_whitespace().collect::<Vec<_>>().join(" ");
    if line.chars().count() <= MAX {
        return line;
    }
    let cut: String = line.chars().take(MAX).collect();
    format!("{cut}...")
}

/// A text whose lines after the first are indented by `indent` spaces.
fn indent_rest(text: &str, indent: usize) -> String {
    let pad = " ".repeat(indent);
    text.trim_end()
        .lines()
        .collect::<Vec<_>>()
        .join(&format!("\n{pad}"))
}

/// A measure without a needless fraction: `72`, `1.6`.
fn number(value: f32) -> String {
    let rounded = (f64::from(value) * 100.0).round() / 100.0;
    if rounded.fract() == 0.0 {
        format!("{rounded:.0}")
    } else {
        format!("{rounded}")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn counts_and_plurals() {
        assert_eq!(count(0), "0");
        assert_eq!(count(999), "999");
        assert_eq!(count(1000), "1,000");
        assert_eq!(count(1_234_567), "1,234,567");
        assert_eq!(plural(1, "word", "words"), "1 word");
        assert_eq!(plural(2400, "word", "words"), "2,400 words");
    }

    #[test]
    fn times_are_cut_to_the_minute() {
        assert_eq!(time(Time(1_709_208_000_000)), "2024-02-29T12:00Z");
    }

    #[test]
    fn small_helpers() {
        assert_eq!(one_line("a\n  b\tc"), "a b c");
        assert_eq!(one_line(&"x".repeat(300)).chars().count(), 203);
        assert_eq!(indent_rest("a\nb\n", 2), "a\n  b");
        assert_eq!(number(72.0), "72");
        assert_eq!(number(1.6), "1.6");
        assert_eq!(finish("a\n\n".into()), "a\n");
    }
}
