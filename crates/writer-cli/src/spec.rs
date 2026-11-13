//! The command table: every command of `writer` with its arguments, options, help text
//! and examples. The parser ([`crate::parse`]) and the help ([`crate::help`]) are both
//! generated from it, so the help cannot promise an option the parser does not read.

/// The sections of `writer --help`, in order.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Group {
    Library,
    Documents,
    Items,
    Comments,
    History,
    Agents,
}

impl Group {
    /// Every section, in the order the overview prints them.
    pub const ALL: [Group; 6] = [
        Group::Library,
        Group::Documents,
        Group::Items,
        Group::Comments,
        Group::History,
        Group::Agents,
    ];

    /// The heading of the section.
    pub fn heading(self) -> &'static str {
        match self {
            Group::Library => "Look around",
            Group::Documents => "Documents (text goes in and comes out as Markdown)",
            Group::Items => "Documents and folders",
            Group::Comments => "Comments (to review a text instead of rewriting it)",
            Group::History => {
                "History (a version is kept before this tool changes the text of a document)"
            }
            Group::Agents => "Agents",
        }
    }
}

/// An argument of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Positional {
    pub name: &'static str,
    pub help: &'static str,
    pub required: bool,
}

/// An option of a command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Opt {
    /// The name without the dashes.
    pub name: &'static str,
    /// What the value is called in the help; `None` for a switch.
    pub value: Option<&'static str>,
    pub help: &'static str,
}

/// Whether a command takes text (`--content`, `--file`, `--stdin`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Content {
    /// It takes none.
    None,
    /// It may be given; standard input is read only when asked for.
    Optional,
    /// It must be given; without an option the text is read from standard input when
    /// that is a pipe or a file.
    Required,
}

/// One command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Spec {
    /// The name, two words for the command of a group (`comment add`).
    pub name: &'static str,
    /// Other first words that mean the same command.
    pub aliases: &'static [&'static str],
    pub group: Group,
    /// What follows the name in the overview and in the usage line.
    pub synopsis: &'static str,
    /// What follows the name in the usage line, when that is more than the overview has
    /// room for.
    pub long_synopsis: &'static str,
    /// One line for the overview.
    pub summary: &'static str,
    /// What it does, for the command's own help.
    pub about: &'static str,
    pub positionals: &'static [Positional],
    pub options: &'static [Opt],
    pub content: Content,
    /// Whether it changes the library (and so is carried out by the app while that is open).
    pub writes: bool,
    /// Whether `--dry-run` does something for it.
    pub dry_run: bool,
    /// Real command lines.
    pub examples: &'static [&'static str],
}

const fn arg(name: &'static str, help: &'static str) -> Positional {
    Positional {
        name,
        help,
        required: true,
    }
}

const fn optional(name: &'static str, help: &'static str) -> Positional {
    Positional {
        name,
        help,
        required: false,
    }
}

const fn value(name: &'static str, value: &'static str, help: &'static str) -> Opt {
    Opt {
        name,
        value: Some(value),
        help,
    }
}

const fn switch(name: &'static str, help: &'static str) -> Opt {
    Opt {
        name,
        value: None,
        help,
    }
}

const DOCUMENT: &str =
    "A document: its id (12 or doc:12) or its path in the library (\"Book/Part one/Chapter 1\")";
const FOLDER: &str =
    "A folder: its id (3 or folder:3) or its path in the library (\"Book/Part one\")";
const ITEM: &str = "A document or a folder: an id (doc:12, folder:3) or a path in the library";
const THREAD: &str = "A comment thread: its id as \"writer comments\" prints it";
const VERSION: &str = "A version: its id as \"writer history\" prints it";
const VERSION_DOCUMENT: &str = "The document the version belongs to (a check; may be left out)";
const AGENT: &str = "An agent by its id as \"writer agents\" prints it (claude-code, codex, ...)";

/// The options every command with text has.
pub const CONTENT_OPTIONS: [Opt; 3] = [
    value("content", "markdown", "The text, as an argument"),
    value(
        "file",
        "path",
        "The text, from a file (\"-\" is standard input)",
    ),
    switch("stdin", "The text, from standard input"),
];

const FORMAT_READ: Opt = value(
    "format",
    "markdown|text|json",
    "markdown (default), text (no markup) or json (the document as the app stores it)",
);
const OCCURRENCE_HEADING: Opt = value(
    "occurrence",
    "n",
    "Which heading, when several have that text: its number among them, from 1",
);
const FIRST: Opt = switch("first", "Put it before its siblings instead of after them");
const BEFORE: Opt = value(
    "before",
    "sibling",
    "Put it directly before this item of the destination folder",
);
const AFTER: Opt = value(
    "after",
    "sibling",
    "Put it directly after this item of the destination folder",
);
const CREATE_FOLDERS: Opt = switch(
    "create-folders",
    "Create the folder path when it does not exist yet",
);

