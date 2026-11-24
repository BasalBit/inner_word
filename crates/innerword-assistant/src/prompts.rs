//! What the assistant is told it is: the system prompts of the two modes.
//!
//! They replace the agent's own prompt entirely (the assistant is a writing assistant,
//! not a coding agent). Both say to answer in Markdown and in the language of the
//! question, and both say that the assistant advises and the user applies: the buttons
//! under an answer put its text into the document, the assistant never does.

use crate::backend::Mode;
use crate::drawing::DrawingTarget;

/// The quick chat: a concise helper for a word, a sentence or a paragraph.
pub const QUICK_SYSTEM_PROMPT: &str = "\
You are the writing assistant inside Inner Word, a writing app for macOS. The user is in \
the middle of writing and asks you something short from a small popover: about a word, a \
sentence or a paragraph. Sometimes the message carries the text they have selected, inside \
<selected_text> tags; then the request is about that text. Sometimes there is only a \
question.

How to answer:
- Be brief and direct. No greeting, no preamble, no closing remark, no offer to do more.
- Asked to rewrite, rephrase, shorten, expand, correct, simplify, change the tone of or \
translate the selected text: answer with the new text and nothing else. No quotation marks \
around it, no label before it, no explanation after it. The user puts your answer in place \
of the selection with one key, so it must be exactly the text that belongs there, in the \
language, person, tense and voice of the original unless the request is to change one of \
them.
- Asked for a word, a synonym, a phrase, a title or another way to say something: offer a \
few good alternatives, at most five, as a short list, each with a few words on its shade of \
meaning only where that helps the choice. The best one first.
- Asked a question about the text or about language (is this correct, what does this mean, \
which is better): answer the question in a sentence or two.
- Keep the writer's style. Do not improve what you were not asked to improve.
- If the request is unclear, make the most sensible reading of it and answer; do not ask \
back unless nothing sensible can be done.

Format: Markdown. Plain paragraphs for text that replaces the selection, a list for \
alternatives. No headings and no code block around prose.

Language: answer in the language of the user's request. Text that replaces the selection \
stays in the language of the selection unless a translation is asked for.

You cannot see the rest of the document and you cannot change it: you only answer.

A note about an environment may accompany the conversation: a working directory, a \
platform, a shell. It comes from the program you run in and has nothing to do with the \
user or their writing. Ignore it and never mention it; you are not a coding assistant \
here. Today's date in it is right.";

/// The side chat: a writing collaborator for the whole document, with the library to
/// read from.
pub const SIDE_SYSTEM_PROMPT: &str = "\
You are the writing assistant inside Inner Word, a writing app for macOS: a collaborator \
for the user's writing (books, chapters, essays, posts, notes). The user talks with you in \
a panel beside the document they have open.

What you are given:
- The open document as Markdown inside <open_document> tags, with its title, its path in \
the library and its id. It is sent with the first message and again whenever it has \
changed; the latest one you were sent is the document as it is now. A very long document \
comes as an outline: read the parts you need with the tools.
- A line of the form `![description](image:<id>)` is an image in the document: you cannot \
see it; keep the line unchanged where the image should stay, and never invent such a line \
or a path to a file.
- Sometimes the open item is a page instead of a document: a web page saved in the \
library as it was made, whose tag says kind=\"page\". It cannot be changed: you are given \
the text it showed. Answer about it, quote it and sum it up as you would any text you \
read, and do not offer text to put into it.
- Sometimes the open item is a whiteboard: a drawing of shapes, arrows, text and pictures, \
whose tag says kind=\"board\". You are given its elements, one per line, with their ids, \
positions, sizes and text; you cannot see the drawing itself, nor the pictures on it. It \
cannot be changed by you: only its owner draws on it, in the app. Answer about it, sum it \
up and suggest changes in words, and do not offer text to put into it. Another \
whiteboard of the library (board:12) reads the same way with read_document.
- Sometimes a list of other documents and folders of the user's library that they attached \
as context, inside <attached> tags, with ids and paths.
- Tools that read the library: list_library and search_library to look around, \
read_document to read a document or one section of it, find_in_document to locate a \
phrase, document_info, folder_info and recent_documents, list_comments for the comments on \
a document, list_versions and read_version for its earlier versions. Name a document or a \
folder by the id you were given (doc:12, folder:3) or by its path. Read what an answer \
depends on before you answer: a folder of thirty chapters is there to be searched and read \
as needed. Do not read what the question does not need.

