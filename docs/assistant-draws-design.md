# The assistant draws on whiteboards: design

The side chat (CLAUDE.md, "The assistant") reads a whiteboard as the listing of its
elements and cannot change it. With this feature it proposes **drawings**: an answer may
hold one drawing, in the agents' own formats (CLAUDE.md, "Agents" > "Whiteboards";
`docs/whiteboard-design.md`, sections 7.2 and 7.3), and a button under the answer applies
it:

- over an open **whiteboard**: "Draw on Whiteboard" adds shapes, arrows, text and frames,
  changes elements by id and removes elements by id, as one step of Excalidraw's undo;
- over a text **document**: "New Whiteboard from Answer" makes a new whiteboard beside the
  document, in its folder, with the drawing on it.

What does not change: the assistant advises and the writer applies (nothing happens until
a button, its key or its command); the assistant keeps the twelve read-only tools and no
others; a path or an address in an answer is never read; tests use the fake engine only;
the product rules of CLAUDE.md hold.

This is the design for three builders working in parallel on fixed interfaces (section
11). The words "the listing", "the base rule", "adopt", "collect", "held scene" and
"deliberate end" are `docs/whiteboard-design.md`'s.

## Decisions at a glance

| # | Question | Decision |
|---|---|---|
| 1 | How an answer carries a drawing | One fenced code block with the info string `whiteboard`, holding one JSON object `{"title"?, "add"?, "update"?, "remove"?}` in the agents' formats (a bare JSON array is read as `{"add": [...]}`). Unknown keys, an `image` element and more than 2,000 entries in a list make it unusable. |
| 2 | Several blocks in one answer | Taken together, in order, as one drawing (adds, then changes, then removals; the first `title`). The prompt asks for one block per answer and no alternatives as separate blocks. |
| 3 | What the chat shows for a block | Never the JSON. Each block is replaced in the shown answer by one line in italics: its summary ("Drawing: adds 2 ellipses and 3 arrows; changes api; removes db."), its problem ("Drawing that cannot be used: …"), or "Drawing…" while it is still being written. |
| 4 | Streaming | The button is offered only under an answer that is not being written and whose blocks are closed and read. |
| 5 | Who reads the block | `innerword-board` owns the format (`Drawing`, `Drawing::from_json`, `draw`); `innerword-assistant` finds the blocks and reads them with it (a new edge `assistant → board`: pure, no cycle, already in its build through `mcp → agent → board`); `innerword-assistant-ui` asks `innerword-assistant` only; the app applies with `innerword-board`. |
| 6 | The prompts | The side chat's system prompt is chosen by the open item: a whiteboard adds the section "Drawing on the open whiteboard", a document "Drawing a new whiteboard", a page and nothing add nothing. At most 4,400 and 3,400 characters (about 1,100 and 850 tokens), each with one worked example that a test draws. |
| 7 | Applying on an open whiteboard | As a restored version is applied (6.5 of the whiteboard design): collect, flush, keep a version labelled "Before the assistant's drawing", `innerword_board::draw` on the stored scene, save compare-and-swap, `adopt` the delta with `undoable: true` (one step of Excalidraw's undo, the base rule against a gesture made meanwhile), then the keyboard to the canvas and a toast with Undo. |
| 8 | Ids that are gone | A removal of an element that is gone is left out (it is gone). A change of a gone element, or a new element that names one (an arrow's end, a frame's child), draws nothing and the toast names the ids. An id the answer gives a new element that the whiteboard has is renamed (`api` → `api-2`) with its references inside `add`. |
| 9 | Placement | As written, except a drawing that stands on its own (only `add`, naming nothing on the whiteboard): when it would cover the drawing or lie outside the view, it is moved as a whole to the free place in view nearest to where it was written; when no place in view is free, beside the drawn area, and the view zooms to fit. |
| 10 | New Whiteboard from Answer | In the document's folder, titled with the block's `title` (else "<document title> (drawing)"), opened with the keyboard on its canvas; the document's conversation stays the document's. Offered only for a drawing that only adds; `update` or `remove` over a document disable the button with a reason. |
| 11 | Keys and commands | One action `assistant::DrawAnswer`, key `cmd-shift-d` in the side chat (the marked answer, else the latest). Button "Draw" (tooltip "Draw on Whiteboard") over a whiteboard, "New Whiteboard" (tooltip "New Whiteboard from Answer") over a document, first in the row; an item of the answer's menu; palette "New whiteboard from answer", renamed "Draw answer on whiteboard" while a whiteboard is open; no menu bar item (Insert, Replace and Copy have none either). The list of shortcuts gains one row "In the assistant's chats" (443 with the assistant shown, 412 without, unchanged) and the shortcuts file two entries (426). |
| 12 | Insert, Replace, Copy | Take the answer without its drawing blocks; disabled under an answer that is nothing but a drawing. |
| 13 | Safety | A drawing changes only the open whiteboard (or makes one new whiteboard), only on the writer's press, shows what it does in a summary the app writes from the block (not the assistant's prose), keeps a version first, is one undo step, brings no picture, reads no path, fetches nothing, links only `http`, `https`, `mailto`, and stays within the agents' limits. |
| 14 | Packages | A: `innerword-board` + `innerword-assistant` (pure). B: `innerword-assistant-ui`. Both start at once; B builds against A's first milestone (A1, the types and the block reader), committed first. C: `innerword-app` and CLAUDE.md, after A and B. |

## 1. What the writer sees

**On a whiteboard.** The writer has "Plan" open, the side chat beside it (`cmd-3`), and
asks "Add the database the API reads from, and make the API blue." The answer arrives:

> I put the database to the right of the API and joined them with an arrow labelled
> "reads"; the API is filled in blue.
>
> *Drawing: adds an ellipse and an arrow; changes api.*
>
> **Draw** · Insert · Replace · Copy

(Insert and Replace are dimmed on a whiteboard, as today.) While the answer is being
written the italic line says *Drawing…* and there are no buttons. The writer presses
**Draw**, `cmd-shift-d`, the item "Draw on Whiteboard" of the answer's menu, or "Draw
answer on whiteboard" in the palette. The ellipse and the arrow appear, the API turns
blue, the keyboard is on the canvas, and a toast says "Drew on “Plan”: 2 elements added,
1 changed. The whiteboard as it was is kept as a version." with **Undo**. `cmd-z` right
away takes the whole drawing out; so does Undo; the version history has a row "Before the
assistant's drawing".

**On a document.** The writer has "Chapter 2" open and asks "Draw who serves whom in this
chapter." The answer has a drawing that only adds; its line reads *Drawing “Who serves
whom”: adds 3 ellipses and 2 arrows.*, and its first button is **New Whiteboard**.
Pressing it makes the whiteboard "Who serves whom" in the folder of "Chapter 2", opens it
with the keyboard on the canvas, fitted to its drawing, and says "Made the whiteboard
“Who serves whom” in Part one from the answer." The side chat now shows the whiteboard's
own (empty) conversation, where the writer can go on ("make the arrows dashed"); the
conversation about "Chapter 2" is where it was, with the chapter.

## 2. The drawing in an answer

### 2.1 The block

````text
```whiteboard
{"title": "Who serves whom",
 "add": [ ...skeletons... ],
 "update": [ ...changes by id... ],
 "remove": ["db", "cache"]}
```
````

- **The fence**: a line of three or more backticks or tildes, indented at most three
  spaces, whose info string's first word is `whiteboard` (letters compared without case),
  up to a closing fence of the same character at least as long (CommonMark's rule; the
  same scanner as `context::outline`'s). A `whiteboard` fence inside another fenced block
  is text of that block, not a drawing. Its body is everything between the two fence lines.
- **The JSON**: one object with any of four keys, every other key refused:
  - `add`: a list of element skeletons, exactly `innerword_board::Skeleton`s (7.2);
  - `update`: a list of changes, exactly `innerword_board::ElementChange`s (7.3);
  - `remove`: a list of ids (strings);
  - `title`: a string, the title of a new whiteboard (trimmed, white space runs made one
    space, cut at 120 characters; ignored over a whiteboard, which keeps its name).
  A bare JSON array is read as `{"add": [...]}` (the skill teaches agents an array, and a
  model may write one).
- **Refused when the block is read** (the drawing cannot be used; section 2.4 has the
  words): not JSON; JSON that is not such an object or array; a key that is not one of the
  four; `add`, `update` or `remove` not a list, an entry of `add` or `update` not an
  object, of `remove` not a string; nothing in any list; an `add` entry of `"type":
  "image"` (a drawing from an answer brings no picture: section 8); more than
  `innerword_board::limits::MAX_REQUEST_ELEMENTS` (2,000) entries in one of the lists,
  after the blocks of an answer are joined. Everything else (a field, a value, an id the
  whiteboard does not have, the whiteboard's limits) is judged when the drawing is applied,
  by `innerword_board::draw`, against the scene as it is then (section 5).
- **Strict JSON**: serde_json, no comments, no trailing commas. The error names the line
  and column within the block.

### 2.2 Several blocks

The prompt asks for one block per answer, and for alternatives in words. When an answer
has several anyway, **they are one drawing**, joined in order (`Drawing::join`): the adds
of all blocks, then the changes, then the removals, and the first `title` given. An
answer with an unusable block has no usable drawing; the button is disabled with the
first problem, named with the block's ordinal ("its second block is not JSON …").

### 2.3 What the chat shows

The side chat's answer view (a read-only editor over the Markdown the importer makes of
the answer, `innerword_assistant_ui::markdown::answer_document`) is given the answer with
each block replaced by one paragraph in italics (`innerword_assistant::shown_markdown`):

| The block | The line |
|---|---|
| closed, read, usable | `*Drawing: adds 2 ellipses and 3 arrows; changes api and db; removes cache.*` (with a title: `*Drawing “Who serves whom”: adds …*`) |
| closed, cannot be used | `*Drawing that cannot be used: it is not JSON (line 3, column 14: expected `,` or `}`).*` |
| open, the answer still being written | `*Drawing…*` (everything from the fence to the end of the text) |
| open, the answer finished (stopped, failed, or the model forgot the fence) | `*Drawing that cannot be used: it was not finished.*` |