/// A command that only reads the library.
const fn reading(name: &'static str, group: Group) -> Spec {
    Spec {
        name,
        aliases: &[],
        group,
        synopsis: "",
        long_synopsis: "",
        summary: "",
        about: "",
        positionals: &[],
        options: &[],
        content: Content::None,
        writes: false,
        dry_run: false,
        examples: &[],
    }
}

/// A command that changes the library.
const fn writing(name: &'static str, group: Group) -> Spec {
    Spec {
        writes: true,
        dry_run: true,
        ..reading(name, group)
    }
}

/// Every command, in the order of the overview.
pub const COMMANDS: &[Spec] = &[
    // ----- Look around ---------------------------------------------------------------
    Spec {
        aliases: &["ls", "tree"],
        synopsis: "[<folder>]",
        summary: "Folders and documents as a tree, with their ids (--trash: the trash)",
        about: "Prints the library as a tree: every folder and every document with its id, and for a \
document its word count and when it was last changed. With a folder, only what is in it. With \
--trash, what is in the trash instead.",
        positionals: &[optional("folder", FOLDER)],
        options: &[
            value(
                "depth",
                "n",
                "How many levels of folders to go into (0: only what is directly there)",
            ),
            switch("folders", "Leave the documents out"),
            switch(
                "trash",
                "List the trash instead: what \"writer restore\" can bring back",
            ),
        ],
        examples: &[
            "writer list",
            "writer list \"The Salt Road/Part one\" --depth 0",
            "writer list --trash --json",
        ],
        ..reading("list", Group::Library)
    },
    Spec {
        aliases: &["grep"],
        synopsis: "<query>",
        summary: "Full-text search in titles and text, with snippets",
        about: "Searches the titles and the text of every document. Each hit names the document (id, \
path) and shows the words around the match. Every word must occur; \"quoted words\" must occur \
as that phrase; the last word also finds words that begin with it.",
        positionals: &[arg(
            "query",
            "The words to look for (one argument: quote it)",
        )],
        options: &[
            value("limit", "n", "How many hits at most (default: 20)"),
            value(
                "in",
                "folder",
                "Only documents in this folder, at any depth",
            ),
        ],
        examples: &[
            "writer search lighthouse",
            "writer search '\"salt road\" storm' --limit 5 --json",
        ],
        ..reading("search", Group::Library)
    },
    Spec {
        summary: "Documents by when they were last changed",
        about: "Prints the documents that were changed last, the newest first.",
        options: &[value(
            "limit",
            "n",
            "How many documents at most (default: 10)",
        )],
        examples: &["writer recent", "writer recent --limit 3 --json"],
        ..reading("recent", Group::Library)
    },
    Spec {
        aliases: &["stat"],
        synopsis: "<item>",
        summary: "One document or folder: path, counts, dates, headings, settings",
        about: "Prints what the library knows about a document (id, folder, words and characters, when \
it was created and last changed, its headings, how many comment threads and versions it has, \
its page and type settings) or about a folder (id, path, what is in it).",
        positionals: &[arg("item", ITEM)],
        examples: &[
            "writer info \"The Salt Road/Part one/Chapter 1\"",
            "writer info doc:12 --json",
        ],
        ..reading("info", Group::Library)
    },
    Spec {
        summary: "Where the library is, and whether The Writer is open",
        about: "Prints the data directory, how many documents and folders the library has, and whether \
the app has it open right now. While it does, the commands that read work, and a change is carried \
out by the app, where the user sees it at once; the status says when the app takes no changes, \
and which of the two it is: they are switched off in its Agents panel, or it is a version before \
0.6.0 (with --json: \"app_takes_changes\", and \"app_refuses\" with the reason a change would be \
refused for). The result of every command that writes has \"app_is_open\" and \
\"app_takes_changes\" too: true when the app made the change, false when the tool wrote to the \
library itself.",
        examples: &["writer status", "writer status --json"],
        ..reading("status", Group::Library)
    },
    // ----- Documents -----------------------------------------------------------------
    Spec {
        aliases: &["cat", "read", "show"],
        synopsis: "<document>",
        summary: "Print a document (--format markdown|text|json, --section <heading>)",
        about: "Prints the text of a document: as Markdown (headings, lists, emphasis, links, code), as \
plain text, or as the JSON the app stores, which has everything (block ids, colours, fonts, \
comment anchors). Nothing but the text is printed, so the output can be piped or saved. With \
--section only the part under one heading is printed. An image is printed as a line \
![description](image:<id>); \"writer images\" lists the images of a document with their sizes.",
        positionals: &[arg("document", DOCUMENT)],
        options: &[
            FORMAT_READ,
            value(
                "section",
                "heading",
                "Only the section under this heading (named by its text)",
            ),
            OCCURRENCE_HEADING,
            switch("comments", "Print the comment threads after the text"),
        ],
        examples: &[
            "writer get \"The Salt Road/Part one/Chapter 1\"",
            "writer get doc:12 --format text | wc -w",
            "writer get doc:12 --section \"The harbour\" --comments",
        ],
        ..reading("get", Group::Documents)
    },
    Spec {
        synopsis: "<document> <text>",
        summary: "Where a piece of text stands in a document (what edit would match)",
        about: "Prints how many times a text occurs in a document and, for each occurrence, its number, \
the heading it is under and the line it is on. The text is matched the way \"writer edit\" and \
\"writer comment add\" match theirs, so this is the way to check that a text occurs once, or \
to pick an occurrence, before changing anything.",
        positionals: &[
            arg("document", DOCUMENT),
            arg("text", "The text to find, exactly as it stands"),
        ],
        options: &[value(
            "limit",
            "n",
            "How many occurrences to print at most (default: 20)",
        )],
        examples: &["writer find doc:12 \"the keeper\""],
        ..reading("find", Group::Documents)
    },
    Spec {
        aliases: &["new"],
        synopsis: "[<title>]",
        summary: "New document from Markdown (--in <folder>)",
        about: "Creates a document, at the top level of the library or in a folder, and prints its id. \
The text is Markdown; without --content, --file or --stdin the document is empty. Without a \
title the title follows the text (the first heading, else the first line). The folder must \
exist unless --create-folders is given.",
        positionals: &[optional("title", "The title of the new document")],
        options: &[
            value(
                "in",
                "folder",
                "The folder to create it in (default: the top level)",
            ),
            CREATE_FOLDERS,
            FIRST,
            BEFORE,
            AFTER,
        ],
        content: Content::Optional,
        examples: &[
            "writer create \"Chapter 4\" --in \"The Salt Road/Part one\" --file chapter4.md",
            "writer create \"Ideas\" --content \"A lighthouse keeper who cannot swim.\"",
            "printf '# Reading list\\n\\n- The Rings of Saturn\\n' | writer create --in Notes --stdin",
        ],
        ..writing("create", Group::Documents)
    },
    Spec {
        aliases: &["write"],
        synopsis: "<document>",
        summary: "Replace the whole text of a document",
        about: "Replaces the text of the document with the given Markdown. Paragraphs that read the same \
are left as they are, with their formatting and comments; still, to change part of a text, \
\"writer edit\", \"writer replace-section\" and \"writer append\" are the better commands. An \
image stays, with its size, where its line ![description](image:<id>) stays in the text, and is \
removed where the line is left out. The text before the change is kept as a version.",
        positionals: &[arg("document", DOCUMENT)],
        content: Content::Required,
        examples: &[
            "writer set doc:12 --file rewritten.md",
            "writer get doc:12 | sed 's/colour/color/g' | writer set doc:12",
        ],
        ..writing("set", Group::Documents)
    },
    Spec {
        synopsis: "<document> --old <text> --new <text>",
        summary: "Replace an exact piece of text; the rest keeps its formatting",
        about: "Replaces --old with --new in the document. --old must be in the document exactly as \
given (words, punctuation, capitals) and exactly once, unless --all or --occurrence says \
otherwise: quote enough of the words around it. It is matched against the text without markup \
(as \"writer get --format text\" prints it); a piece copied from the Markdown is found too. \
--new is Markdown: plain words take the formatting of what they replace, an empty --new \
deletes. Formatting and comments outside the replaced text stay as they are. An --old that runs \
across an image is refused, so that no image goes unnoticed: replace the text before and after \
it separately, or give --old as Markdown with the line of the image in it.",
        positionals: &[arg("document", DOCUMENT)],
        options: &[
            value(
                "old",
                "text",
                "The text to replace, exactly as it stands in the document",
            ),
            value("new", "text", "The text to put in its place (\"\" deletes)"),
            value(
                "old-file",
                "path",
                "Read --old from a file instead (\"-\" is standard input)",
            ),
            value(
                "new-file",
                "path",
                "Read --new from a file instead (\"-\" is standard input)",
            ),
            switch("all", "Replace every occurrence"),
            value(
                "occurrence",
                "n",
                "Replace only this occurrence: its number, from 1 (see \"writer find\")",
            ),
            switch(
                "literal",
                "Read no Markdown in --old and --new: they are the characters they are",
            ),
        ],
        examples: &[
            "writer edit doc:12 --old \"teh lighthouse\" --new \"the lighthouse\"",
            "writer edit \"Notes/Plan\" --old colour --new color --all --dry-run",
        ],
        ..writing("edit", Group::Documents)
    },
    Spec {
        synopsis: "<document>",
        summary: "Add text at the end of a document",
        about: "Adds the given Markdown after the last paragraph of the document.",
        positionals: &[arg("document", DOCUMENT)],
        content: Content::Required,
        examples: &[
            "writer append \"Notes/Journal\" --content \"Wrote the storm scene today.\"",
            "writer append doc:12 --file scene.md",
        ],
        ..writing("append", Group::Documents)
    },
    Spec {
        synopsis: "<document> <place>",
        long_synopsis: "<document> (--at-start | --before <heading> | --after <heading> | --end-of <heading>)",
        summary: "Add text: --at-start, or --before, --after or --end-of <heading>",
        about: "Adds the given Markdown at one place in the document: at the very start, directly before \
a heading, directly after a heading line (before the text under it), or at the end of the \
section under a heading. A heading is named by its text; \"writer info\" lists the headings \
of a document.",
        positionals: &[arg("document", DOCUMENT)],
        options: &[
            switch("at-start", "Insert before everything else"),
            value("before", "heading", "Insert directly before this heading"),
            value(
                "after",
                "heading",
                "Insert directly after this heading line",
            ),
            value(
                "end-of",
                "heading",
                "Insert at the end of the section under this heading",
            ),
            OCCURRENCE_HEADING,
        ],
        content: Content::Required,
        examples: &[
            "writer insert doc:12 --after \"Chapter 2\" --content \"*Three days later.*\"",
            "writer insert doc:12 --at-start --file epigraph.md",
        ],
        ..writing("insert", Group::Documents)
    },
    Spec {
        synopsis: "<document> <heading>",
        summary: "Replace what stands under a heading",
        about: "Replaces the section under a heading with the given Markdown: everything below the \
heading down to the next heading of the same or a higher level. The heading itself stays, \
unless --with-heading is given. A heading is named by its text.",
        positionals: &[
            arg("document", DOCUMENT),
            arg(
                "heading",
                "The text of the heading whose section is replaced",
            ),
        ],
        options: &[
            OCCURRENCE_HEADING,
            switch(
                "with-heading",
                "Replace the heading line too; the text should then begin with the new one",
            ),
        ],
        content: Content::Required,
        examples: &["writer replace-section doc:12 \"Synopsis\" --file synopsis.md"],
        ..writing("replace-section", Group::Documents)
    },
    Spec {
        synopsis: "<document>",
        summary: "The images of a document: id, size, width, description",
        about: "Prints the images of a document in the order of its text: for each the line it is in \
Markdown (![description](image:<id>)), the size of its picture, how wide it is drawn (a share \
of the text column) and where it stands on its line. Also written \"writer images \
<document>\". To look at the pictures themselves, export the document as Markdown: they are \
written into a folder beside the file.",
        positionals: &[arg("document", DOCUMENT)],
        examples: &[
            "writer images doc:12",
            "writer image list \"The Salt Road/Part one/Chapter 1\" --json",
        ],
        ..reading("image list", Group::Documents)
    },
    Spec {
        synopsis: "<document> <file>",
        summary: "Add an image from a file (--alt, --width, --align, --at)",
        about: "Adds an image file to a document, as a block of its own: at the end, or where --at \
says. PNG, JPEG, GIF, HEIC, TIFF, WebP and BMP work; the file is read by this tool and stored \
in the library. Without --width the image has its own size on the page; it is never drawn \
wider than the text or taller than a page, and the result says the width it was given. It is \
centered unless --align says otherwise. In the Markdown of the document the image then reads \
as a line ![description](image:<id>): a text that is written back keeps the image where that \
line stays. The text before the change is kept as a version.",
        positionals: &[
            arg("document", DOCUMENT),
            arg(
                "file",
                "The image file (a relative path is taken from where the command runs)",
            ),
        ],
        options: &[
            value(
                "alt",
                "text",
                "What the image shows: for whoever cannot see it, and for the exports",
            ),
            value(
                "width",
                "percent",
                "How wide it is drawn: a share of the text column, 5 to 100",
            ),
            value(
                "align",
                "left|center|right",
                "Where it stands on its line (default: center)",
            ),
            value(
                "at",
                "place",
                "end (default), start, before-heading, after-heading or end-of-section",
            ),
            value(
                "heading",
                "heading",
                "The heading the place refers to, by its text (for the three places at a heading)",
            ),
            value(
                "heading-occurrence",
                "n",
                "Which heading, when several have that text: its number among them, from 1",
            ),
        ],
        examples: &[
            "writer image add doc:12 ~/Pictures/harbour.jpg --alt \"The harbour at dusk\"",
            "writer image add \"Notes/Trip\" map.png --width 50 --at after-heading --heading \"Day 2\" --author Claude",
        ],
        ..writing("image add", Group::Documents)
    },
    Spec {
        aliases: &["copy", "cp"],
        synopsis: "<document>",
        summary: "Copy a document, beside the original",
        about: "Makes a copy of the document (text, comments and settings; not its versions) and prints \
the id of the copy.",
        positionals: &[arg("document", DOCUMENT)],
        options: &[
            value(
                "title",
                "title",
                "The title of the copy (default: the original's, with \" copy\")",
            ),
            value(
                "in",
                "folder",
                "The folder for the copy (default: beside the original)",
            ),
        ],
        examples: &[
            "writer duplicate \"The Salt Road/Part one/Chapter 1\" --title \"Chapter 1 (second draft)\"",
        ],
        ..writing("duplicate", Group::Documents)
    },
    Spec {
        synopsis: "<item> --to <path>",
        summary: "Write a document or a whole folder as Markdown, DOCX or text",
        about: "Writes a document to a file, or every document of a folder to one file in the order of \
the library (\"/\" is the whole library). The format is taken from the ending of the path (.md, \
.docx, .txt) unless --format names it. A file that exists is not replaced unless --overwrite is \
given. Exporting only reads the library, so it works while the app is open. The images of a \
Markdown export are written as files into a folder beside it (\"chapter images\" for \
chapter.md) and linked from the text, which is also the way to look at a picture; a DOCX holds \
its pictures.",
        positionals: &[arg("item", ITEM)],
        options: &[
            value("to", "path", "The file to write"),
            value(
                "format",
                "markdown|docx|text",
                "The format (default: from the ending of the path)",
            ),
            switch("overwrite", "Replace the file when it exists"),
        ],
        dry_run: true,
        examples: &[
            "writer export \"The Salt Road\" --to ~/Desktop/salt-road.docx",
            "writer export doc:12 --to chapter1.md --overwrite",
        ],
        ..reading("export", Group::Documents)
    },
    // ----- Documents and folders -----------------------------------------------------
    Spec {
        synopsis: "<path>",
        summary: "Create a folder, with the folders above it",
        about: "Creates the folder at this path in the library, and every folder on the way that does \
not exist yet. Prints the id of the folder. A folder that is already there is not an error.",
        positionals: &[arg(
            "path",
            "The path of the folder in the library, parts separated by /",
        )],
        options: &[value(
            "in",
            "folder",
            "The folder the path starts in (default: the top level)",
        )],
        examples: &["writer mkdir \"The Salt Road/Part two\""],
        ..writing("mkdir", Group::Items)
    },
    Spec {
        synopsis: "<item> <name>",
        summary: "Give a document or a folder a new name",
        about: "Changes the title of a document or the name of a folder. It stays where it is; \"writer \
move\" puts it somewhere else. For a document, --follow-text gives the title back to the text \
instead: it becomes the first heading or the first line and follows them from then on.",
        positionals: &[arg("item", ITEM), optional("name", "The new title or name")],
        options: &[switch(
            "follow-text",
            "For a document: let the title follow the text again (no <name>)",
        )],
        examples: &[
            "writer rename doc:12 \"Chapter 1: The Harbour\"",
            "writer rename \"Notes/Old\" Archive",
        ],
        ..writing("rename", Group::Items)
    },
    Spec {
        aliases: &["mv"],
        synopsis: "<item> [<folder>]",
        summary: "Move a document or a folder into a folder (\"/\" is the top level)",
        about: "Moves a document or a folder (with everything in it) into another folder; \"/\" stands \
for the top level of the library. With --first, --before or --after it is also given a place \
among its siblings; without <folder> only that place changes.",
        positionals: &[
            arg("item", ITEM),
            optional(
                "folder",
                "The folder to move it into, or \"/\" for the top level",
            ),
        ],
        options: &[CREATE_FOLDERS, FIRST, BEFORE, AFTER],
        examples: &[
            "writer move doc:12 \"The Salt Road/Part two\"",
            "writer move \"Notes/Old\" /",
            "writer move doc:14 --after doc:12",
        ],
        ..writing("move", Group::Items)
    },
    Spec {
        aliases: &["delete", "rm"],
        synopsis: "<item>",
        summary: "Move a document or a folder to the trash",
        about: "Moves a document, or a folder with everything in it, to the trash of the library, from \
where \"writer restore\" brings it back; nothing is lost. With --permanently the item is erased \
for good instead, with every version of it (one in the trash is looked for first); that cannot \
be taken back, and the MCP server does not offer it.",
        positionals: &[arg("item", ITEM)],
        options: &[switch(
            "permanently",
            "Erase the item for good instead; there is no way back",
        )],
        examples: &[
            "writer trash \"Notes/Old plan\"",
            "writer trash doc:12 --dry-run",
            "writer list --trash",
        ],
        ..writing("trash", Group::Items)
    },
    Spec {
        aliases: &["untrash"],
        synopsis: "<item>",
        summary: "Bring a document or a folder back from the trash",
        about: "Puts a trashed document or folder back where it was (at the top level when its folder is \
gone). \"writer list --trash\" shows what is in the trash, with ids.",
        positionals: &[arg(
            "item",
            "A trashed document or folder: its id (doc:12, folder:3) or its name",
        )],
        examples: &["writer restore doc:12"],
        ..writing("restore", Group::Items)
    },
    Spec {
        synopsis: "--permanently",
        summary: "Erase everything in the trash for good",
        about: "Erases every document and folder in the trash, with their versions. That cannot be taken \
back, so nothing happens without --permanently; --dry-run says what would be erased.",
        options: &[switch(
            "permanently",
            "The confirmation; without it nothing is erased",
        )],
        examples: &[
            "writer empty-trash --permanently --dry-run",
            "writer empty-trash --permanently",
        ],
        ..writing("empty-trash", Group::Items)
    },
    // ----- Comments ------------------------------------------------------------------
    Spec {
        synopsis: "<document>",
        summary: "Comment threads with the text they are anchored to",
        about: "Prints the comment threads of a document: the id of each, the text it is anchored to, \
whether it is resolved, and every comment with its author. Also written \"writer comments \
<document>\".",
        positionals: &[arg("document", DOCUMENT)],
        options: &[switch("open", "Leave out the resolved threads")],
        examples: &[
            "writer comments \"The Salt Road/Part one/Chapter 1\"",
            "writer comments doc:12 --open --json",
        ],
        ..reading("comment list", Group::Comments)
    },
    Spec {
        synopsis: "<document> --quote <text> --body <text>",
        summary: "Comment on an exact quote of the text",
        about: "Starts a comment thread anchored to a piece of the text. --quote is matched the way \
\"writer edit\" matches --old and must occur exactly once, unless --occurrence picks one: \
quote enough to make it unique. The thread shows up in the comments panel of the app, signed \
with the name given with --author.",
        positionals: &[arg("document", DOCUMENT)],
        options: &[
            value(
                "quote",
                "text",
                "The text to anchor the comment to, exactly as it stands",
            ),
            value("body", "text", "What the comment says"),
            value(
                "occurrence",
                "n",
                "Which occurrence of the quote: its number, from 1 (see \"writer find\")",
            ),
        ],
        examples: &[
            "writer comment add doc:12 --quote \"the keeper could not swim\" --body \"Set this up earlier?\" --author Claude",
        ],
        ..writing("comment add", Group::Comments)
    },
    Spec {
        synopsis: "<document> <thread> --body <text>",
        summary: "Answer in a thread",
        about: "Adds a reply to a comment thread, signed with the name given with --author.",
        positionals: &[arg("document", DOCUMENT), arg("thread", THREAD)],
        options: &[value("body", "text", "What the reply says")],
        examples: &["writer comment reply doc:12 3 --body \"Done in chapter 1.\" --author Claude"],
        ..writing("comment reply", Group::Comments)
    },
    Spec {
        synopsis: "<document> <thread>",
        summary: "Mark a thread as resolved",
        about: "Marks a comment thread as resolved. It stays in the document and can be reopened.",
        positionals: &[arg("document", DOCUMENT), arg("thread", THREAD)],
        examples: &["writer comment resolve doc:12 3"],
        ..writing("comment resolve", Group::Comments)
    },
    Spec {
        synopsis: "<document> <thread>",
        summary: "Open a resolved thread again",
        about: "Marks a resolved comment thread as open again.",
        positionals: &[arg("document", DOCUMENT), arg("thread", THREAD)],
        examples: &["writer comment reopen doc:12 3"],
        ..writing("comment reopen", Group::Comments)
    },
    Spec {
        synopsis: "<document> <thread>",
        summary: "Remove a thread (or one reply of it)",
        about: "Removes a comment thread with all its replies, or with --reply only one reply. The text \
the thread was about is not changed. The document as it was is kept as a version.",
        positionals: &[arg("document", DOCUMENT), arg("thread", THREAD)],
        options: &[value("reply", "id", "Remove only this reply of the thread")],
        examples: &["writer comment delete doc:12 3"],
        ..writing("comment delete", Group::Comments)
    },
    // ----- History -------------------------------------------------------------------
    Spec {
        synopsis: "<document>",
        summary: "The saved versions of a document, newest first",
        about: "Prints the versions the library keeps of a document, the newest first: the id of each, \
when it was kept, its size and, for one kept before a change by an agent, the label that says \
so. Also written \"writer history <document>\".",
        positionals: &[arg("document", DOCUMENT)],
        examples: &["writer history doc:12"],
        ..reading("history list", Group::History)
    },
    Spec {
        synopsis: "[<document>] <version>",
        summary: "Print a saved version",
        about: "Prints the text of a saved version, as \"writer get\" prints a document. The id of a \
version is enough; with the document named too, the version must be one of that document.",
        positionals: &[
            optional("document", VERSION_DOCUMENT),
            arg("version", VERSION),
        ],
        options: &[FORMAT_READ],
        examples: &[
            "writer history show 87",
            "writer history show doc:12 87 --format text",
        ],
        ..reading("history show", Group::History)
    },
    Spec {
        synopsis: "[<document>] <version>",
        summary: "Put a saved version back",
        about: "Makes a saved version the text of its document again. The text as it is now is kept as a \
version first, so restoring can itself be taken back. The id of a version is enough; with the \
document named too, the version must be one of that document, or nothing is restored.",
        positionals: &[
            optional("document", VERSION_DOCUMENT),
            arg("version", VERSION),
        ],
        examples: &[
            "writer history restore 87",
            "writer history restore doc:12 87 --dry-run",
        ],
        ..writing("history restore", Group::History)
    },
    // ----- Agents --------------------------------------------------------------------
    Spec {
        summary: "The agents found on this Mac and the state of the skill for each",
        about: "Prints every agent this tool knows about: whether it is found on this Mac, where its \
skills live, and whether the skill for The Writer is installed there and current. Also written \
\"writer agents\".",
        examples: &["writer agents", "writer agents --json"],
        ..reading("agents list", Group::Agents)
    },
    Spec {
        synopsis: "<agent> | --all",
        summary: "Install the skill that tells an agent about this tool",
        about: "Writes the skill (one file, the-writer/SKILL.md) into the skills directory of one agent, \
or with --all of every agent found on this Mac. A skill that is already there is brought up to \
date. A folder of that name that this tool did not write is left alone.",
        positionals: &[optional("agent", AGENT)],
        options: &[switch("all", "Every agent found on this Mac")],
        dry_run: true,
        examples: &[
            "writer agents install claude-code",
            "writer agents install --all --dry-run",
        ],
        ..reading("agents install", Group::Agents)
    },
    Spec {
        synopsis: "<agent> | --all",
        summary: "Bring an installed skill up to date",
        about: "Rewrites the skill where it is installed, for one agent or with --all everywhere this \
tool installed it. Nothing is installed where the skill is not.",
        positionals: &[optional("agent", AGENT)],
        options: &[switch("all", "Everywhere the skill is installed")],
        dry_run: true,
        examples: &["writer agents update --all"],
        ..reading("agents update", Group::Agents)
    },
    Spec {
        synopsis: "<agent> | --all",
        summary: "Remove the skill from an agent",
        about: "Deletes the skill from the skills directories one agent reads, or with --all from every \
directory this tool installed it in. Nothing else in those directories is touched. Agents that \
share a directory lose the skill together; the output names them.",
        positionals: &[optional("agent", AGENT)],
        options: &[switch("all", "Everywhere the skill is installed")],
        dry_run: true,
        examples: &["writer agents remove codex"],
        ..reading("agents remove", Group::Agents)
    },
    Spec {
        summary: "Print the skill",
        about: "Prints the skill file as it would be installed: what an agent is told about this tool.",
        examples: &["writer agents skill"],
        ..reading("agents skill", Group::Agents)
    },
    Spec {
        synopsis: "[<agent>]",
        summary: "How to add the MCP server to each agent",
        about: "Prints, for one agent or for all of them, the command or the piece of configuration that \
adds The Writer's MCP server (\"writer mcp\") to it, with the path of this binary filled in. \
Nothing is changed: run the command or paste the configuration yourself.",
        positionals: &[optional("agent", AGENT)],
        examples: &["writer agents mcp", "writer agents mcp claude-code --json"],
        ..reading("agents mcp", Group::Agents)
    },
    Spec {
        summary: "Run the MCP server on standard input and output",
        about: "Runs the Model Context Protocol server: the operations of this tool as tools an agent \
calls, over standard input and output (one JSON-RPC message per line), until the input ends. \
It is started by the agent, not by hand; \"writer agents mcp\" prints how to add it to each \
agent. With --library the server works on that library.",
        examples: &["writer mcp", "writer --library /tmp/test-library mcp"],
        ..reading("mcp", Group::Agents)
    },
];