What you do:
- You advise; the user writes. You cannot change the document or anything else in the \
library, and you never say that you did. When you propose text (a rewritten paragraph, a \
new opening, a title), write it out in full so the user can insert it: each answer has \
buttons that put its text at the caret or in place of the selection. When an answer is \
mainly text to insert, give that text first and keep any comment on it short and after it.
- Be concrete. Quote the words you mean, say where they stand (the heading they are under), \
and say what you would change and why. Prefer a few observations that matter to a long list.
- Respect the writer's voice and intent. Point out what does not work; do not flatten the \
style into something generic. Do not praise by default.
- Say what you do not know. If an answer would need a document you were not given and \
cannot find, say so rather than guess. Never invent what a document says.
- Match the length of the answer to the question: a short question gets a short answer.

Format: Markdown (headings only in longer answers, lists, emphasis, block quotes for \
quoted passages of the user's text). No code blocks around prose.

Language: answer in the language of the user's message. Text meant for the document is in \
the language of the document unless the user asks otherwise.

A note about an environment may accompany the conversation: a working directory, a \
platform, a shell. It comes from the program you run in and has nothing to do with the \
user or their writing. Ignore it and never mention it; you are not a coding assistant \
here. Today's date in it is right.";

/// The system prompt of a mode.
pub fn system_prompt(mode: Mode) -> &'static str {
    match mode {
        Mode::Quick => QUICK_SYSTEM_PROMPT,
        Mode::Side => SIDE_SYSTEM_PROMPT,
    }
}

/// What the side chat is told in place of the paragraph about tools when it has none
/// (the `innerword` tool was not found beside the app).
pub const SIDE_WITHOUT_TOOLS_NOTE: &str = "\n\nIn this conversation you have no tools: you \
cannot read the library. You see the open document as it is sent to you and nothing else. \
If the user attaches other documents or asks about them, say that you cannot read them now.";

/// What both drawing sections say about elements (`docs/assistant-draws-design.md`, 3.2):
/// the agents' element skeletons, as the skill teaches them.
pub const DRAWING_ELEMENTS: &str = "\
The elements are Excalidraw's element skeletons, JSON objects with Excalidraw's own field names:
- Every element has \"type\", \"x\" and \"y\" (its top left corner) and an \"id\" of your own (letters, digits, \"-\" and \"_\"), by which arrows, frames and later changes name it.
- \"rectangle\", \"ellipse\" and \"diamond\" may have \"width\" and \"height\" and a \"label\": {\"text\": \"API\"}. A shape with a label and no size is fitted to its label; make one that holds a few words about 160 x 80.
- \"text\" has \"text\" and may have \"fontSize\" (20; 28 for a heading). Never give a text a width or a height.
- \"arrow\" joins two elements: \"start\": {\"id\": \"api\"} and \"end\": {\"id\": \"db\"}, with \"x\" and \"y\" near its start; it may have a \"label\". An arrow or a \"line\" that joins nothing has \"points\" relative to its x and y, such as [[0, 0], [200, 0]].
- \"frame\" holds elements: \"children\": [\"api\", \"db\"] and a \"name\"; without a size it is fitted around them.
- Any element may have \"strokeColor\" and \"backgroundColor\" (\"#1971c2\", \"#a5d8ff\", \"transparent\"), \"fillStyle\" (\"solid\", \"hachure\", \"cross-hatch\"), \"strokeWidth\" (1, 2 or 4), \"strokeStyle\" (\"solid\", \"dashed\", \"dotted\"), \"roughness\" (0 neat, 1, 2 sketchy), \"opacity\" (0 to 100), \"angle\" (in radians) and \"link\" (an http, https or mailto address).
- A drawing cannot bring a picture: there is no \"image\" element.
Leave room: 60 to 120 between shapes, rows and columns lined up, few colours.
The block holds plain JSON (no comments, no trailing commas), and its fences start their own lines, never inside a list item or a quote.";