The JSON is never shown in the chat. The summary is written by the app from the block, so
it says what the drawing does whatever the prose around it says (section 8). The quick
chat does not do this: its prompt has no drawings, and its answers are shown as today
(`MessageListOptions::drawings(false)`, the default).

**The summary's words** (`innerword_assistant::drawing_sentence`):

- adds: counts by type in a fixed order (rectangle, ellipse, diamond, text, arrow, line,
  drawn line for `freedraw`, frame), "a"/"an" for one ("a frame", "an ellipse"), joined
  with commas and "and": "adds 4 rectangles, 3 arrows and a frame";
- changes and removes: up to three ids, then "and N more": "changes api, db and cache",
  "changes 7 elements (api, db, cache and 4 more)", "removes db";
- the parts joined with "; ", ended with a period; a `title` after "Drawing" in curly
  quotes.

### 2.4 What a block that cannot be used says

`innerword_board::DrawingError` (its `Display`, a clause without a capital or a period)
and `innerword_assistant::DrawingProblem::reason` give the clause; the chat puts it after
"Drawing that cannot be used: ", the disabled button's tooltip and its menu item after the
same words, and the toast of the key after "Nothing was drawn: ".

| Problem | Clause |
|---|---|
| not JSON | `it is not JSON (line 3, column 14: expected `,` or `}`)` |
| not a drawing | `it is not a drawing: “elements” is not one of its parts (add, update, remove and title are)`; `… add must be a list of elements`; `… remove must be a list of ids` |
| empty | `it adds, changes and removes nothing` |
| a picture | `the element at index 2 of add is a picture, and a drawing from an answer cannot bring one` |
| too large | `it adds more than 2,000 elements` (changes, removes) |
| not closed | `it was not finished` |
| several blocks | `its second block is not JSON (…)` (the ordinal before the clause of that block) |
| over a document, with `update` or `remove` | `it changes elements of a whiteboard: open that whiteboard and ask there` |

## 3. The prompts

### 3.1 Which prompt a conversation gets

A side chat's conversation is the open item's own (CLAUDE.md, "Conversations"), so its
system prompt can be the item's. **`innerword_assistant::DrawingTarget`** says what an open
item offers, and the system prompt follows it:

| Open item | Page engine (whiteboards can be shown) | `DrawingTarget` | Section added to the side prompt |
|---|---|---|---|
| a whiteboard | yes | `OpenWhiteboard` | "Drawing on the open whiteboard" (3.3) |
| a document | yes | `NewWhiteboard` | "Drawing a new whiteboard" (3.4) |
| a page, nothing, or any item without a page engine | | `None` | none |

`ChatSession` holds the target (`set_drawing_target`), builds the system prompt from it
when it starts a conversation (`prompts::side_system_prompt(tools, target)`), and a change
of target retires the running conversation, so the next message starts one with the new
prompt and the conversation so far (as a change of tools does). The panel sets it from the
host (`AssistantPanel::set_drawing_target`) before anything is sent or prepared. The quick
chat's prompt does not change.

The base side prompt changes in one clause: the whiteboard paragraph's last sentence
becomes "Answer about it, sum it up and suggest changes, and do not offer text to put into
it." (was "… suggest changes in words …"). The sentence the existing test holds ("It
cannot be changed by you: only its owner draws on it, in the app.") stays: it is still
true, the owner applies.

### 3.2 The part both sections share

`prompts::DRAWING_ELEMENTS`, reused from the skill's "Whiteboards" paragraph and 7.2:

```text
The elements are Excalidraw's element skeletons, JSON objects with Excalidraw's own field names:
- Every element has "type", "x" and "y" (its top left corner) and an "id" of your own (letters, digits, "-" and "_"), by which arrows, frames and later changes name it.
- "rectangle", "ellipse" and "diamond" may have "width" and "height" and a "label": {"text": "API"}. A shape with a label and no size is fitted to its label; make one that holds a few words about 160 x 80.
- "text" has "text" and may have "fontSize" (20; 28 for a heading). Never give a text a width or a height.
- "arrow" joins two elements: "start": {"id": "api"} and "end": {"id": "db"}, with "x" and "y" near its start; it may have a "label". An arrow or a "line" that joins nothing has "points" relative to its x and y, such as [[0, 0], [200, 0]].
- "frame" holds elements: "children": ["api", "db"] and a "name"; without a size it is fitted around them.
- Any element may have "strokeColor" and "backgroundColor" ("#1971c2", "#a5d8ff", "transparent"), "fillStyle" ("solid", "hachure", "cross-hatch"), "strokeWidth" (1, 2 or 4), "strokeStyle" ("solid", "dashed", "dotted"), "roughness" (0 neat, 1, 2 sketchy), "opacity" (0 to 100), "angle" (in radians) and "link" (an http, https or mailto address).
- A drawing cannot bring a picture: there is no "image" element.
Leave room: 60 to 120 between shapes, rows and columns lined up, few colours.
```

### 3.3 "Drawing on the open whiteboard"

````text
Drawing on the open whiteboard. You cannot change the whiteboard yourself, but you can propose a drawing: a button under your answer lets the writer apply it, as one step they can undo, and the whiteboard as it was is kept as a version. Propose a drawing when the writer asks you to draw, add, arrange, connect, restyle, relabel or remove something on the whiteboard. Never say that you drew or changed anything: say what the drawing will do.

Write the drawing as one fenced code block whose info string is whiteboard, holding one JSON object with up to three lists:

```whiteboard
{"add": [new elements], "update": [changes], "remove": ["id"]}
```

The writer does not see the JSON: in its place the app shows one line that says what the drawing adds, changes and removes. Write one such block per answer, with a sentence or two around it on what it does; to offer alternatives, describe them and draw the one the writer chooses.

{DRAWING_ELEMENTS}

"update" changes elements of the whiteboard by the ids of the listing: {"id": "api", "dx": 40, "dy": 0} moves one ("x" and "y" move it to a place); a change may also set "width", "height", "angle", "text" (of a text, or the label of a shape or an arrow), "fontSize", "name" (of a frame), "start" or "end" (an arrow joined to another element), "link" (null removes it) and any style field above. A label, the ends of arrows and what a frame holds follow by themselves. "remove" takes ids: a shape goes with its label and the arrows joined to it stay; a frame goes and what it held stays.

Positions are the whiteboard's own, as the listing gives them: x grows to the right and y downwards, in pixels at 100 %; "at (x, y)" is an element's top left corner, and the listing's first line gives the drawn area. Give new elements ids the listing does not have. A drawing that stands on its own goes beside what is drawn, 100 or more to the right of or below the drawn area; one that would cover the drawing is moved by the app to free space.

For example, with this listing:
board:12 “Plan”: 1 element, 0 pictures; drawn area 200 x 90 at (100, 80)
rectangle  api  at (100, 80)  200 x 90  label "API"
asked to add the database the API reads from and to make the API blue, answer:

```whiteboard
{"add": [
  {"type": "ellipse", "id": "db", "x": 420, "y": 80, "width": 160, "height": 90, "label": {"text": "Postgres"}},
  {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}, "label": {"text": "reads"}}],
 "update": [{"id": "api", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}]}
```
````

### 3.4 "Drawing a new whiteboard"

````text
Drawing a new whiteboard. When the writer asks you to draw, diagram, map or chart something (how the characters are related, the outline as boxes, a timeline, how the parts of an argument connect), answer with a drawing: a button under your answer makes it into a new whiteboard beside the document. Never say that you made the whiteboard: say what the drawing shows.

Write the drawing as one fenced code block whose info string is whiteboard, holding one JSON object with a short "title" for the whiteboard and the elements to "add":

```whiteboard
{"title": "Who serves whom", "add": [elements]}
```

The writer does not see the JSON: in its place the app shows one line that says what the drawing holds. Write one such block per answer, with a sentence before or after it on what it shows.

{DRAWING_ELEMENTS}

x grows to the right and y downwards, in pixels; start near (0, 0). Draw what was asked, usually under a hundred elements.

For example, asked who serves whom in the chapter:

```whiteboard
{"title": "Who serves whom", "add": [
  {"type": "ellipse", "id": "keeper", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "The keeper"}},
  {"type": "ellipse", "id": "ada", "x": 320, "y": 0, "width": 180, "height": 80, "label": {"text": "Ada"}},
  {"type": "arrow", "id": "serves", "x": 180, "y": 40, "start": {"id": "keeper"}, "end": {"id": "ada"}, "label": {"text": "serves"}}]}
```
````

### 3.5 Budget, and the history