/// The first words that open a group, with the group's own name (the first word of its
/// commands).
const GROUPS: [(&str, &str); 8] = [
    ("image", "image"),
    ("images", "image"),
    ("comment", "comment"),
    ("comments", "comment"),
    ("history", "history"),
    ("versions", "history"),
    ("agents", "agents"),
    ("agent", "agents"),
];

/// What a first word (and the word after it) names.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Found {
    /// A command; `true` when the second word was its subcommand.
    Command(&'static Spec, bool),
    Unknown,
}

fn by_name(name: &str) -> Option<&'static Spec> {
    COMMANDS.iter().find(|spec| spec.name == name)
}

/// Finds the command for the first word of a command line and the word after it. A group
/// (`comment`, `history`, `agents`) followed by one of its subcommands is that command;
/// followed by anything else it is the group's `list`.
pub fn find(first: &str, second: Option<&str>) -> Found {
    if let Some((_, group)) = GROUPS.iter().find(|(word, _)| *word == first) {
        if let Some(spec) = second.and_then(|second| by_name(&format!("{group} {second}"))) {
            return Found::Command(spec, true);
        }
        return match by_name(&format!("{group} list")) {
            Some(spec) => Found::Command(spec, false),
            None => Found::Unknown,
        };
    }
    match COMMANDS.iter().find(|spec| {
        !spec.name.contains(' ') && (spec.name == first || spec.aliases.contains(&first))
    }) {
        Some(spec) => Found::Command(spec, false),
        None => Found::Unknown,
    }
}