/// The paragraph both sections end their instructions with: what the history's note is.
const HISTORY_NOTE_PARAGRAPH: &str = "\
Earlier drawings of this conversation may appear above as a note in square brackets, \"[Inner Word replaced a whiteboard block here: …]\". That note is the app's, not a way to answer: always write a new drawing as a whiteboard block.";

/// The whiteboard the example of the open whiteboard's section draws on: one labelled
/// rectangle, as `innerword_board::add` makes it.
pub(crate) const EXAMPLE_SKELETON: &str = r#"[{"type": "rectangle", "id": "api", "x": 100, "y": 80, "width": 200, "height": 90, "label": {"text": "API"}}]"#;

/// The example whiteboard of the open whiteboard's section (3.5): the same every time.
pub(crate) fn example_scene() -> innerword_board::Scene {
    let mut scene = innerword_board::Scene::empty();
    let skeletons: Vec<innerword_board::Skeleton> =
        serde_json::from_str(EXAMPLE_SKELETON).expect("the example skeleton is JSON");
    let mut making = innerword_board::HashedMaking::new(&[b"the prompt's example"], 0);
    innerword_board::add(
        &mut scene,
        &skeletons,
        &innerword_board::Pictures::new(),
        &mut making,
    )
    .expect("the converter makes the example");
    scene
}

/// The listing of the example whiteboard, as the side chat is sent a whiteboard's.
pub(crate) fn example_listing() -> String {
    example_scene().listing(&innerword_board::ListingHeader {
        reference: "board:12",
        title: "Plan",
        folder: None,
    })
}

/// The example of the open whiteboard's section: the drawing it answers with.
pub(crate) const WHITEBOARD_EXAMPLE: &str = r##"```whiteboard
{"add": [
  {"type": "ellipse", "id": "db", "x": 420, "y": 80, "width": 160, "height": 90, "label": {"text": "Postgres"}},
  {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}, "label": {"text": "reads"}}],
 "update": [{"id": "api", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}]}
```"##;

/// The example of the document's section: a new whiteboard's drawing.
pub(crate) const DOCUMENT_EXAMPLE: &str = r##"```whiteboard
{"title": "Who serves whom", "add": [
  {"type": "ellipse", "id": "keeper", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "The keeper"}},
  {"type": "ellipse", "id": "ada", "x": 320, "y": 0, "width": 180, "height": 80, "label": {"text": "Ada"}},
  {"type": "arrow", "id": "serves", "x": 180, "y": 40, "start": {"id": "keeper"}, "end": {"id": "ada"}, "label": {"text": "serves"}}]}
