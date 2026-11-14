//! The results of the command layer as text.
//!
//! One function per result type. The text is what `writer` prints and what a tool call
//! returns beside the structured data, so it is written for an agent reading it: complete
//! (every id needed for the next step is in it), compact, plain ASCII punctuation, no
//! decoration. Ids are written the way they can be passed back: `doc:12`, `folder:3`.

use writer_agent::live::{CHANGES_SWITCH, Refusal};
use writer_agent::{
    CommentChanged, CommentList, CommentThreadInfo, ContentChanged, DocumentChanged,
    DocumentCreated, DocumentInfo, DocumentList, DocumentSummary, DocumentText, ExportFormat,
    Exported, FindResults, FolderChanged, FolderCreated, FolderInfo, FolderSummary, ImageSummary,
    LibraryStatus, MatchedAs, PageAdded, PageOutcome, PageSummary, PagesList, SearchResults, Time,
    TrashEmptied, TrashItem, TrashList, Tree, TreeFolder, VersionInfo, VersionList,
    VersionRestored, VersionText, Written,
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
/// A page is `page:12 "Claude Pages/Orbitarium"`.
pub fn document_name(document: &DocumentSummary) -> String {
    let (kind, prefix) = if document.is_page() {
        ("page", "page")
    } else {
        ("document", "doc")
    };
    match document.id.0 {
        0 => format!("a new {kind} \"{}\"", document.path),
        id => format!("{prefix}:{id} \"{}\"", document.path),
    }
}

/// The reference of a document as it can be passed back: `doc:12`, or `page:12` for a
/// page.
pub fn reference(document: &DocumentSummary) -> String {
    if document.is_page() {
        format!("page:{}", document.id.0)
    } else {
        format!("doc:{}", document.id.0)
    }
}

/// What a line of a list says of a document's size: `1,200 words`, or `page` for a page
/// (its words are those of the text it shows, which nobody wrote here).
fn size_word(document: &DocumentSummary) -> String {
    if document.is_page() {
        "page".to_string()
    } else {
        plural(document.words, "word", "words")
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
        "{}  {}  ({}, changed {})",
        reference(document),
        document.path,
        size_word(document),
        time(document.updated_at)
    )
}

/// What a dry run says before the report of what the real run would do.
const DRY_RUN: &str = "Dry run: nothing was changed. The real run would report:\n";

/// What a write says about itself, in front of its report.
fn dry_run_prefix(written: &Written) -> &'static str {
    if written.dry_run { DRY_RUN } else { "" }
}

/// How the report of a request that has nothing to do begins (the text is that text
/// already, the item has that name or is in that place), or `None` when there is
/// something to do. A dry run says so as well: it reports what the real run would.
fn nothing_to_do(written: &Written) -> Option<&'static str> {
    if written.would_change || written.changed {
        None
    } else if written.dry_run {
        Some("Dry run: the real run would change nothing")
    } else {
        Some("Nothing changed")
    }
}

/// What a write says about itself, after everything else.
fn written_suffix(written: &Written, out: &mut String) {
    if written.dry_run && written.app_is_open {
        out.push_str(&if written.app_takes_changes {
            "\nThe Writer is open: the real run will be carried out by the app, where the user sees \
             the change at once."
                .to_string()
        } else {
            match no_changes(written.app_refuses.as_ref()) {
                Some((why, until)) => format!(
                    "\nThe Writer is open and {why}: the real run will be refused until {until}."
                ),
                None => "\nThe Writer is open and has only just started, so whether it takes \
                         changes from agents is not known yet: the real run is carried out by the \
                         app if it does, and refused if it is a version before 0.6.0."
                    .to_string(),
            }
        });
    }
}