/// The options of a command: its own, then the ones for text when it takes text.
pub fn options(spec: &Spec) -> Vec<Opt> {
    let mut options = spec.options.to_vec();
    if spec.content != Content::None {
        options.extend(CONTENT_OPTIONS);
    }
    options
}

/// The option `name` of a command.
pub fn option(spec: &Spec, name: &str) -> Option<Opt> {
    options(spec).into_iter().find(|option| option.name == name)
}

/// The usage line of a command without its options for text.
pub fn usage_head(spec: &Spec) -> String {
    let synopsis = if spec.long_synopsis.is_empty() {
        spec.synopsis
    } else {
        spec.long_synopsis
    };
    if synopsis.is_empty() {
        format!("writer {}", spec.name)
    } else {
        format!("writer {} {synopsis}", spec.name)
    }
}

/// How the usage line of a command names its text and its options.
pub fn usage_tail(spec: &Spec) -> String {
    let mut tail = String::new();
    match spec.content {
        Content::None => {}
        Content::Optional => tail.push_str("[--content <markdown> | --file <path> | --stdin]"),
        Content::Required => tail.push_str("(--content <markdown> | --file <path> | --stdin)"),
    }
    if !options(spec).is_empty() || spec.dry_run {
        if !tail.is_empty() {
            tail.push(' ');
        }
        tail.push_str("[options]");
    }
    tail
}