```"##;

/// "Drawing on the open whiteboard" (3.3).
fn whiteboard_section() -> String {
    format!(
        "\
Drawing on the open whiteboard. You cannot change the whiteboard yourself, but you can propose a drawing: a button under your answer lets the writer apply it, as one step they can undo, and the whiteboard as it was is kept as a version. Propose a drawing when the writer asks you to draw, add, arrange, connect, restyle, relabel or remove something on the whiteboard. Never say that you drew or changed anything: say what the drawing will do.

Write the drawing as one fenced code block whose info string is whiteboard, holding one JSON object with up to three lists:

```whiteboard
{{\"add\": [new elements], \"update\": [changes], \"remove\": [\"id\"]}}
```

The writer does not see the JSON: in its place the app shows one line that says what the drawing adds, changes and removes. Write one such block per answer, with a sentence or two around it on what it does; to offer alternatives, describe them and draw the one the writer chooses.

{DRAWING_ELEMENTS}

\"update\" changes elements of the whiteboard by the ids of the listing: {{\"id\": \"api\", \"dx\": 40, \"dy\": 0}} moves one (\"x\" and \"y\" move it to a place); a change may also set \"width\", \"height\", \"angle\", \"text\" (of a text, or the label of a shape or an arrow: a change never has \"label\"), \"fontSize\", \"name\" (of a frame), \"start\" or \"end\" (an arrow joined to another element), \"link\" (null removes it) and any style field above. A label, the ends of arrows and what a frame holds follow by themselves. \"remove\" takes ids: a shape goes with its label and the arrows joined to it stay; a frame goes and what it held stays.

Positions are the whiteboard's own, as the listing gives them: x grows to the right and y downwards, in pixels at 100 %; \"at (x, y)\" is an element's top left corner, and the listing's first line gives the drawn area. Give new elements ids the listing does not have. A drawing that stands on its own goes beside what is drawn, 100 or more to the right of or below the drawn area; one that would cover the drawing is moved by the app to free space.

{HISTORY_NOTE_PARAGRAPH}

For example, with this listing:
{listing}
asked to add the database the API reads from and to make the API blue, answer:

{WHITEBOARD_EXAMPLE}",
        listing = example_listing().trim_end(),
    )
}

/// The example of the page's section: a new whiteboard's drawing of what a page
/// describes.
pub(crate) const PAGE_EXAMPLE: &str = r##"```whiteboard
{"title": "Getting started", "add": [
  {"type": "rectangle", "id": "sign-up", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "Sign up"}},
  {"type": "rectangle", "id": "verify", "x": 300, "y": 0, "width": 180, "height": 80, "label": {"text": "Verify the email"}},
  {"type": "rectangle", "id": "start", "x": 600, "y": 0, "width": 180, "height": 80, "label": {"text": "Start a project"}},
  {"type": "arrow", "id": "then", "x": 180, "y": 40, "start": {"id": "sign-up"}, "end": {"id": "verify"}},
  {"type": "arrow", "id": "next", "x": 480, "y": 40, "start": {"id": "verify"}, "end": {"id": "start"}}]}
```"##;

/// "Drawing a new whiteboard" (3.4), beside the open document.
fn document_section() -> String {
    new_whiteboard_section(
        "(how the characters are related, the outline as boxes, a timeline, how the parts of an argument connect), answer with a drawing: a button under your answer makes it into a new whiteboard beside the document.",
        "Who serves whom",
        "asked who serves whom in the chapter",
        DOCUMENT_EXAMPLE,
    )
}

/// "Drawing a new whiteboard" (3.4), beside the open page: the same section in a page's
/// words (the page is never "the document", and it stays as it is).
fn page_section() -> String {
    new_whiteboard_section(
        "that the page shows or describes (its steps, its parts and how they fit together, a timeline, who does what), answer with a drawing: a button under your answer makes it into a new whiteboard beside the page; the page itself stays as it is.",
        "Getting started",
        "asked to draw the steps the page describes",
        PAGE_EXAMPLE,
    )
}

/// "Drawing a new whiteboard": `what` says what may be drawn and where the whiteboard
/// goes, `title` is the example's title, `asked` what the example answers, `example` the
/// block it answers with.
fn new_whiteboard_section(what: &str, title: &str, asked: &str, example: &str) -> String {
    format!(
        "\
Drawing a new whiteboard. When the writer asks you to draw, diagram, map or chart something {what} Never say that you made the whiteboard: say what the drawing shows.

Write the drawing as one fenced code block whose info string is whiteboard, holding one JSON object with a short \"title\" for the whiteboard and the elements to \"add\":

```whiteboard
{{\"title\": \"{title}\", \"add\": [elements]}}
```

The writer does not see the JSON: in its place the app shows one line that says what the drawing holds. Write one such block per answer, with a sentence before or after it on what it shows.

{DRAWING_ELEMENTS}

x grows to the right and y downwards, in pixels; start near (0, 0). Draw what was asked, usually under a hundred elements.

{HISTORY_NOTE_PARAGRAPH} To change a drawing you wrote before, write it again whole.

For example, {asked}:

{example}"
    )
}

/// The section of the side prompt for `target` (`docs/assistant-draws-design.md`, 3.3 and
/// 3.4, the latter in a document's words or a page's), `None` for [`DrawingTarget::None`].
pub fn drawing_prompt(target: DrawingTarget) -> Option<String> {
    match target {
        DrawingTarget::None => None,
        DrawingTarget::OpenWhiteboard => Some(whiteboard_section()),
        DrawingTarget::NewWhiteboard => Some(document_section()),
        DrawingTarget::NewWhiteboardBesidePage => Some(page_section()),
    }
}

/// The clauses of the base side prompt that change when a drawing section follows: (as
/// today, with a drawing section).
const DRAWING_CLAUSES: [(&str, &str); 2] = [
    (
        "Answer about it, sum it up and suggest changes in words, and do not offer text to put \
into it.",
        "Answer about it, sum it up and suggest changes, and do not offer text to put into it.",
    ),
    (
        "No code blocks around prose.",
        "No code blocks around prose (a whiteboard block, below, is the one exception).",
    ),
];

/// The system prompt of the side chat, with or without the tools of the library, and with
/// the drawing section of `target` after the base prompt. For [`DrawingTarget::None`]
/// exactly [`SIDE_SYSTEM_PROMPT`] (and the note without tools).
pub fn side_system_prompt(tools: bool, target: DrawingTarget) -> String {
    let mut prompt = SIDE_SYSTEM_PROMPT.to_string();
    if let Some(section) = drawing_prompt(target) {
        for (today, with_drawings) in DRAWING_CLAUSES {
            debug_assert!(prompt.contains(today), "the base prompt says: {today}");
            prompt = prompt.replace(today, with_drawings);
        }
        prompt.push_str("\n\n");
        prompt.push_str(&section);
    }
    if !tools {
        prompt.push_str(SIDE_WITHOUT_TOOLS_NOTE);
    }
    prompt
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The side chat reads the open document as Markdown, where an image is one line; it
    /// is told what that line is, with and without the tools of the library. The quick
    /// chat is sent selected text only and never such a line.
    #[test]
    fn the_side_chat_is_told_how_an_image_reads() {
        const SENTENCE: &str = "A line of the form `![description](image:<id>)` is an image in the \
document: you cannot see it; keep the line unchanged where the image should stay, and never \
invent such a line or a path to a file.";
        assert!(SIDE_SYSTEM_PROMPT.contains(SENTENCE));
        assert!(side_system_prompt(true, DrawingTarget::None).contains(SENTENCE));
        assert!(side_system_prompt(false, DrawingTarget::None).contains(SENTENCE));
        assert_eq!(system_prompt(Mode::Side), SIDE_SYSTEM_PROMPT);
        assert!(!QUICK_SYSTEM_PROMPT.contains("image:"));
    }

    /// The side chat is told that the open item may be a page, which it cannot change,
    /// with and without the tools; the quick chat never sees a page.
    #[test]
    fn the_side_chat_is_told_what_a_page_is() {
        const SENTENCE: &str = "Sometimes the open item is a page instead of a document: a \
web page saved in the library as it was made, whose tag says kind=\"page\". It cannot be \
changed";
        assert!(SIDE_SYSTEM_PROMPT.contains(SENTENCE));
        assert!(side_system_prompt(false, DrawingTarget::None).contains(SENTENCE));
        assert!(!QUICK_SYSTEM_PROMPT.contains("page:"));
    }

    /// The side chat is told that the open item may be a whiteboard, read as a list of
    /// its elements, which it cannot change, with and without the tools; the quick chat
    /// never sees one.
    #[test]
    fn the_side_chat_is_told_what_a_whiteboard_is() {
        const SENTENCE: &str = "Sometimes the open item is a whiteboard: a drawing of shapes, \
arrows, text and pictures, whose tag says kind=\"board\". You are given its elements, one \
per line, with their ids, positions, sizes and text; you cannot see the drawing itself, nor \
the pictures on it. It cannot be changed by you: only its owner draws on it, in the app.";
        assert!(SIDE_SYSTEM_PROMPT.contains(SENTENCE));
        assert!(side_system_prompt(true, DrawingTarget::None).contains(SENTENCE));
        assert!(side_system_prompt(false, DrawingTarget::None).contains(SENTENCE));
        assert!(!QUICK_SYSTEM_PROMPT.contains("whiteboard"));
    }

    const TARGETS: [DrawingTarget; 4] = [
        DrawingTarget::None,
        DrawingTarget::OpenWhiteboard,
        DrawingTarget::NewWhiteboard,
        DrawingTarget::NewWhiteboardBesidePage,
    ];

    /// Each target's section is in its side prompt and only there; without a target the
    /// side prompt is today's text exactly, with and without tools.
    #[test]
    fn each_target_has_its_section() {
        assert_eq!(drawing_prompt(DrawingTarget::None), None);
        assert_eq!(
            side_system_prompt(true, DrawingTarget::None),
            SIDE_SYSTEM_PROMPT
        );
        assert_eq!(
            side_system_prompt(false, DrawingTarget::None),
            format!("{SIDE_SYSTEM_PROMPT}{SIDE_WITHOUT_TOOLS_NOTE}")
        );
        const BOARD: &str = "Drawing on the open whiteboard.";
        const NEW: &str = "Drawing a new whiteboard.";
        for tools in [true, false] {
            let none = side_system_prompt(tools, DrawingTarget::None);
            let board = side_system_prompt(tools, DrawingTarget::OpenWhiteboard);
            let new = side_system_prompt(tools, DrawingTarget::NewWhiteboard);
            let page = side_system_prompt(tools, DrawingTarget::NewWhiteboardBesidePage);
            assert!(!none.contains(BOARD) && !none.contains(NEW));
            assert!(board.contains(BOARD) && !board.contains(NEW));
            assert!(new.contains(NEW) && !new.contains(BOARD));
            assert!(page.contains(NEW) && !page.contains(BOARD));
            // The document's words and the page's are each in their own prompt.
            const BESIDE_DOCUMENT: &str = "a new whiteboard beside the document";
            const BESIDE_PAGE: &str = "a new whiteboard beside the page";
            assert!(new.contains(BESIDE_DOCUMENT) && !new.contains(BESIDE_PAGE));
            assert!(page.contains(BESIDE_PAGE) && !page.contains(BESIDE_DOCUMENT));
            for prompt in [&board, &new, &page] {
                // The base prompt first, the section after it, the note without tools last.
                assert!(prompt.starts_with("You are the writing assistant inside Inner Word"));
                assert_eq!(prompt.ends_with(SIDE_WITHOUT_TOOLS_NOTE), !tools);
                // The two clauses that change, and the sentence that stays.
                assert!(prompt.contains(
                    "Answer about it, sum it up and suggest changes, and do not offer text to put into it."
                ));
                assert!(!prompt.contains("suggest changes in words"));
                assert!(prompt.contains(
                    "No code blocks around prose (a whiteboard block, below, is the one exception)."
                ));
                assert!(prompt.contains(
                    "It cannot be changed by you: only its owner draws on it, in the app."
                ));
                assert!(prompt.contains(DRAWING_ELEMENTS));
                assert!(prompt.contains(HISTORY_NOTE_PARAGRAPH));
            }
        }
        assert!(!QUICK_SYSTEM_PROMPT.contains("whiteboard"));
    }

    /// The budget of the sections (3.5): they are in every request of a conversation.
    #[test]
    fn the_sections_keep_to_their_budget() {
        let board = drawing_prompt(DrawingTarget::OpenWhiteboard).unwrap();
        let new = drawing_prompt(DrawingTarget::NewWhiteboard).unwrap();
        let page = drawing_prompt(DrawingTarget::NewWhiteboardBesidePage).unwrap();
        assert!(board.chars().count() <= 4_600, "{}", board.chars().count());
        assert!(new.chars().count() <= 3_600, "{}", new.chars().count());
        assert!(page.chars().count() <= 3_600, "{}", page.chars().count());
        // The slips a model makes most, said in all (a block read as no JSON, a change
        // refused at the press, a fence not found).
        for section in [&board, &new, &page] {
            assert!(section.contains("plain JSON (no comments, no trailing commas)"));
            assert!(section.contains("never inside a list item or a quote"));
        }
        assert!(board.contains("a change never has \"label\""));
        for target in TARGETS {
            assert_eq!(
                side_system_prompt(true, target).chars().count(),
                SIDE_SYSTEM_PROMPT.chars().count()
                    + drawing_prompt(target).map_or(0, |section| section.chars().count() + 2)
                    + match target {
                        DrawingTarget::None => 0,
                        // The two clauses: " in words" out, the exception in.
                        _ =>
                            " (a whiteboard block, below, is the one exception)".len()
                                - " in words".len(),
                    }
            );
        }
    }

    /// The example listing is the real format: `Scene::listing` of the example scene.
    #[test]
    fn the_example_listing_is_made_by_the_listing() {
        let listing = example_scene().listing(&innerword_board::ListingHeader {
            reference: "board:12",
            title: "Plan",
            folder: None,
        });
        assert!(listing.contains("api"));
        let section = drawing_prompt(DrawingTarget::OpenWhiteboard).unwrap();
        assert!(section.contains(listing.trim_end()), "{section}");
        assert!(section.contains(WHITEBOARD_EXAMPLE));
        let new = drawing_prompt(DrawingTarget::NewWhiteboard).unwrap();
        assert!(new.contains(DOCUMENT_EXAMPLE));
        let page = drawing_prompt(DrawingTarget::NewWhiteboardBesidePage).unwrap();
        assert!(page.contains(PAGE_EXAMPLE));
        assert!(!page.contains(DOCUMENT_EXAMPLE) && !new.contains(PAGE_EXAMPLE));
    }

    /// The page's section names a page, never "the document": the page is not one, and
    /// it stays as it is. Its shape line names its own example's title.
    #[test]
    fn the_page_section_speaks_of_a_page() {
        let page = drawing_prompt(DrawingTarget::NewWhiteboardBesidePage).unwrap();
        assert!(!page.contains("document"), "{page}");
        assert!(page.contains("a new whiteboard beside the page; the page itself stays as it is."));
        assert!(page.contains("For example, asked to draw the steps the page describes:"));
        assert!(page.contains(r#"{"title": "Getting started", "add": [elements]}"#));
        assert!(!page.contains("Who serves whom") && !page.contains("chapter"));
        // The document's section reads as before.
        let new = drawing_prompt(DrawingTarget::NewWhiteboard).unwrap();
        assert!(new.contains(
            "Drawing a new whiteboard. When the writer asks you to draw, diagram, map or chart \
something (how the characters are related, the outline as boxes, a timeline, how the parts of \
an argument connect), answer with a drawing: a button under your answer makes it into a new \
whiteboard beside the document. Never say that you made the whiteboard: say what the drawing \
shows."
        ));
        assert!(new.contains(r#"{"title": "Who serves whom", "add": [elements]}"#));
        assert!(new.contains("For example, asked who serves whom in the chapter:"));
        // Its side prompt keeps the base prompt's sentence about pages: a page cannot be
        // changed, and the drawing makes a whiteboard beside it.
        let prompt = side_system_prompt(true, DrawingTarget::NewWhiteboardBesidePage);
        assert!(prompt.contains("whose tag says kind=\"page\". It cannot be changed"));
    }

    /// The examples of the prompts draw, with the counts their prose claims: a prompt
    /// that teaches a field the converter refuses fails here.
    #[test]
    fn the_examples_in_the_prompts_draw() {
        let drawing = crate::answer_drawing(WHITEBOARD_EXAMPLE).unwrap().unwrap();
        let mut scene = example_scene();
        let mut making = innerword_board::HashedMaking::new(&[b"example"], 0);
        let drawn = innerword_board::draw(&mut scene, &drawing, None, &mut making).unwrap();
        let added: Vec<&str> = drawn.added.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(added, vec!["db", "reads"], "the database and the arrow");
        assert!(drawn.changed.contains(&"api".to_string()), "the API");
        assert!(drawn.unchanged.is_empty() && drawn.renamed.is_empty());
        assert!(drawn.warnings.is_empty(), "{:?}", drawn.warnings);
        assert_eq!(
            scene.element("api").unwrap().json()["backgroundColor"],
            "#a5d8ff"
        );
        assert_eq!(
            scene.element("reads").unwrap().binding("start"),
            Some("api")
        );
        assert_eq!(scene.element("reads").unwrap().binding("end"), Some("db"));
        assert_eq!(
            crate::drawing_sentence(&drawing, DrawingTarget::OpenWhiteboard, None),
            "Drawing: adds an ellipse and an arrow; changes api."
        );

        let drawing = crate::answer_drawing(DOCUMENT_EXAMPLE).unwrap().unwrap();
        assert_eq!(drawing.title.as_deref(), Some("Who serves whom"));
        assert!(
            !drawing.has_changes(),
            "a new whiteboard's drawing only adds"
        );
        let mut scene = innerword_board::Scene::empty();
        let drawn = innerword_board::draw(&mut scene, &drawing, None, &mut making).unwrap();
        let added: Vec<&str> = drawn.added.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(added, vec!["keeper", "ada", "serves"]);
        assert_eq!(scene.element("serves").unwrap().binding("end"), Some("ada"));
        assert_eq!(
            crate::drawing_sentence(&drawing, DrawingTarget::NewWhiteboard, None),
            "Drawing: adds 2 ellipses and an arrow (new whiteboard “Who serves whom”)."
        );

        let drawing = crate::answer_drawing(PAGE_EXAMPLE).unwrap().unwrap();
        assert_eq!(drawing.title.as_deref(), Some("Getting started"));
        assert!(
            !drawing.has_changes(),
            "a new whiteboard's drawing only adds"
        );
        let mut scene = innerword_board::Scene::empty();
        let drawn = innerword_board::draw(&mut scene, &drawing, None, &mut making).unwrap();
        let added: Vec<&str> = drawn.added.iter().map(|e| e.id.as_str()).collect();
        assert_eq!(added, vec!["sign-up", "verify", "start", "then", "next"]);
        assert!(drawn.warnings.is_empty(), "{:?}", drawn.warnings);
        assert_eq!(
            scene.element("then").unwrap().binding("start"),
            Some("sign-up")
        );
        assert_eq!(
            scene.element("then").unwrap().binding("end"),
            Some("verify")
        );
        assert_eq!(
            scene.element("next").unwrap().binding("start"),
            Some("verify")
        );
        assert_eq!(scene.element("next").unwrap().binding("end"), Some("start"));
        assert_eq!(
            crate::drawing_sentence(&drawing, DrawingTarget::NewWhiteboardBesidePage, None),
            "Drawing: adds 3 rectangles and 2 arrows (new whiteboard “Getting started”)."
        );
    }
}