- **Budget**: the whiteboard section (with the shared part) at most 4,400 characters, the
  document section at most 3,400 (about 1,100 and 850 tokens at four characters a token),
  held by a test. The base side prompt is about 5,300 characters, so a whiteboard's
  conversation starts with about 2,400 tokens of system prompt. It is sent once per
  process, not per message. The builder may shorten the wording to fit; the facts of 3.2
  to 3.4 stay.
- **The examples are tested**: a test of `innerword-assistant` takes the ```` ```whiteboard
  ```` block out of each section with `answer_drawing`, and draws it with
  `innerword_board::draw` on the scene its listing names (the whiteboard example on a scene
  holding the rectangle `api`, the document example on an empty scene) without an error.
  A prompt that teaches a field the converter refuses fails.
- **The history carried into a new process** (`Transcript::history`): an answer's drawing
  blocks are replaced by their summary line in parentheses ("(A drawing: adds an ellipse
  and an arrow; changes api.)"). The JSON would crowd the 60,000 characters out, and the
  listing sent with the next message says what is on the whiteboard now. The stored
  transcript keeps the answer as written.

## 4. Where each piece lives

Dependency direction (CLAUDE.md) gains one edge, `assistant → board`: `innerword-board`
depends on nothing of ours and has no GPUI, and it is already in `innerword-assistant`'s
build through `mcp → agent → board`. So the format has one reader: the chat's summary, the
button's state and the app's application read the same `innerword_board::Drawing`.
`innerword-assistant-ui` does not depend on `innerword-board`; it uses the types through
`innerword-assistant`'s re-exports.

### 4.1 `innerword-board`: `src/drawing.rs` (new), re-exported from `lib.rs`

```rust
/// A drawing from an answer of the assistant: what it adds, changes and removes, in the
/// agents' own formats (`docs/whiteboard-design.md`, 7.2 and 7.3). Read with
/// [`Drawing::from_json`], applied with [`draw`].
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Drawing {
    /// The title of a new whiteboard: one line, at most 120 characters. `None`: not given.
    pub title: Option<String>,
    pub add: Vec<Skeleton>,
    pub update: Vec<ElementChange>,
    pub remove: Vec<String>,
}

/// Why the JSON of a drawing cannot be used (no whiteboard was looked at).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingError {
    /// Not JSON: where, within the text given, and serde_json's words.
    NotJson { line: usize, column: usize, message: String },
    /// JSON, but not an object of `add`, `update`, `remove` and `title`, nor an array of
    /// skeletons: what is wrong, in words.
    NotADrawing { message: String },
    /// Nothing to add, change or remove.
    Empty,
    /// An `image` skeleton: a drawing from an answer brings no picture.
    Picture { element: usize },
    /// A list over `limits::MAX_REQUEST_ELEMENTS`: `list` is "add", "update" or "remove".
    TooLarge { list: &'static str, limit: usize },
}

impl std::fmt::Display for DrawingError { /* the clauses of section 2.4 */ }
impl std::error::Error for DrawingError {}

impl Drawing {
    /// Reads the body of a drawing block (2.1).
    pub fn from_json(text: &str) -> Result<Drawing, DrawingError>;
    /// The drawings of several blocks as one, in order (2.2): `TooLarge` when a joined
    /// list is over the limit.
    pub fn join(drawings: Vec<Drawing>) -> Result<Drawing, DrawingError>;
    /// Whether it adds, changes and removes nothing.
    pub fn is_empty(&self) -> bool;
    /// Whether it changes or removes anything (`update` or `remove` not empty): it needs a
    /// whiteboard that has those elements.
    pub fn has_changes(&self) -> bool;
    /// What it adds by type, in the order of 2.3, each with its count; a label is not
    /// counted apart.
    pub fn added_types(&self) -> Vec<(String, usize)>;
    /// The ids its changes name, each once, in order.
    pub fn changed_ids(&self) -> Vec<String>;
    /// The ids its removals name, each once, in order.
    pub fn removed_ids(&self) -> Vec<String>;
    /// The drawing as compact JSON (the seed of its `HashedMaking`, and tests).
    pub fn to_json(&self) -> String;
}

/// Where a drawing that stands on its own was put (section 5.3).
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Placement {
    /// How far its elements were moved from where the answer wrote them.
    pub dx: f64,
    pub dy: f64,
    /// Whether all of it lies within the view given (`false` without a view).
    pub in_view: bool,
}

/// What [`draw`] did.
#[derive(Clone, Debug, PartialEq)]
pub struct Drawn {
    /// The elements added, as `add` reports them (ids as made).
    pub added: Vec<AddedElement>,
    /// Every element its changes changed, followers included (`Updated::changed`).
    pub changed: Vec<String>,
    /// The elements removed, labels included (`Removed::removed`).
    pub removed: Vec<String>,
    /// Arrows left unbound by a removal.
    pub unbound: Vec<String>,
    /// Ids its removals named that the whiteboard no longer has: left out.
    pub already_gone: Vec<String>,
    /// Ids the answer gave new elements that the whiteboard had: (as written, as made).
    pub renamed: Vec<(String, String)>,
    /// `Some` for a drawing that stands on its own (only `add`, naming nothing that was
    /// on the whiteboard); `None`: it stays where it was written.
    pub placement: Option<Placement>,
    /// The converter's warnings ("db sticks out of frame backend: …"), after placement.
    pub warnings: Vec<String>,
}

/// Why nothing was drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawError {
    /// It names elements the whiteboard no longer has: `changed` by its changes, `named`
    /// by its new elements (an arrow's `start` or `end`, a frame's `children`).
    Gone { changed: Vec<String>, named: Vec<String> },
    /// The converter's refusal: a field, a value, the whiteboard's limits for agents.
    Board(BoardError),
}

impl std::fmt::Display for DrawError { /* "the answer changes api and db, which are no longer on the whiteboard"; BoardError's words */ }

/// Applies `drawing` to `scene`, whole or not at all (on an error the scene is as it was):
/// 1. removals of ids the scene does not have are left out (`already_gone`);
/// 2. a change of an id the scene does not have, or an add that names one that is neither
///    in the scene nor among its own adds, is `DrawError::Gone`;
/// 3. an id of `add` that the scene has is renamed to the first free `<id>-2`, `<id>-3`, …,
///    with every reference to it inside `add` (`start`, `end`, `children`); `update` and
///    `remove` always name what was on the whiteboard;
/// 4. [`add`] with no pictures, then, for a drawing that stands on its own, [`place`] and
///    the added elements moved by its offset (one rigid move: they name nothing else);
/// 5. [`update`], then [`remove`];
/// 6. the warnings of frames computed again after the move.
/// The limits are the agents' (`check_limits`: 20,000 elements, 16 MB). `view` is where
/// the writer is looking, in scene coordinates (`None`: unknown, or a new whiteboard).
pub fn draw(
    scene: &mut Scene,
    drawing: &Drawing,
    view: Option<Bounds>,
    making: &mut dyn Making,
) -> Result<Drawn, DrawError>;

/// Where a group of elements with bounds `group` goes, given the bounds of what is on the
/// whiteboard and the view (section 5.3). Pure; deterministic.
pub fn place(group: Bounds, occupied: &[Bounds], view: Option<Bounds>) -> Placement;

/// The room kept free around a placed drawing, and its distance from the drawn area.
pub const PLACE_MARGIN: f64 = 40.0;
pub const PLACE_GAP: f64 = 100.0;
```

`lib.rs`: `mod drawing; pub use drawing::{DrawError, Drawing, DrawingError, Drawn, Placement,
PLACE_GAP, PLACE_MARGIN, draw, place};` and the crate docs gain a line: "What the assistant
proposes: a drawing ([`Drawing`]) applied whole or not at all ([`draw`]), a drawing that
stands on its own put where it is seen ([`place`])."

### 4.2 `innerword-assistant`

`Cargo.toml`: `innerword-board.workspace = true` (with a comment: the format of an answer's
drawing, read with the converter's own types).

**`src/drawing.rs`** (new):

```rust
pub use innerword_board::{Drawing, DrawingError};

/// The info string of the fenced block that holds an answer's drawing.
pub const DRAWING_INFO: &str = "whiteboard";

/// What drawings a side chat offers, by its open item (section 3.1).
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash)]
pub enum DrawingTarget {
    /// None: the open item is a page, nothing is open, or whiteboards cannot be shown.
    #[default]
    None,
    /// The open item is a whiteboard: a drawing adds to it, changes and removes elements.
    OpenWhiteboard,
    /// The open item is a document: a drawing that only adds makes a new whiteboard.
    NewWhiteboard,
}

impl DrawingTarget {
    /// The target for an open item of `kind`, where `boards` says whether whiteboards can
    /// be shown (the app has a page engine).
    pub fn of(kind: Option<&OpenKind>, boards: bool) -> DrawingTarget;
}

/// One fenced drawing block of an answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DrawingBlock {
    /// Its bytes in the answer's Markdown: from the start of the opening fence's line to
    /// the end of the closing fence's line (its line end included), or to the end of the
    /// text when it is not closed.
    pub range: std::ops::Range<usize>,
    /// The text between the fence lines.
    pub body: String,
    /// Whether its closing fence came.
    pub closed: bool,
}

/// The drawing blocks of an answer, in order (2.1).
pub fn drawing_blocks(markdown: &str) -> Vec<DrawingBlock>;