/// What is said of an app that is open and takes no changes from agents, where the tool
/// knows which case it is: that it takes none and why, and what ends that. `None` where
/// it cannot be known (an app that has only just started names no channel yet, and a
/// version before 0.6.0 never does).
fn no_changes(refuses: Option<&Refusal>) -> Option<(&'static str, String)> {
    match refuses? {
        Refusal::SwitchedOff => Some((
            "takes no changes from agents (they are switched off in its Agents panel)",
            format!("the user switches on \"{CHANGES_SWITCH}\" there, or quits the app"),
        )),
        Refusal::OldVersion => Some((
            "takes no changes from agents while it runs (it is a version before 0.6.0, or it \
             could not open its channel)",
            "the user quits the app, or updates it".to_string(),
        )),
        _ => None,
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
        out.push_str(&if status.app_takes_changes {
            format!(
                "The Writer is open{pid}: reading works, and a change is carried out by the app, \
                 where the user sees it at once and can undo it.\n"
            )
        } else {
            match no_changes(status.app_refuses.as_ref()) {
                Some((why, until)) => format!(
                    "The Writer is open{pid} and {why}: reading works, every change is refused \
                     until {until}.\n"
                ),
                None => format!(
                    "The Writer is open{pid} and has only just started, so whether it takes \
                     changes from agents is not known yet (a version before 0.6.0 takes none): \
                     reading works; ask again in a few seconds.\n"
                ),
            }
        });
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
            "{indent}{}  {}  ({}, changed {})\n",
            document.title,
            reference(document),
            size_word(document),
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
    if let Some(page) = &info.page {
        return page_info(info, page);
    }
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

/// `writer info` for a page: what it is made of, where it came from, when it was taken.
fn page_info(info: &DocumentInfo, page: &PageSummary) -> String {
    let document = &info.document;
    let mut out = String::new();
    out.push_str(&format!("{}  {}\n", reference(document), document.path));
    out.push_str(&format!(
        "title:     {} ({})\n",
        document.title,
        if info.custom_title {
            "chosen by hand"
        } else {
            "the page's own"
        }
    ));
    out.push_str(
        "kind:      a page: a web page kept as it was saved and shown as it was made; its \
         text cannot be changed\n",
    );
    match document.folder_id {
        Some(folder) => out.push_str(&format!(
            "folder:    {}  folder:{}\n",
            document.folder, folder.0
        )),
        None => out.push_str("folder:    (the top level of the library)\n"),
    }
    out.push_str(&format!(
        "source:    {}\n",
        page.source_url.as_deref().unwrap_or("(none was given)")
    ));
    out.push_str(&format!("opens:     {}\n", page.entry));
    out.push_str(&format!(
        "files:     {}\n",
        page_size(page.files, page.bytes, page.externals)
    ));
    out.push_str(&format!("taken:     {}\n", time(page.content_at)));
    out.push_str(&format!("checked:   by {}\n", page.engine));
    out.push_str(&format!(
        "text:      {}, {} ({} without spaces), {}\n",
        plural(document.words, "word", "words"),
        plural(document.characters, "character", "characters"),
        count(info.characters_no_spaces),
        plural(info.paragraphs, "paragraph", "paragraphs"),
    ));
    out.push_str(&format!("created:   {}\n", time(document.created_at)));
    out.push_str(&format!("changed:   {}\n", time(document.updated_at)));
    if let Some(trashed) = document.trashed_at {
        out.push_str(&format!("trashed:   {} (in the trash)\n", time(trashed)));
    }
    out.push_str(&format!("versions:  {}\n", info.versions));
    if page.warnings.is_empty() {
        out.push_str("warnings:  none\n");
    } else {
        out.push_str("warnings:\n");
        for warning in &page.warnings {
            out.push_str(&format!("  {}\n", whole_line(warning)));
        }
    }
    out
}

/// `30 files, 11.4 MB, 12 pieces from the web` (the last part only when there are any).
fn page_size(files: usize, bytes: u64, externals: usize) -> String {
    let mut out = format!(
        "{}, {}",
        plural(files as u64, "file", "files"),
        writer_page::megabytes(bytes)
    );
    if externals > 0 {
        out.push_str(&format!(
            ", {}",
            plural(
                externals as u64,
                "piece from the web",
                "pieces from the web"
            )
        ));
    }
    out
}

/// A page's name in a line about it: `page:41 “Reading Orbitarium”`, or `a new page
/// “Reading Orbitarium”` in a dry run.
fn page_name(page: &DocumentSummary) -> String {
    match page.id.0 {
        0 => format!("a new page \u{201c}{}\u{201d}", page.title),
        id => format!("page:{id} \u{201c}{}\u{201d}", page.title),
    }
}

/// `writer page add`, one page: what became of it, in one line, with its warnings under
/// it.
///
/// ```text
/// added page:41 “Reading Orbitarium” in Claude Pages (30 files, 11.4 MB, 12 pieces from the web)
/// unchanged page:12 “Orbitarium” (https://claude.ai/artifact/ExampleArtifactId00002)
/// updated page:13 “Nimbus Release Notes”: the page as it was is kept as version 88 (…)
/// trashed page:9 “Nimbus Brand Sheet” is in the trash, so it was not added again: restore it, or add it with --include-trashed
/// ```
pub fn page_added(added: &PageAdded) -> String {
    let written = &added.written;
    let mut out = String::from(dry_run_prefix(written));
    let name = page_name(&added.page);
    let size = page_size(added.files, added.bytes, added.externals);
    let mut line = match added.outcome {
        PageOutcome::Added => {
            let place = if added.page.folder.is_empty() {
                "at the top of the library".to_string()
            } else {
                format!("in {}", added.page.folder)
            };
            format!("added {name} {place} ({size})")
        }
        PageOutcome::Updated => match &added.snapshot {
            Some(snapshot) => format!(
                "updated {name}: the page as it was is kept as version {} ({size})",
                snapshot.id.0
            ),
            None => format!("updated {name} ({size})"),
        },
        PageOutcome::Unchanged => match &added.page.source_url {
            Some(url) => format!("unchanged {name} ({url})"),
            None => format!("unchanged {name}"),
        },
        PageOutcome::Trashed => format!(
            "trashed {name} is in the trash, so it was not added again: restore it, or add it \
             with --include-trashed"
        ),
    };
    match added.warnings.len() {
        0 => line.push('\n'),
        n => {
            line.push_str(&format!(
                ", with {}:\n",
                plural(n as u64, "warning", "warnings")
            ));
            for warning in &added.warnings {
                line.push_str(&format!("  {}\n", whole_line(warning)));
            }
        }
    }
    out.push_str(&line);
    written_suffix(written, &mut out);
    finish(out)
}

/// `writer pages`: one line a page, the trashed ones marked.
///
/// ```text
/// page:41  Claude Pages/Reading Orbitarium  https://claude.ai/artifact/ExampleArtifactId00001  taken 2026-11-13T21:04Z  30 files, 11.4 MB
/// ```
pub fn pages(list: &PagesList) -> String {
    if list.pages.is_empty() {
        return "No pages.\n".to_string();
    }
    let mut out = String::new();
    for listed in &list.pages {
        let page = &listed.page;
        let summary = &listed.summary;
        out.push_str(&format!(
            "{}  {}  {}  taken {}  {}",
            reference(page),
            page.path,
            summary
                .source_url
                .as_deref()
                .unwrap_or("(no source address)"),
            time(summary.content_at),
            page_size(summary.files, summary.bytes, 0)
        ));
        if listed.trashed {
            out.push_str("  (in the trash)");
        }
        out.push('\n');
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
                "  {}  {}  ({}, changed {})\n",
                document.title,
                reference(document),
                size_word(document),
                time(document.updated_at)
            ));
        }
    }
    out
}