/// The usage line of a command.
pub fn usage(spec: &Spec) -> String {
    let tail = usage_tail(spec);
    if tail.is_empty() {
        usage_head(spec)
    } else {
        format!("{} {tail}", usage_head(spec))
    }
}

/// The command name closest to a mistyped word, when one is close.
pub fn nearest(word: &str) -> Option<&'static str> {
    let mut names: Vec<&'static str> = Vec::new();
    for spec in COMMANDS {
        if !spec.name.contains(' ') {
            names.push(spec.name);
            names.extend(spec.aliases);
        }
    }
    names.extend(GROUPS.iter().map(|(word, _)| *word));
    let (name, distance) = names
        .into_iter()
        .map(|name| (name, edit_distance(word, name)))
        .min_by_key(|(_, distance)| *distance)?;
    // A typo or two, not another word.
    (distance <= 2 && distance < word.chars().count()).then(|| {
        // Name the command, not its alias.
        match find(name, None) {
            Found::Command(spec, _) if !GROUPS.iter().any(|(word, _)| *word == name) => spec.name,
            _ => name,
        }
    })
}

fn edit_distance(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, x) in a.iter().enumerate() {
        let mut previous = row[0];
        row[0] = i + 1;
        for (j, y) in b.iter().enumerate() {
            let current = row[j + 1];
            row[j + 1] = if x == y {
                previous
            } else {
                1 + previous.min(current).min(row[j])
            };
            previous = current;
        }
    }
    row[b.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_table_is_consistent() {
        let mut first_words: Vec<&str> = Vec::new();
        for spec in COMMANDS {
            assert!(
                !spec.summary.is_empty() && !spec.summary.ends_with('.'),
                "{}: summary",
                spec.name
            );
            assert!(
                spec.summary.chars().count() <= 72,
                "{}: summary too long for one line",
                spec.name
            );
            assert!(spec.about.ends_with('.'), "{}: about", spec.name);
            assert!(!spec.examples.is_empty(), "{} has no example", spec.name);
            for example in spec.examples {
                assert!(
                    example.contains("writer "),
                    "{}: example {example}",
                    spec.name
                );
            }
            let options = options(spec);
            for (index, option) in options.iter().enumerate() {
                assert!(
                    !["help", "version", "json", "dry-run", "library", "author"]
                        .contains(&option.name),
                    "{}: --{} is a global option",
                    spec.name,
                    option.name
                );
                assert!(
                    !options[..index]
                        .iter()
                        .any(|other| other.name == option.name),
                    "{}: --{} twice",
                    spec.name,
                    option.name
                );
            }
            if let Some((group, _)) = spec.name.split_once(' ') {
                assert!(
                    GROUPS.iter().any(|(_, name)| *name == group),
                    "{}: unknown group",
                    spec.name
                );
                assert!(
                    spec.aliases.is_empty(),
                    "{}: a subcommand has no alias",
                    spec.name
                );
            } else {
                first_words.push(spec.name);
                first_words.extend(spec.aliases);
            }
        }
        first_words.extend(GROUPS.iter().map(|(word, _)| *word));
        first_words.push("help");
        let mut sorted = first_words.clone();
        sorted.sort_unstable();
        sorted.dedup();
        assert_eq!(
            sorted.len(),
            first_words.len(),
            "a first word means two commands"
        );
        for (_, group) in GROUPS {
            assert!(
                by_name(&format!("{group} list")).is_some(),
                "{group} has no list"
            );
        }
    }

    #[test]
    fn nearest_names_a_command_for_a_typo_only() {
        assert_eq!(nearest("serch"), Some("search"));
        assert_eq!(nearest("staus"), Some("status"));
        assert_eq!(nearest("lst"), Some("list"));
        assert_eq!(nearest("coment"), Some("comment"));
        assert_eq!(nearest("apend"), Some("append"));
        assert_eq!(nearest("frobnicate"), None);
        assert_eq!(nearest("x"), None);
    }

    #[test]
    fn usage_lines() {
        let Found::Command(edit, _) = find("edit", None) else {
            panic!()
        };
        assert_eq!(
            usage(edit),
            "writer edit <document> --old <text> --new <text> [options]"
        );
        let Found::Command(append, _) = find("append", None) else {
            panic!()
        };
        assert_eq!(
            usage(append),
            "writer append <document> (--content <markdown> | --file <path> | --stdin) [options]"
        );
        let Found::Command(skill, _) = find("agents", Some("skill")) else {
            panic!()
        };
        assert_eq!(usage(skill), "writer agents skill");
    }
}