/// Why an answer's drawing cannot be used (here).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum DrawingProblem {
    /// A block was not closed.
    NotFinished { block: usize, of: usize },
    /// A block cannot be read, or the blocks joined are over a limit.
    Unreadable { block: Option<usize>, of: usize, error: DrawingError },
    /// Over a document: it changes or removes elements.
    ChangesAWhiteboard,
    /// No drawing can be applied over the open item.
    NotHere,
}

impl DrawingProblem {
    /// The clause of 2.4 (no capital, no period), with the block's ordinal when the answer
    /// has several.
    pub fn reason(&self) -> String;
}

/// The drawing of an answer: `None` when it has no block; else its blocks read and
/// joined (2.2), or the first problem.
pub fn answer_drawing(markdown: &str) -> Option<Result<Drawing, DrawingProblem>>;

/// Whether `drawing` can be applied for `target`: `NotHere` for `None`,
/// `ChangesAWhiteboard` for `NewWhiteboard` and a drawing with changes.
pub fn drawing_fits(drawing: &Drawing, target: DrawingTarget) -> Result<(), DrawingProblem>;

/// The answer without its drawing blocks, what Insert, Replace and Copy take (the blank
/// lines around a block made one).
pub fn without_drawings(markdown: &str) -> String;

/// The answer as the side chat shows it (2.3): each block replaced by one line in
/// italics. Borrowed when there is no block.
pub fn shown_markdown(markdown: &str, streaming: bool) -> std::borrow::Cow<'_, str>;

/// The summary of a drawing (2.3): "Drawing “Plan”: adds an ellipse and an arrow;
/// changes api."
pub fn drawing_sentence(drawing: &Drawing) -> String;
```

**`src/prompts.rs`**:

```rust
/// What both drawing sections say about elements (3.2).
pub const DRAWING_ELEMENTS: &str = "...";

/// The section of the side prompt for `target` (3.3, 3.4), `None` for `DrawingTarget::None`.
pub fn drawing_prompt(target: DrawingTarget) -> Option<String>;

/// The system prompt of the side chat, with or without the tools of the library, and with
/// the drawing section of `target`. (Was `side_system_prompt(tools)`.)
pub fn side_system_prompt(tools: bool, target: DrawingTarget) -> String;
```

The sections are built by `format!` from `DRAWING_ELEMENTS` (`concat!` takes literals only).
The order of the side prompt: the base prompt, the drawing section after a blank line, the
note without tools last.

**`src/session.rs`** (`ChatSession`):

```rust
/// The same chat offering the drawings of `target` (its system prompt follows).
pub fn with_drawing_target(self, target: DrawingTarget) -> Self;
/// Offers the drawings of `target` from the next message on: a running conversation
/// started for another target is retired, and the next message starts one with the
/// conversation so far, as a change of tools does.
pub fn set_drawing_target(&mut self, target: DrawingTarget);
/// What the chat offers.
pub fn drawing_target(&self) -> DrawingTarget;
```

`Running` records its target; `ensure_running` restarts when it differs (as for the
selection); a quick chat ignores the target. A system prompt given with
`with_system_prompt` wins as today.

**`src/transcript.rs`**: `Transcript::history` replaces each drawing block of an answer by
"(A drawing: …)" (3.5).

**`src/lib.rs`**: `pub mod drawing;` and `pub use drawing::{DRAWING_INFO, Drawing, DrawingBlock,
DrawingError, DrawingProblem, DrawingTarget, answer_drawing, drawing_blocks, drawing_fits,
drawing_sentence, shown_markdown, without_drawings};`, `pub use prompts::{DRAWING_ELEMENTS,
drawing_prompt, side_system_prompt, ...}`. The crate docs gain "Drawings" in the list.

### 4.3 `innerword-assistant-ui`

**`src/actions.rs`**:

```rust
actions!(assistant, [ /* the existing ones */
    /// Draw the answer's drawing: on the open whiteboard, or as a new whiteboard beside
    /// the open document.
    DrawAnswer,
]);
// key_bindings(): beside InsertAtCaret, in the same two places
KeyBinding::new("cmd-shift-d", DrawAnswer, panel),
KeyBinding::new("cmd-shift-d", DrawAnswer, side_editor),
// commands(): after "Replace selection with answer"
CommandDescriptor::new("New whiteboard from answer", CATEGORY, DrawAnswer).keywords([
    "assistant", "draw", "drawing", "diagram", "whiteboard", "apply",
]),
```

The module's table of keys and the crate docs' table gain the row `cmd-shift-d`.

**`src/messages.rs`**:

```rust
/// Whether an action is offered under one answer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ActionState {
    /// Not under this answer, nor in its menu.
    Hidden,
    /// Shown, and does what it says.
    Enabled,
    /// Shown dimmed; its tooltip and its menu item say why.
    Disabled(SharedString),
}

/// What the list knows of one answer, made once each time its text changes.
pub struct AnswerFacts<'a> {
    pub message: &'a Message,
    /// Its drawing (`innerword_assistant::answer_drawing`), when the list shows drawings
    /// and it has one.
    pub drawing: Option<&'a Result<Drawing, DrawingProblem>>,
    /// Whether it has text besides its drawings.
    pub has_text: bool,
}