/// `writer get`: the text and nothing else, unless the comments were asked for.
pub fn text(text: &DocumentText) -> String {
    let mut out = text.content.clone();
    // The text a page showed ends where it ends; printed, it ends with a line.
    if text.page.is_some() && !out.is_empty() && !out.ends_with('\n') {
        out.push('\n');
    }
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
        if occurrence.across_image {
            out.push_str(
                "  (runs across an image: an edit refuses it; name the image as it reads in \
                 Markdown, or change the text before and after it separately)",
            );
        }
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
    if let Some(nothing) = nothing_to_do(written) {
        out.push_str(&format!("{nothing}: {name} already reads that way.\n"));
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
    let mut images: Vec<String> = Vec::new();
    if changed.images_added > 0 {
        images.push(format!("{} added", changed.images_added));
    }
    if changed.images_removed > 0 {
        images.push(format!("{} removed", changed.images_removed));
    }
    if !images.is_empty() {
        facts.push(format!("images: {}", images.join(", ")));
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
    if let Some(image) = &changed.image {
        out.push_str(&format!("The image: {}\n", image_line(image)));
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
    if let Some(nothing) = nothing_to_do(written) {
        out.push_str(&format!(
            "{nothing}: {} is already so.\n",
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
    if let Some(nothing) = nothing_to_do(written) {
        out.push_str(&format!(
            "{nothing}: {} is already so.\n",
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
                "{}  {}  ({}, trashed {})\n",
                reference(document),
                document.path,
                size_word(document),
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
    if let Some(nothing) = nothing_to_do(written) {
        out.push_str(&format!(
            "{nothing}: thread {} in {name} is already so.\n",
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
    if let Some(nothing) = nothing_to_do(written) {
        out.push_str(&format!(
            "{nothing}: {name} already has the text of version {}.\n",
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
        ExportFormat::Html => "a web page folder",
        ExportFormat::Pdf => "PDF",
    };
    let page = match exported.documents.as_slice() {
        [document] if document.is_page() => Some(document),
        _ => None,
    };
    match page {
        Some(page) => {
            let files = if exported.format == ExportFormat::Html {
                format!("{}, ", plural(exported.files as u64, "file", "files"))
            } else {
                String::new()
            };
            out.push_str(&format!(
                "Exported {} to {} ({format}, {files}{} bytes).\n",
                document_name(page),
                exported.path.display(),
                count(exported.bytes)
            ));
            if exported.format == ExportFormat::Html && !exported.dry_run {
                out.push_str(
                    "Open its HTML file in a browser; a few pages need to be served (python3 -m \
                     http.server in the folder).\n",
                );
            }
        }
        None => out.push_str(&format!(
            "Exported \"{}\" ({}) to {} ({format}, {} bytes).\n",
            exported.title,
            plural(exported.documents.len() as u64, "document", "documents"),
            exported.path.display(),
            count(exported.bytes)
        )),
    }
    // The pictures a Markdown file links: where they are, so that they can be opened.
    if let Some(folder) = exported.assets.first().and_then(|file| file.parent()) {
        out.push_str(&format!(
            "Its {} in {}:\n",
            if exported.assets.len() == 1 {
                "picture is".to_string()
            } else {
                format!("{} pictures are", count(exported.assets.len() as u64))
            },
            folder.display()
        ));
        for file in &exported.assets {
            let name = file
                .file_name()
                .map(|name| name.to_string_lossy().into_owned())
                .unwrap_or_default();
            out.push_str(&format!("  {name}\n"));
        }
    }
    warnings(&exported.warnings, &mut out);
    out
}

/// One image in a sentence: how it reads in Markdown, how large the picture is, and
/// what Markdown does not say of it (how wide it is drawn, where it stands).
fn image_line(image: &ImageSummary) -> String {
    let place = match image.align.as_str() {
        "center" => "centered",
        "right" => "at the right",
        _ => "at the left",
    };
    // The line as the document's Markdown has it (the description escaped), so that the
    // line written back is this image; an app of a version that did not send it gets
    // the line as it was put together before.
    let markdown = if image.markdown.is_empty() {
        format!("![{}](image:{})", one_line(&image.description), image.id)
    } else {
        image.markdown.clone()
    };
    let mut line = format!("{markdown}, {} x {} pixels", image.pixels.0, image.pixels.1);
    if let Some(format) = &image.format {
        line.push_str(&format!(" ({})", format.to_ascii_uppercase()));
    }
    line.push_str(&format!(
        ", {} % of the text column wide, {place}",
        image.width_percent
    ));
    if image.missing {
        line.push_str("; the library does not have the picture itself");
    }
    line
}

/// The images of a document, one line each.
pub fn images(text: &DocumentText) -> String {
    let name = document_name(&text.document);
    if text.images.is_empty() {
        return format!("No images in {name}.\n");
    }
    let mut out = format!(
        "{} in {name}, in the order of the text:\n",
        plural(text.images.len() as u64, "image", "images")
    );
    for image in &text.images {
        out.push_str(&format!("  block {}  {}\n", image.block, image_line(image)));
    }
    out.push_str(
        "In the Markdown of the document each is the line shown here: keep it where the image \
         should stay.\n",
    );
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

/// A text on one line, whole: line breaks become spaces. For a sentence that has to be
/// read to its end (a warning).
fn whole_line(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
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

    /// An app that is open and takes no changes: the status and the note of a dry run
    /// say which case it is, and keep an "or" only for the one that cannot be known.
    #[test]
    fn an_app_that_takes_no_changes_is_said_to_be_the_case_it_is() {
        let said = |refuses: serde_json::Value| {
            let mut status = serde_json::json!({
                "data_dir": "/library", "database": "/library/library.db", "exists": true,
                "app_is_open": true, "app_pid": 7, "app_takes_changes": false,
                "documents": 1, "folders": 0, "trashed": 0, "version": "1",
            });
            let mut written =
                serde_json::json!({"dry_run": true, "changed": false, "app_is_open": true});
            if !refuses.is_null() {
                status["app_refuses"] = refuses.clone();
                written["app_refuses"] = refuses;
            }
            let status: LibraryStatus = serde_json::from_value(status).unwrap();
            let written: Written = serde_json::from_value(written).unwrap();
            let mut note = String::new();
            written_suffix(&written, &mut note);
            (super::status(&status), note)
        };
        let (status, note) = said(serde_json::json!({"why": "switched_off"}));
        assert!(
            status.contains(
                "The Writer is open (process 7) and takes no changes from agents (they are \
                 switched off in its Agents panel): reading works, every change is refused \
                 until the user switches on \"Agents can make changes while The Writer is \
                 open\" there, or quits the app.\n"
            ),
            "{status}"
        );
        assert!(!status.contains("0.6.0") && !note.contains("0.6.0"));
        assert!(note.starts_with("\nThe Writer is open and takes no changes from agents (they"));
        assert!(note.ends_with("there, or quits the app."), "{note}");

        let (status, note) = said(serde_json::json!({"why": "old_version"}));
        assert!(
            status.contains(
                "takes no changes from agents while it runs (it is a version before 0.6.0, or \
                 it could not open its channel): reading works, every change is refused until \
                 the user quits the app, or updates it.\n"
            ),
            "{status}"
        );
        assert!(!status.contains("switched off") && !note.contains("switched off"));
        assert!(
            note.ends_with("until the user quits the app, or updates it."),
            "{note}"
        );

        // An app that has only just started names no channel yet: that is the one case
        // that cannot be told from a version before 0.6.0.
        let (status, note) = said(serde_json::Value::Null);
        assert!(
            status.contains(
                "The Writer is open (process 7) and has only just started, so whether it \
                 takes changes from agents is not known yet (a version before 0.6.0 takes \
                 none): reading works; ask again in a few seconds.\n"
            ),
            "{status}"
        );
        assert!(note.contains("has only just started"), "{note}");
        assert!(!status.contains("switched off") && !note.contains("switched off"));
    }

    #[test]
    fn an_image_is_said_as_it_reads_and_with_what_markdown_does_not_say() {
        let image = ImageSummary {
            id: "3f2a9c1b5d6e7f80a1b2c3d4e5f60718".into(),
            block: 4,
            format: Some("jpeg".into()),
            pixels: (1600, 1067),
            width_percent: 50,
            align: "center".into(),
            description: "The harbour\nat dusk".into(),
            missing: false,
            // From an app that did not send the line: it is put together here.
            markdown: String::new(),
        };
        assert_eq!(
            image_line(&image),
            "![The harbour at dusk](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718), 1600 x 1067 pixels \
             (JPEG), 50 % of the text column wide, centered"
        );
        let missing = ImageSummary {
            format: None,
            align: "right".into(),
            description: String::new(),
            missing: true,
            ..image.clone()
        };
        assert_eq!(
            image_line(&missing),
            "![](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718), 1600 x 1067 pixels, 50 % of the text \
             column wide, at the right; the library does not have the picture itself"
        );
        assert!(image_line(&image).is_ascii());
        // The line the library gives is the one said, escaped as the document has it.
        let escaped = ImageSummary {
            description: "Fig. 3] *draft*".into(),
            markdown: "![Fig. 3\\] \\*draft\\*](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718)".into(),
            ..image.clone()
        };
        assert!(
            image_line(&escaped).starts_with(
                "![Fig. 3\\] \\*draft\\*](image:3f2a9c1b5d6e7f80a1b2c3d4e5f60718), 1600 x 1067"
            ),
            "{}",
            image_line(&escaped)
        );
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