impl AnswerAction {
    /// The action's state under each answer. Without it the action is under every answer
    /// that can be applied, enabled as [`AnswerAction::enabled`] says.
    pub fn state_for(self, state: impl Fn(&AnswerFacts<'_>) -> ActionState + 'static) -> Self;
    /// Its state under the answer of `facts` (`enabled` false wins).
    pub fn state(&self, facts: &AnswerFacts<'_>) -> ActionState;
}

impl MessageListOptions {
    /// Show drawings (2.3): each block of an answer as its line; the facts carry drawings.
    pub fn drawings(self, on: bool) -> Self;
}

impl MessageList {
    /// What the list knows of answer `id`, `None` for another message.
    pub fn facts(&self, id: MessageId) -> Option<AnswerFacts<'_>>;
}
```

`AnswerView` caches the drawing and `has_text` with its fingerprint. A disabled action's
button carries its reason as the tooltip, its menu item is dimmed with the reason after
its label; `invoke_action` acts only on `Enabled`. The keys of the side chat (an action's
`key_of`) are unchanged in kind.

**`src/panel/`**:

```rust
/// The name of the drawing action of an answer.
pub const ACTION_DRAW: &str = "draw";

#[non_exhaustive]
pub enum AssistantPanelEvent {
    // the existing variants, and:
    /// The drawing of an answer is to be applied: on the open whiteboard
    /// (`DrawingTarget::OpenWhiteboard`), or as a new whiteboard (`NewWhiteboard`).
    Draw {
        message: MessageId,
        target: DrawingTarget,
        drawing: Drawing,
    },
}

impl AssistantPanel {
    /// Says what drawings the open item offers (the host calls it with `set_document`,
    /// and keeps it in step as it keeps `set_apply_state`). Sets the target of the shown
    /// chat's session and of every chat made from then on, the drawing action's label,
    /// tooltip and state, and the empty state.
    pub fn set_drawing_target(&mut self, target: DrawingTarget, cx: &mut Context<Self>);
    /// What the host said.
    pub fn drawing_target(&self) -> DrawingTarget;
    /// Tells the host to apply answer `message`'s drawing ([`AssistantPanelEvent::Draw`]).
    /// Nothing happens for an answer that is being written or has no usable drawing for
    /// the target; then a toast says why, when `say` (the key, the command). Returns
    /// whether the host was told.
    pub fn draw(&mut self, message: MessageId, say: bool, cx: &mut Context<Self>) -> bool;
    /// What `cmd-shift-d` and the command do: [`AssistantPanel::draw`] with the marked
    /// answer while the keyboard is in the message list, else with the latest answer.
    pub fn draw_target_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool;
    /// Puts `text` into the composer when it is empty (the keyboard stays where it is).
    /// Returns whether it did. The host offers the writer a message to send.
    pub fn offer_message(&mut self, text: &str, cx: &mut Context<Self>) -> bool;
}
```

- `sync_actions` puts the drawing action first: `AnswerAction::new(ACTION_DRAW, "Draw")
  .tooltip("Draw on Whiteboard")` for `OpenWhiteboard`, `("New Whiteboard",
  "New Whiteboard from Answer")` for `NewWhiteboard`, `.key_of(DrawAnswer).state_for(..)`:
  `Hidden` without a drawing or for `DrawingTarget::None`; `Disabled(reason)` for a
  problem or a drawing that does not fit the target (`drawing_fits`); `Enabled` otherwise.
  Insert and Replace get `state_for`: `Disabled("The answer has nothing but a drawing")`
  when `!has_text`, else as today. The panel's list is made with `.drawings(true)`.
- `apply` (Insert, Replace, Copy) sends `without_drawings(text)` as `markdown`.
- **Toasts of the key and the command** (`draw(.., say: true)`): no answer, or the target
  answer has no drawing: "The answer has no drawing."; a problem: "Nothing was drawn:
  {reason}."; `DrawingTarget::None`: "Open a whiteboard or a document to draw the answer."
- **The empty state** follows the target. `OpenWhiteboard`: title "Ask about this
  whiteboard", text "The assistant reads the whiteboard's elements as they are when you
  send a message. It can propose a drawing; you decide whether it goes on the
  whiteboard.", examples "Group these into frames by topic.", "Add a legend that explains
  the colours.", "What is missing from this diagram?"; the composer's placeholder "Ask
  about this whiteboard". Documents keep theirs.
- `render.rs`: the root's `on_action(DrawAnswer)` and the global listener
  (`global!(DrawAnswer => …draw_target_answer…)`), as InsertAtCaret has.
- `debug_state` gains `draw=whiteboard|new|none`.

### 4.4 `innerword-app`

- **`workspace/assistant.rs`**: `fn drawing_target_now(&self, cx: &App) -> DrawingTarget`
  (`DrawingTarget::of(kind of the open item, pages::engine(cx).is_some())`), set on the side
  chat in `assistant_document_changed` (right after `set_document`) and kept in step in
  `follow_the_text`; `on_side_chat_event` handles `AssistantPanelEvent::Draw` by target.
- **`workspace/assistant_draw.rs`** (new):

  ```rust
  /// The label of the version kept before an answer's drawing.
  pub const ASSISTANT_DRAWING_LABEL: &str = "Before the assistant's drawing";

  impl Workspace {
      /// "Draw on Whiteboard" (section 5). Nothing happens at once; the drawing is applied
      /// after the page was asked for what it holds.
      pub fn draw_answer_on_board(&mut self, drawing: Drawing, window: &mut Window, cx: &mut Context<Self>);
      /// "New Whiteboard from Answer" (section 6): the new whiteboard's id, opened.
      pub fn new_board_from_answer(&mut self, drawing: Drawing, window: &mut Window, cx: &mut Context<Self>) -> Option<DocumentId>;
  }
  ```

  with the sentences of 5.5 and 6 as `pub const`s, so tests compare against them.
- **`workspace/board.rs`**: `BoardState::assistant_drawing: Option<PendingDrawing>`
  (`{ seq: u64, sentence: String }`), read in `board_adopted` and `board_adopt_failed`
  (5.4); `pub(crate) fn board_view_bounds(&self, center: &calls::Center) -> Option<Bounds>`
  (5.3).
- **`workspace/render.rs`**: `capture_action(DrawAnswer)` brings the side chat, as for
  `NewChat` (`bring_the_side_chat_for_its_command`).
- **`workspace/mod.rs`** (the palette): `DrawAnswer` left out while a page or nothing is
  open (`commands_not_for_the_open_page`, and the arm with nothing open), and
  `board_title_of` names it "Draw answer on whiteboard".
- **`boards.rs`** (the fake engine): a lever `FakePages::set_center(x, y)` (what `center`
  answers; today `(0, 0)`), and `zoom` in the fake status as tests set it (`set_status`).
- **Scripted runs**: in a run with `INNER_WORD_AUTOMATION` the workspace prints one line per
  drawing, `[automation] draw: 2 added, 1 changed, 0 removed, version kept, placed as
  written, change 3.1 ms, adopt answered in 40 ms` (or `draw: nothing: <reason>`), and
  `draw: new board:14 “Who serves whom” in Part one, 5 added, change 2.0 ms`.

## 5. Drawing on the open whiteboard

### 5.1 The steps

`Workspace::draw_answer_on_board(drawing)`:

1. **Can it be done?** A whiteboard is open (`open_board_id`), else "Open a whiteboard to
   draw on it." Its page answers (`board_ready()`), else `board_not_ready_sentence` (still
   loading, not drawn, stopped). Another drawing of the assistant is under way on it, or
   an adopt is in flight (`board.adopting`): "The whiteboard is taking in another change.
   Press again in a moment." One drawing at a time.
2. **Ask the page** (a task of the window, as `add_to_board` does): `collect` when the
   status says the page holds more (`board.status.holds_more()`), awaited at most
   `VIEW_COLLECT_PATIENCE`; then `center`, awaited at most `VIEW_COLLECT_PATIENCE`. The
   calls run in order in the page, so `center` reads the canvas after the collect.
3. **In one update** (no await from here to the adopt call, so no gesture, agent request
   or key comes between):
   - the whiteboard must still be the open one (else nothing, silently: the writer went
     elsewhere);
   - `save_board_now(id)`: the scene the page handed over is saved (a flush); when it
     cannot be: "Nothing was drawn: the whiteboard as it is now could not be saved first."
   - `S` = the stored scene (`board.base`, the flush's), `stamp` = `board.stamp`;
   - the view (5.3), `making` = `HashedMaking::new(&[stamp hex, rev, now, drawing.to_json()],
     now)` (as the Add commands seed theirs);
   - `S2 = S.clone(); innerword_board::draw(&mut S2, &drawing, view, &mut making)`; an error
     ends here with its toast (5.5): no version, no save, nothing in the view;
   - `library.change_outside(|store| { store.create_labeled_snapshot(id,
     ASSISTANT_DRAWING_LABEL)?; store.save_board(id, &prepared(S2).content(), Some(stamp)) })`
     (`super::saver::prepared_of`, as Restore prepares its scene), reporting
     `OutsideChange::SnapshotsChanged(id)` and `OutsideChange::DocumentSaved(id)`; a
     snapshot that fails ends here and nothing is saved; `Stale` (cannot happen after the
     flush in the same turn, but is said): "Nothing was drawn: the whiteboard changed while
     the drawing was made. Press again.";
   - `adopt_stored_scene(id, &S2 content, true, cx)`: the delta from `S` to `S2`, by the base
     rule, **one step of Excalidraw's undo**, the posts of the view held until it answers
     (4.6 of the whiteboard design); `board.assistant_drawing = Some(PendingDrawing { seq:
     board.adopt_seq, sentence })`;
   - `give_the_board_the_keyboard`: `cmd-z` next takes the drawing out;
   - when `Drawn::placement` says `in_view: false` with a view known:
     `zoom_board(calls::Zoom::Fit)`.
4. **When the view answers** (`board_adopted` with that `seq`): the toast (5.5), with the
   conflicts the view reports; `Undo` offered while the canvas is at the answered rev. When
   the adopt fails, the existing path runs (the merge in Rust, a reload of the view) and the
   toast says "Drew on “Plan”. The whiteboard was loaded again to show it." without Undo.

This is the path of Restore (6.5) and of an agent's change (6.7) with the writer as the one
who asked: the library has the drawing the moment it is applied, a version has the
whiteboard as it was, and the view merges it by the base rule. The Add commands'
`insertElements` path is not used: it has no removals, no base rule against a gesture
made between the collect and the call, and no version.

### 5.2 Ids

- **A removal of an element that is gone** is left out: what it asked for is so. The
  toast counts it ("1 element to remove was gone already.").
- **A change of an element that is gone, or a new element that names one** (an arrow's
  `start` or `end`, a frame's `children`): nothing is drawn, because half a diagram (an
  arrow with no end) is worse than none. The toast names the ids (at most five, then "and
  N more") and says what to do: the assistant is sent the whiteboard as it is with the
  next message, so asking again works.
- **An id of a new element that the whiteboard has** (the assistant reused "title" in a
  second drawing; the writer's own ids are Excalidraw's random ones) is renamed with its
  references inside `add`, never refused: the writer cannot fix an id.
- **Labels**: the listing names a shape, not its label; a change of `text` on a shape
  changes its label (7.3), and removing a shape removes its label.

### 5.3 Placement

A drawing **stands on its own** when it has no `update` and no `remove` and its adds name
nothing that was on the whiteboard. Only such a drawing is ever moved, as a whole, by one
offset (a rigid move: its arrows' points are relative, its bindings and frames are among
its own elements). A drawing that touches what is there stays where the answer put it.

**The view** in scene coordinates: Excalidraw's `center` answers the middle of the view
`(cx, cy)`; the page's status has the canvas zoom `z`; the page element's bounds of the
last frame (`PageSession::bounds`) give its size `w x h` in CSS pixels. So `view = (cx -
w/2z, cy - h/2z, w/z, h/z)`; `None` when one of them is missing.

`place(group, occupied, view)`, with `m = PLACE_MARGIN` (40), `gap = PLACE_GAP` (100),
`occupied` the bounds of every element that was on the whiteboard (`element_bounds`,
rotation included), and "free" meaning that no occupied rectangle meets the rectangle
grown by `m`:

1. The group where it was written is free and wholly in the view: it stays (`dx = dy = 0`,
   `in_view: true`).
2. Otherwise, places in the view: the top left corners at which the group lies inside the
   view shrunk by `m`, on a grid of step `max(16, min(view.width, view.height) / 24)` (the
   step grown until there are at most 2,500 places), together with the corner as written
   clamped into that range; ordered by the distance to the corner as written (clamped),
   then by `y`, then by `x`. The first free one is taken (`in_view: true`). Only occupied
   rectangles that meet the view grown by `m` are tested.
3. Otherwise (the group is larger than the view, or the view is full): where it was
   written when that is free, else beside the drawn area (`x = drawn.right + gap`, `y =
   drawn.top`); `in_view: false`, and the app zooms to fit (5.1).

Without a view (unknown; a new whiteboard): step 1's "in the view" is dropped and steps 2
are skipped: as written when free, else beside the drawn area, `in_view: false`, and no
zoom. On an empty whiteboard everything is free: as written when in view, else the
nearest place in view.

**Why not always as written:** the assistant knows where the drawing is (the listing's
drawn area) but not where the writer is looking, and a drawing that lands outside the view
looks like a button that did nothing. **Why not always the middle of the view:** "a legend
below the diagram" means below the diagram; where it is free and seen, it stays.

### 5.4 The undo, the version, the keyboard

- **One step of Excalidraw's undo**: `adopt` with `undoable: true` is one `updateScene`
  with `CaptureUpdateAction.IMMEDIATELY` (the glue, `edit.js`); Restore relies on the same.
  After it the keyboard is on the canvas, where `cmd-z` is Excalidraw's (a fixed binding
  in `BoardView`), so `cmd-z` right after takes the whole drawing out; the page posts the
  result as a change of the writer's, which the autosave saves compare-and-swap.
- **The toast's Undo** (for the pointer): `undo` with one step, while the status's rev is
  the rev the adopt answered (the writer has not drawn since); later: "Not undone: the
  whiteboard has changed since. Version History has it as it was." The keyboard way is
  `cmd-z` on the canvas and the Edit menu's Undo.
- **A version is kept before every drawing**, labelled `ASSISTANT_DRAWING_LABEL` ("Before
  the assistant's drawing"), as an agent's change keeps one ("Before a drawing by
  Claude"): Excalidraw's undo does not outlast a reload or a relaunch, and the version
  history is the undo that lasts. It is the flushed scene, what the writer had.

### 5.5 What is said

The toast of a drawing applied (`Toast::success`, 8 s, button **Undo**):

- "Drew on “Plan”: 2 elements added, 1 changed. The whiteboard as it was is kept as a
  version." (the parts that are not zero: "N elements added", "N changed", "N removed";
  "1 element added"); then, each as a sentence of its own when it applies:
  - "1 element to remove was gone already."
  - the conflicts of the view, in the agents' words: "2 of its changes were not taken,
    because you changed those elements meanwhile: api, reads." (at most five ids, then
    "and 3 more");
  - the converter's warnings: "db sticks out of frame backend: what is outside is not
    shown."
  - placed elsewhere: "It was put where it is free and in view." / beside the drawing:
    "It was put beside the drawing."

Nothing drawn (`Toast::error`):

| Why | Words |
|---|---|
| gone ids (changes) | "Nothing was drawn: the answer changes api and db, which are no longer on the whiteboard. Ask again: the assistant is sent the whiteboard as it is now." |
| gone ids (named by new elements) | "Nothing was drawn: the answer joins arrows or frames to cache, which is no longer on the whiteboard. Ask again: …" |
| the converter refused | "Nothing was drawn: the element at index 3 of add (fillStyle): …" (`BoardError`'s words); and the composer, when empty, is offered "Your drawing could not be used: <the same words>. Please write it again." (`offer_message`), so one `enter` asks |
| the flush failed | "Nothing was drawn: the whiteboard as it is now could not be saved first." |
| stale | "Nothing was drawn: the whiteboard changed while the drawing was made. Press again." |
| busy | "The whiteboard is taking in another change. Press again in a moment." |
| not ready | `board_not_ready_sentence` (still loading, could not be shown, stopped working) |

### 5.6 Limits and cost

The agents' limits hold (`innerword_board::limits`): 2,000 entries a list, 20,000 characters
a text, 20,000 elements and 16 MB of scene after the drawing (a writer's whiteboard over
the agents' limits cannot be drawn on by the assistant: the converter's words say which).
An answer is at most `Transcript::MAX_ANSWER_CHARS` (200,000). The main thread carries what
an agent's change to the open whiteboard carries (CLAUDE.md, "Whiteboards", "What it
costs": on 3,000 elements change 81 ms and window 41 ms), plus the flush; a drawing on a
small whiteboard is a few milliseconds. Placement is at most 2,500 places against the
elements near the view.

## 6. New Whiteboard from Answer

`Workspace::new_board_from_answer(drawing)`:

1. The open item is a document (`session`), whiteboards can be shown (`pages::engine`), and
   the drawing has no changes (`has_changes()` false: the panel offers it only so, and the
   app checks again: "This drawing changes elements of a whiteboard: open that whiteboard
   and ask there.").
2. `Scene::empty()`, `innerword_board::draw(&mut scene, &drawing, None, &mut making)` with
   `making = HashedMaking::new(&[document id, drawing.to_json(), now], now)`; an error:
   "Nothing was drawn: …" (and the composer offered the message, as in 5.5).
3. **Where**: the folder of the open document (`document_meta(id).folder_id`), at its end;
   at the root when that folder is gone (as `new_board` falls back).
4. **Title**: the drawing's `title`, else "<document title> (drawing)"; a chosen name
   (`custom_title` flag, as `new_board` sets it for a title given).
5. `library.change_outside(|store| store.create_board(folder, &title, &prepared(scene)))`
   (the first whiteboard of a library takes it to schema 5, as any).
6. **It opens** (`open_document(id, true, ..)`): the keyboard on its canvas, fitted to its
   drawing (a whiteboard without a stored view opens fitted; if the builder finds it does
   not, `zoom_board(Zoom::Fit)` once the page is ready). The side chat shows the new
   whiteboard's own conversation, empty; the document's conversation was stored with the
   document and comes back when the document is opened. The writer goes on about the
   drawing on the whiteboard itself.
7. The toast: "Made the whiteboard “Who serves whom” in Part one from the answer." (at the
   root: "… from the answer."), with the converter's warnings after it. No Undo: the
   whiteboard is a new item, and Trash removes it.

No version is kept (there was nothing before). The document is not changed. `update` and
`remove` are never applied over a document: a drawing with them is shown, and its button
disabled with the reason (2.4).

## 7. Keyboard and mouse

| Way | Over a whiteboard | Over a document |
|---|---|---|
| The button under an answer with a drawing (first in the row; on the latest answer always, on older ones under the pointer and on the marked one) | "Draw", tooltip "Draw on Whiteboard" | "New Whiteboard", tooltip "New Whiteboard from Answer" |
| The answer's menu (right-click on the answer; `enter`, `ctrl-enter`, `shift-f10` on the marked message) | "Draw on Whiteboard" with its key | "New Whiteboard from Answer" with its key |
| Key, in the side chat (the composer, the answers, the message list, anywhere in the panel) | `cmd-shift-d`: the marked answer, else the latest | the same |
| Palette (category "Assistant"; brings the side chat first when it is closed or behind the comments) | "Draw answer on whiteboard" | "New whiteboard from answer" |
| Menu bar | none (as Insert, Replace and Copy) | none |

- **Over a page, or with nothing open**: no button, the palette leaves the command out, and
  the key says "Open a whiteboard or a document to draw the answer."
- **One action**, `assistant::DrawAnswer`: its meaning follows the open item, as "Rename
  document" is "Rename whiteboard" on a whiteboard. The list of shortcuts shows the base
  title "New whiteboard from answer" (keywords "draw", "whiteboard"), as it shows "Rename
  document".
- **`cmd-shift-d`** is free in the side chat's places: the panel (`AssistantPanel`) and its
  editors (`Editor && assistant_side`) bind nothing on it, the editor and the window
  neither; the list of shortcuts binds it to "Reset all" in `KeymapView`, another place
  (keymap::places keeps them apart). It is a `cmd` chord, so Vim mode lets it through,
  and the whiteboard's own keys are not in play (the keyboard is in the chat).
- **The list of shortcuts**: one row more, "New whiteboard from answer", "In the
  assistant's chats" (the existing place; no new key context, so `keymap::places` and
  `shortcuts::Place::of_predicate` do not change). With the assistant shown 443 rows (was
  442); without it 412 (its rows are not listed); the shortcuts file 426 entries (two key
  contexts, as InsertAtCaret has; was 424). C confirms the three numbers with a scripted run
  (`keys:cmd-/; state`, with and without `INNER_WORD_ASSISTANT_FAKE=ready`, and the file of
  "Export…") and writes them into CLAUDE.md and the tests that hold them.
- **Product rules**: the button and its menu item are for the pointer, the key and the
  palette for the keyboard; every key is asked of the keymap (the button's hint is
  `key_hint_at`, the toast names no key); the words are plain.

## 8. Safety

The answer is untrusted text: a document, a page's text or another whiteboard the assistant
read with its tools can carry instructions written to steer it. What a drawing can and
cannot do:

- **Nothing without the writer's press**: no drawing is applied when the answer arrives,
  when it is shown, or when the chat is opened; only the button, its menu item, its key or
  its command apply it.
- **Only the open item**: the open whiteboard, or one new whiteboard beside the open
  document. A drawing cannot name another whiteboard, a document, a folder or the library.
- **What it does is shown first, by the app**: the italic line is written from the block,
  so a block that removes forty elements says "removes 40 elements (…)" whatever its prose
  claims; the button's tooltip says what it applies to.
- **It is taken back in one step**: one step of Excalidraw's undo, and a version kept
  first (5.4).
- **No picture, no file, no address read**: an `image` element is refused when the block is
  read; the converter is given no pictures; nothing in a drawing is a path the app would
  open, and nothing is fetched. An existing picture can be moved, resized or removed by
  id, which the writer sees in the summary.
- **Links** are kept as text on an element, `http`, `https` and `mailto` only (the
  converter refuses `javascript:`, `file:`, element links); a link opens in the browser
  only by the reader's own act, as every link of a whiteboard.
- **Nothing runs**: elements are data; Excalidraw draws text as text; `customData`,
  `iframe`, `embeddable`, `magicframe` and the fields only Excalidraw sets are refused by
  the converter (7.2); the whiteboard's view has no subframe and no eval.
- **Bounded**: the agents' limits (5.6).
- **The tools stay read-only**: the assistant gains no tool; drawings are text in an
  answer. The MCP server's table, `read_only_tools` and the isolation of the engine do not
  change.

## 9. Tests

### 9.1 `innerword-board` (`tests/suite/drawing.rs`, new module of the one test binary)

- `a_drawing_reads_as_an_object_or_an_array`
- `what_is_not_a_drawing_says_why`: not JSON (line and column), an unknown key, `add` not a
  list, an id in `remove` not a string, empty, an image, 2,001 adds; each `Display` as 2.4
- `blocks_join_in_order_with_the_first_title`, and over the limit after joining
- `a_drawing_adds_changes_and_removes_whole_or_not_at_all` (a bad change leaves the scene
  as it was)
- `removing_what_is_gone_is_left_out`, `changing_what_is_gone_draws_nothing`,
  `an_arrow_to_what_is_gone_draws_nothing`
- `an_id_the_whiteboard_has_is_renamed_with_its_references` (`add` renamed, `update` and
  `remove` keep naming the whiteboard's)
- placement: `a_drawing_that_covers_the_drawing_moves_to_the_nearest_free_place_in_view`,
  `a_drawing_written_free_and_in_view_stays`, `a_drawing_written_outside_the_view_comes_into_it`,
  `a_drawing_that_names_what_is_there_never_moves`, `a_drawing_larger_than_the_view_goes_beside_the_drawing`,
  `without_a_view_a_drawing_stays_where_it_is_free`, `an_empty_whiteboard`, and `place`
  on bare rectangles (the order of places, the margin, the step, at most 2,500 places)
- `the_same_drawing_gives_the_same_elements` (determinism of ids and seeds)
- `the_warnings_of_frames_are_said_after_the_move`

### 9.2 `innerword-assistant` (`tests/suite/drawing.rs` new; `prompts.rs` unit tests)

- blocks: backticks and tildes, `Whiteboard` and `whiteboard json` as info strings, a
  longer fence, three spaces of indent, a `whiteboard` fence inside a four-backtick block
  is not one, an unclosed block, two blocks, the byte ranges
- `answer_drawing`: none, usable, joined, a problem with its ordinal; `drawing_fits` for the
  three targets
- `without_drawings` and `shown_markdown` (finished, unusable, streaming, not closed after
  the end); `drawing_sentence` (plurals, "a"/"an", three ids then "and N more", a title)
- prompts: the section of each target is in `side_system_prompt` and only there (a page,
  nothing, the quick prompt: none), with and without tools; the budget (4,400 and 3,400
  characters); **the examples in the prompts draw** (3.5); the existing three tests still
  hold (the whiteboard sentence they quote is kept)
- session: `FakeBackend::last_started().system_prompt` has the section for the target; a
  change of target restarts the conversation with its history; `prepare` after
  `set_drawing_target` starts with the target's prompt
- history: `Transcript::history` has "(A drawing: …)" for a block, the stored transcript
  the block

### 9.3 `innerword-assistant-ui` (`tests/suite/panel.rs`, `components.rs`, `markdown.rs`, `rebinding.rs`)

- an answer with a drawing shows its line, never its JSON; while it is written "Drawing…"
  and no actions; the quick chat's list (drawings off) shows the block as code, as today
- the drawing button: only under answers with a drawing, first in the row, its label and
  tooltip by target, hidden for `None`, dimmed with the reason for an unusable drawing and
  for changes over a document; the answer's menu item likewise
- the button, the menu item and `cmd-shift-d` emit `Draw { message, target, drawing }` with
  the parsed drawing; the marked answer, else the latest; the global action reaches a shown
  panel from elsewhere in the window
- the key's toasts: no drawing, a problem, no target
- Insert, Replace and Copy carry the answer without its blocks, and are dimmed under an
  answer that is only a drawing
- the empty state and the placeholder over a whiteboard; `offer_message` fills only an empty
  composer; `debug_state` has `draw=`
- rebinding: `DrawAnswer`'s key changed with `keymap::replace_key` is the key of the button's
  hint and works, the old one does nothing

### 9.4 `innerword-app` (FakeBackend and the fake page engine only)

`tests/suite/boards.rs`:

- `the_assistant_draws_on_the_open_whiteboard_in_one_step_of_its_undo`: a reply with a
  drawing over a whiteboard, the button pressed: the stored scene has the elements, a
  version labelled "Before the assistant's drawing", the fake's `adopt` was called once
  with `undoable: true`, the keyboard is on the canvas, the toast's words; the Edit menu's
  Undo then calls `undo 1`, and the canvas is as before
- `a_drawing_waits_for_what_the_page_holds` (`draw_unposted`, then the press: `collect`
  first, the writer's stroke kept beside the drawing)
- `a_drawing_that_changes_what_is_gone_draws_nothing` (no version, no adopt, the toast)
- `a_drawing_the_converter_refuses_offers_the_message` (the composer's text; a composer
  with text is left alone)
- `a_drawing_that_covers_the_drawing_lands_free_in_view` (`set_center`, a zoom in the
  status, the page element's bounds) and `one_larger_than_the_view_zooms_to_fit`
- `conflicts_are_said_once_the_view_answered` (the fake's base rule with a writer's change
  between the collect and the adopt)
- `the_toast_offers_undo_while_the_canvas_has_not_moved_on`
- `a_whiteboard_that_is_not_ready_is_not_drawn_on`, `one_drawing_at_a_time`
- `without_a_page_engine_no_drawing_is_offered` (target `None`; the system prompt without the
  section)

`tests/suite/assistant.rs`:

- `an_answer_over_a_document_makes_a_new_whiteboard_in_its_folder` (title from the block and
  without one, the folder, opened, the keyboard on the canvas, the document's conversation
  stored and back when the document is opened again)
- `a_drawing_that_changes_a_whiteboard_is_not_offered_over_a_document`
- `insert_at_caret_leaves_the_drawing_out`
- `the_side_chat_is_told_the_drawing_format_of_its_open_item` (`last_started` per kind: a
  document, a whiteboard, a page)
- the palette: the title per item, left out over a page; `commands_of_the_assistant` counts
  one more by itself

`tests/suite/keybindings.rs` and `pages.rs`: the counts of 7, the row's place, and
`two_rows_under_one_place_name_are_in_the_same_place` and
`every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file` still pass; `mouse.rs`: a click
on the "Draw" button (`assistant-action-draw-<id>`) draws; `context_menus.rs`: the answer's
menu has the item and it works.

### 9.5 The scripted acceptance run

The fake engine with a replies file; screen recording is permitted now, and `screenshot:`
captures only the app's own window (by its window number, `screenshot::capture_window`).

`$T/replies.txt`:

````text
I put the database to the right of the API and joined them with an arrow labelled "reads"; the API is filled in blue.

```whiteboard
{"add": [
  {"type": "ellipse", "id": "db", "x": 420, "y": 80, "width": 160, "height": 90, "label": {"text": "Postgres"}},
  {"type": "arrow", "id": "reads", "x": 300, "y": 125, "start": {"id": "api"}, "end": {"id": "db"}, "label": {"text": "reads"}}],
 "update": [{"id": "api", "backgroundColor": "#a5d8ff", "fillStyle": "solid"}]}
```
====
Here is who serves whom in the chapter.

```whiteboard
{"title": "Who serves whom", "add": [
  {"type": "ellipse", "id": "keeper", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "The keeper"}},
  {"type": "ellipse", "id": "ada", "x": 320, "y": 0, "width": 180, "height": 80, "label": {"text": "Ada"}},
  {"type": "arrow", "id": "serves", "x": 180, "y": 40, "start": {"id": "keeper"}, "end": {"id": "ada"}, "label": {"text": "serves"}}]}
```
====
````

Run 1, on a whiteboard (the debug build; then once in release for the cost line):

```bash
T=$(mktemp -d)
INNER_WORD_DATA_DIR=$T INNER_WORD_ASSISTANT_FAKE=ready INNER_WORD_ASSISTANT_FAKE_REPLIES=$T/replies.txt \
INNER_WORD_AUTOMATION='wait:1500; board:new:Plan; board:wait; board:draw:[{"type":"rectangle","id":"api","x":100,"y":80,"width":200,"height":90,"label":{"text":"API"}}]; wait:600; keys:cmd-3; type:Add the database the API reads from and make the API blue.; keys:enter; wait:800; state; screenshot:'$T'/1-answer.png; keys:cmd-shift-d; wait:800; board:scene; state; screenshot:'$T'/2-drawn.png; board:press:cmd-z; wait:600; board:scene; screenshot:'$T'/3-undone.png; keys:cmd-shift-p; type:version history; keys:enter; wait:400; screenshot:'$T'/4-versions.png; keys:escape; theme:dark; keys:cmd-3; wait:300; screenshot:'$T'/5-dark.png; quit' \
./target/debug/inner-word
```

Asserted from the output: the `state` after the answer has the side chat's `draw=whiteboard`
and the answer's actions; `[automation] draw: 2 added, 1 changed, 0 removed, version
kept, placed as written`; `board:scene` lists `db` and `reads` and `api` with `fill
#a5d8ff solid`; after `cmd-z` the listing is the rectangle alone; the version history's
row says "Before the assistant's drawing". Looked at: the italic line and the "Draw" button
in 1, the drawing on the canvas and the toast in 2, the dark theme in 5.

Run 2, on a document:

```bash
INNER_WORD_DATA_DIR=$T2 INNER_WORD_SEED_DEMO=1 INNER_WORD_ASSISTANT_FAKE=ready INNER_WORD_ASSISTANT_FAKE_REPLIES=$T/replies-doc.txt \
INNER_WORD_AUTOMATION='wait:1500; keys:cmd-p; type:chap 2; keys:enter; wait:400; keys:cmd-3; type:Draw who serves whom.; keys:enter; wait:800; screenshot:'$T2'/1-answer.png; keys:cmd-shift-d; board:wait; board:scene; state; screenshot:'$T2'/2-new-board.png; quit' \
./target/debug/inner-word
```

(`replies-doc.txt` is the second answer alone.) Asserted: `draw: new board:N “Who serves
whom” in <the chapter's folder>`; `state` has `board=[… elements=3 …]`, `focus=board`;
the sidebar has the whiteboard in the chapter's folder. Never script Copy (the user's
clipboard) and never `board:press:cmd-c`.

Release cost: run 1 on a whiteboard of 3,000 elements made by the tool (the whiteboard
design's 9.4 script) and read the `draw:` line's `change` and the adopt's time; record
them in CLAUDE.md beside the agents' cost.

## 10. CLAUDE.md lines that change (package C)

- **Workspace table**: `innerword-board`: "…; an answer's drawing (`Drawing`: `add`,
  `update`, `remove` in the agents' formats; `draw`, whole or not at all, ids that are gone,
  ids renamed, a drawing that stands on its own placed free and in view, `place`)".
  `innerword-assistant`: "…, the drawings of answers (`drawing`: the fenced `whiteboard`
  block found and read with `innerword_board::Drawing`, `DrawingTarget`, the lines the chat
  shows), the system prompts (by the open item's `DrawingTarget`) …". `innerword-assistant-ui`:
  "the answer actions (Insert at Caret, Replace Selection, Copy, Draw)". `innerword-app`:
  "the assistant in the app (…, what is applied to the text, an answer's drawing on the open
  whiteboard or as a new whiteboard, `workspace/assistant_draw.rs`)".
- **Dependency direction**: `assistant → {mcp, agent-setup, board}`, with "for the format of
  an answer's drawing".
- **"The assistant"**: a new bullet **"Drawings."** (sections 1, 2, 5, 6 and 8 in short: the
  block, the line in the chat, the button by item, `cmd-shift-d`, the version and the undo,
  the ids, placement, New Whiteboard from Answer, what a drawing cannot do), and in
  "Whiteboards." of that section: "Insert at Caret and Replace Selection are off, Copy works,
  Draw on Whiteboard applies an answer's drawing" in place of "… are off, Copy works".
- **"Whiteboards"**: a line under "The version history" for the label "Before the assistant's
  drawing"; "Not in this version" loses "the assistant changing a whiteboard" and gains "a
  picture in an answer's drawing, a drawing for another whiteboard than the open one, the
  quick chat's drawings, an answer's drawing over a page".
- **Keys**: "In the assistant's chats" gains "`cmd-shift-d` draws the answer (on the open
  whiteboard, or as a new whiteboard beside the open document)".
- **"Changing keys"**: the counts (443 rows, 412 without the assistant, 426 entries) with
  "and the assistant's drawing, one row and two entries" in the account of how they grew.
- **The tests paragraph** of "Build and test": `assistant.rs` and `boards.rs` hold an
  answer's drawing (the new whiteboard, and the open one).
- **"What it costs"** (Whiteboards): the release cost of run 1 on 3,000 elements.
- `docs/whiteboard-design.md`, 6.6: one paragraph that points here ("Built later: the side
  chat proposes drawings, `docs/assistant-draws-design.md`").

## 11. The build plan

Three packages; each builder edits only its crates (CLAUDE.md, "Several agents at once:
stay in your crate"), commits its paths only, and leaves `cargo test -p <crate>` passing
and `cargo build -p innerword-app` compiling.

### Package A: `innerword-board` + `innerword-assistant` (pure, `CARGO_TARGET_DIR=target-pure`)

- **A1, first, committed as soon as it is done** (B builds against it): `innerword-board`'s
  `Drawing`, `DrawingError`, `from_json`, `join`, `is_empty`, `has_changes`,
  `added_types`, `changed_ids`, `removed_ids`, `to_json`, with their tests; the signatures of
  `draw`, `place`, `Drawn`, `Placement`, `DrawError`, `PLACE_MARGIN`, `PLACE_GAP` are part of
  A2. `innerword-assistant`'s dependency on `innerword-board` and the whole `drawing` module
  (`DRAWING_INFO`, `DrawingTarget`, `DrawingBlock`, `DrawingProblem`, `drawing_blocks`,
  `answer_drawing`, `drawing_fits`, `without_drawings`, `shown_markdown`,
  `drawing_sentence`) with its tests, and the re-exports. Everything working, nothing
  stubbed.
- **A2**: `draw` and `place` with their tests; the prompts (3.2 to 3.5, `drawing_prompt`,
  `side_system_prompt(tools, target)`, the budget test, the examples drawn); `ChatSession`'s
  target; `Transcript::history`.
- Done when `CARGO_TARGET_DIR=target-pure cargo test -p innerword-board` and `-p
  innerword-assistant` pass, and the crates that call `side_system_prompt` compile (only
  `session.rs` and its tests do).

### Package B: `innerword-assistant-ui` (GPUI, `target/`)

- Starts at once with what needs nothing of A (the action, its keys and command,
  `ActionState`, `AnswerAction::state_for`, the per-answer cache), then uses A1's items
  (`answer_drawing`, `shown_markdown`, `without_drawings`, `drawing_fits`, `DrawingTarget`,
  `Drawing`) once A1 is committed. It needs nothing of A2.
- Everything of 4.3, its tests (9.3).
- Done when `cargo test -p innerword-assistant-ui` passes and `cargo build -p innerword-app`
  compiles (the app matches `AssistantPanelEvent` with a `_` arm, so the new variant does
  not break it; `side_system_prompt`'s new signature is A's and is not called by the app).

### Package C: `innerword-app` + CLAUDE.md (after A and B)

- Everything of 4.4, sections 5 and 6, the fake's lever, its tests (9.4), the scripted runs
  and screenshots (9.5), the counts of 7 confirmed, CLAUDE.md (10) and the pointer in the
  whiteboard design.
- Done when `cargo test -p innerword-app` passes, the scripted runs give what 9.5 says, and
  `df -h .` was looked at before the first build (CLAUDE.md, "The disk is the limit").

### The interfaces between them

| From | To | What |
|---|---|---|
| A (board) | A (assistant), C | `Drawing` (and its methods), `DrawingError`, `draw`, `Drawn`, `DrawError`, `place`, `Placement`, `PLACE_MARGIN`, `PLACE_GAP` (4.1) |
| A (assistant) | B, C | `DrawingTarget::of`, `answer_drawing`, `drawing_fits`, `without_drawings`, `shown_markdown`, `drawing_sentence`, `DrawingProblem::reason`, `ChatSession::set_drawing_target` / `with_drawing_target` / `drawing_target`, the re-exported `Drawing` (4.2) |
| B | C | `DrawAnswer`, `ACTION_DRAW`, `AssistantPanelEvent::Draw { message, target, drawing }`, `AssistantPanel::set_drawing_target` / `drawing_target` / `draw` / `draw_target_answer` / `offer_message`, `ActionState`, `AnswerFacts`, `AnswerAction::state_for`, `MessageListOptions::drawings`, `debug_state`'s `draw=` (4.3) |
| C | the writer | the behaviour of sections 1, 5, 6 and 7 |

Nothing changes in `innerword-board-web` (no glue change, no new call: placement uses
`center`, the status's zoom and the page element's bounds; the undo is `adopt`'s), in
`innerword-agent`, `innerword-cli`, `innerword-mcp`, the skill, or `innerword-assistant-quick`.

## 12. Not in this version, and what only a person can check

- **Not in this version**: a picture in an answer's drawing (not even one the whiteboard
  has, as a new element; moving or resizing an existing picture by id works); a drawing for
  another whiteboard than the open one; drawings over a page and in the quick chat; a
  drawing that updates a whiteboard from a document's chat; selecting the drawn elements
  after an adopt (the glue's `adopt` does not select; `insertElements` does, without
  removals or the base rule); scrolling the view to a drawing without changing the zoom
  (a glue call would; the zoom to fit is taken when no place in view is free); copying a
  drawing's JSON; elbow arrows and element links from the assistant (refused as for
  agents); the assistant telling the writer the cost of a drawing.
- **Checked by a person only**: that a real model writes usable blocks from these prompts.
  No test and no scripted run calls the real Claude Code (CLAUDE.md, "Tests never call the
  real Claude Code"); the person checks it with the bundled app, a few messages over a
  whiteboard and a document, with the cheapest model, and reports what the blocks needed.
  Also by a person: `cmd-z` on the canvas in a real active key window right after a drawing,
  the toast's Undo with the pointer, the button under an answer with the pointer and its
  tooltip.
- **Risks**: (1) a model that writes JSON with comments or trailing commas: the block is
  refused with the place named, and the offered message asks it to write it again; if real
  use shows it often, a tolerant reader (comments and trailing commas only) is the next
  step. (2) The placement's view comes from three sources (`center`, the status's zoom, the
  element's bounds); a frame between them where the writer zooms is possible and harmless
  (the drawing lands free, a little off the middle). (3) The system prompt grows by about
  1,100 tokens on a whiteboard; it is sent once per process.
