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
| 1 | How an answer carries a drawing | One fenced code block with the info string `whiteboard`, holding one JSON object `{"title"?, "add"?, "update"?, "remove"?}` in the agents' formats (a bare JSON array is read as `{"add": [...]}`). Unknown keys, an `image` element, a `type` the converter does not make, an id that is not 1 to 64 of `[A-Za-z0-9_-]` and more than 2,000 entries in a list make it unusable when it is read. |
| 2 | Several blocks in one answer | Taken together, in order, as one drawing (adds, then changes, then removals; the first `title`). The prompt asks for one block per answer and no alternatives as separate blocks. |
| 3 | What the chat shows for a block | Never the JSON. Each block is replaced in the shown answer by one line in italics that the app writes, its counts first: "Drawing: adds 2 ellipses and 3 arrows; changes the rectangle “API”; removes a picture; links to example.com." (over a document the title after the counts: "… (new whiteboard “Who serves whom”)."), its problem ("Drawing that cannot be used: …"), or "Drawing…" while it is still being written. Every word of the model's in it (a title, a label, a host) is escaped Markdown. |
| 4 | Streaming | The button is offered only under an answer that is not being written and whose blocks are closed and read. |
| 5 | Who reads the block | `innerword-board` owns the format (`Drawing`, `Drawing::from_json`, `draw`); `innerword-assistant` finds the blocks and reads them with it (a new edge `assistant → board`: pure, no cycle, already in its build through `mcp → agent → board`); `innerword-assistant-ui` asks `innerword-assistant` only; the app applies with `innerword-board`. |
| 6 | The prompts | The side chat's system prompt is chosen by the open item: a whiteboard adds the section "Drawing on the open whiteboard", a document "Drawing a new whiteboard", a page and nothing add nothing. At most 4,600 and 3,600 characters (about 1,150 and 900 tokens, in every request of the conversation, cached), each with one worked example that a test draws on a listing made by `Scene::listing`. The target travels with the item (`DocumentRef`), so each chat starts with its own prompt. |
| 7 | Applying on an open whiteboard | As a restored version is applied (6.5 of the whiteboard design): marked "under way" at the press, collect, then in one update everything checked again, flush, `innerword_board::draw` on the stored scene, and only when it changes something: a version labelled "Before the assistant's drawing" and the save compare-and-swap in one closure (the version deleted again when the save fails), `adopt` the delta with `undoable: true` (one step of Excalidraw's undo, the base rule against a gesture made meanwhile); the keyboard goes to the canvas and the toast with Undo comes **when the view answered** that adopt. |
| 8 | Ids that are gone | A removal of an element that is gone is left out (it is gone). A change of a gone element, or an element that names one (an arrow's end, a frame's child, in `add` or in `update`), draws nothing and the toast names the ids. An id the answer gives a new element that the whiteboard has is renamed (`api` → `api-2`, free against everything, at most 64 characters) with its references inside `add`. A change that sets what an element has is left out (the agents' rule, moved into `innerword-board`); a drawing left with nothing draws nothing and says so. |
| 9 | Placement | As written, except a drawing that stands on its own (only `add`, naming nothing on the whiteboard): when it would cover the drawing or lie outside the view, it is moved as a whole to the free place in view nearest to where it was written; when no place in view is free, beside the drawn area, and the view zooms to fit. The view is the page's status (scroll and zoom) and the page element's size. |
| 10 | New Whiteboard from Answer | In the document's folder (the root when the document or its folder is in the trash), titled with the block's `title` (else "<document title> (drawing)", "Drawing" for an untitled document), opened with the keyboard on its canvas; the document's conversation stays the document's, and the toast has "Back to “Chapter 2”". Offered only for a drawing that only adds; `update` or `remove` over a document disable the button with a reason. |
| 11 | Keys and commands | One action `assistant::DrawAnswer`, key `cmd-shift-d` in the side chat (`AssistantPanel` only; the marked answer, else the latest). Button "Draw" (tooltip "Draw on Whiteboard") over a whiteboard, "New Whiteboard" (tooltip "New Whiteboard from Answer") over a document, first in the button row and last in the answer's menu (so `enter enter` on a marked answer stays Insert at Caret); palette "New whiteboard from answer", renamed "Draw answer on whiteboard" while a whiteboard is open, and left out where no drawing can be applied; with the side chat not shown the palette command only brings it with the answer marked, and the next one draws; no menu bar item (Insert, Replace and Copy have none either). The list of shortcuts gains one row "In the assistant's chats" and the shortcuts file one entry; C derives the counts in a scripted run (expected 443 rows, 412 without the assistant, 425 entries). |
| 12 | Insert, Replace, Copy | Take the answer without its drawing blocks; disabled under an answer that is nothing but a drawing. |
| 12a | An answer drawn before | Recorded in the stored conversation (`Message::drawn_at`). Its button reads "Draw Again" ("New Whiteboard Again"), with a tooltip that says what a second time does; the key and the command say so in a toast at the first press and draw at a second press of the same answer within five seconds. |
| 13 | Safety | A drawing changes only the open whiteboard (or makes one new whiteboard), only on the writer's press, shows what it does in a summary the app writes from the block (not the assistant's prose), keeps a version first, is one undo step, brings no picture, reads no path, fetches nothing, links only `http`, `https`, `mailto`, and stays within the agents' limits. |
| 14 | Packages | A: `innerword-board` + `innerword-assistant` (pure; and one move out of `innerword-agent`). B: `innerword-assistant-ui`. Both start at once; B builds against A's first milestone (A1: the types, the block reader, the line, `ChatSession`'s target and `Transcript`'s record of drawn answers), committed first. C: `innerword-app` and CLAUDE.md, after A and B. |

## 1. What the writer sees

**On a whiteboard.** The writer has "Plan" open, the side chat beside it (`cmd-3`), and
asks "Add the database the API reads from, and make the API blue." The answer arrives:

> I put the database to the right of the API and joined them with an arrow labelled
> "reads"; the API is filled in blue.
>
> *Drawing: adds an ellipse and an arrow; changes the rectangle “API”.*
>
> **Draw** · Insert · Replace · Copy

(Insert and Replace are dimmed on a whiteboard, as today.) While the answer is being
written the italic line says *Drawing…* and there are no buttons. The writer presses
**Draw**, `cmd-shift-d`, the item "Draw on Whiteboard" of the answer's menu, or "Draw
answer on whiteboard" in the palette. The ellipse and the arrow appear, the API turns
blue; as the canvas has taken the drawing in, the keyboard moves to the canvas and a
toast says "Drew on “Plan”: 2 elements added, 1 changed. The whiteboard as it was is kept
as a version." with **Undo**. `cmd-z` then takes the whole drawing out; so does Undo; the
version history has a row "Before the assistant's drawing". The button now reads **Draw
Again**.

**On a document.** The writer has "Chapter 2" open and asks "Draw who serves whom in this
chapter." The answer has a drawing that only adds; its line reads *Drawing: adds 3
ellipses and 2 arrows (new whiteboard “Who serves whom”).*, and its first button is **New
Whiteboard**. Pressing it makes the whiteboard "Who serves whom" in the folder of
"Chapter 2", opens it with the keyboard on the canvas, fitted to its drawing, and says
"Made the whiteboard “Who serves whom” in Part one from the answer." with the button
**Back to “Chapter 2”**. The side chat now shows the whiteboard's own (empty)
conversation, whose empty state says where the drawing came from ("Drawn from an answer
about “Chapter 2”."), where the writer can go on ("make the arrows dashed"); the
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
  "image"` (a drawing from an answer brings no picture: section 8); an `add` entry whose
  `type` is missing or is not one the converter makes (`rectangle`, `ellipse`, `diamond`,
  `text`, `arrow`, `line`, `freedraw`, `frame`); **an id that is not the converter's
  kind of id** (1 to `limits::MAX_ID_CHARS`, 64, of `[A-Za-z0-9_-]`, the rule of
  `skeleton::fields::is_id`, made `pub` as `innerword_board::is_id`), wherever the
  drawing names one: an `add` entry's `id`, the `id` of `start` and `end`, an entry of
  `children`, the `id` of an `update` entry and its `start`, `end` and `children`, an entry
  of `remove`; more than `innerword_board::limits::MAX_REQUEST_ELEMENTS` (2,000) entries
  in one of the lists, after the blocks of an answer are joined. So every id that reaches
  the line of 2.3 is plain letters, digits, `-` and `_`, and can carry no formatting.
  Everything else (a field, a value, an id the whiteboard does not have, the whiteboard's
  limits) is judged when the drawing is applied, by `innerword_board::draw`, against the
  scene as it is then (section 5).
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
| closed, read, usable | `*Drawing: adds 2 ellipses and 3 arrows; changes the rectangle “API” and db; removes a picture.*` (over a document, with a title: `*Drawing: adds 3 ellipses and 2 arrows (new whiteboard “Who serves whom”).*`) |
| closed, cannot be used | `*Drawing that cannot be used: it is not JSON (line 3, column 14: expected \`,\` or \`}\`).*` |
| open, the answer still being written | `*Drawing…*` (everything from the fence to the end of the text) |
| open, the answer finished (stopped, failed, or the model forgot the fence) | `*Drawing that cannot be used: it was not finished.*` |

The JSON is never shown in the chat. The summary is written by the app from the block, so
it says what the drawing does whatever the prose around it says (section 8). The quick
chat does not do this: its prompt has no drawings, and its answers are shown as today
(`MessageListOptions::drawings(false)`, the default).

**While the answer is written**, a last line that has not ended yet and that could become
an opening fence (at most three spaces, then a backtick or a tilde and nothing but more of
the same and info-string text: "\`", "\`\`\`whi") is left out of the shown text until it
ends, so a drawing's fence never shows as a code block for a moment; a line that ends and
is no `whiteboard` fence comes back as it was.

**The summary's words** (`innerword_assistant::drawing_sentence(drawing, target, names)`):

- adds: counts by type in a fixed order (rectangle, ellipse, diamond, text, arrow, line,
  drawn line for `freedraw`, frame; no other type can be read, 2.1), "a"/"an" for one ("a
  frame", "an ellipse"), joined with commas and "and": "adds 4 rectangles, 3 arrows and a
  frame";
- changes and removes: up to three elements, then "and N more": "changes api, db and
  cache", "changes 7 elements (api, db, cache and 4 more)", "removes db". **An element is
  named as the whiteboard has it** when the host gives the panel the names of the open
  whiteboard's elements (`ElementNames`, 4.2): "the rectangle “API”" (its type and its
  text or label, cut at 40 characters), "a picture" (every picture, so a removal of a
  picture is always said as one), "the frame “Backend”", "an arrow" without a label; the
  bare id only where no name is known (over a document, in tests without names);
- links: every distinct address that `add` or `update` sets as a `link`, by its host
  (`mailto:` by its address), up to three and then "and N more": "links to example.com";
  `"link": null` in a change: "removes the link of the rectangle “API”";
- the parts joined with "; ", ended with a period; **the counts first**, then, only for
  `DrawingTarget::NewWhiteboard` and a `title`, " (new whiteboard “…”)" before the
  period. Over a whiteboard the title is not shown (it is ignored there).
- **Escaped**: every piece of the model's own text in the line (the title, an element's
  text, a host, the clause of a problem, serde_json's message, which has backticks) goes
  through `innerword_assistant::escape_markdown` (a backslash before every one of
  ``\ ` * _ { } [ ] ( ) # + - . ! | < > ~ "`` and line ends made spaces), so nothing of
  it becomes formatting or a link in the importer, and the line says only what the app
  wrote.

### 2.4 What a block that cannot be used says

`innerword_board::DrawingError` (its `Display`, a clause without a capital or a period)
and `innerword_assistant::DrawingProblem::reason` give the clause; the chat puts it
(escaped) after "Drawing that cannot be used: ", the disabled button's tooltip after the
same words, and the toast of the key after "Nothing was drawn: ". The answer's menu item
shows the reason after its label cut at 60 characters with "…" (serde_json's messages are
long); the tooltip and the toast have it whole.

| Problem | Clause |
|---|---|
| not JSON | `it is not JSON (line 3, column 14: expected `,` or `}`)` |
| not a drawing | `it is not a drawing: “elements” is not one of its parts (add, update, remove and title are)`; `… add must be a list of elements`; `… remove must be a list of ids` |
| empty | `it adds, changes and removes nothing` |
| a picture | `the element at index 2 of add is a picture, and a drawing from an answer cannot bring one` |
| a type it cannot make | `the element at index 2 of add has no type it can draw (rectangle, ellipse, diamond, text, arrow, line, freedraw and frame are)` |
| an id of another kind | `it names an element “all elements”, and an id is 1 to 64 letters, digits, - and _` (the id escaped and cut at 40 characters) |
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

`ChatSession` holds the target (`with_drawing_target`, `set_drawing_target`), builds the
system prompt from it when it starts a conversation (`prompts::side_system_prompt(tools,
target)`), and a change of target retires the running conversation, so the next message
starts one with the new prompt and the conversation so far (as a change of tools does).
**The target is part of the item, not set after it**: the host's `DocumentRef` carries it
(`DocumentRef::with_drawing`, 4.3), and `AssistantPanel::set_document` makes every chat
(new or parked) with the target of its own item, so no process is started under another
item's prompt and restarted. `AssistantPanel::set_drawing_target` remains for a change
while the same item stays open (a page engine that turns up or goes; the whiteboard that
becomes drawable or not), and the restart in `ensure_running` is only the guard behind
both. The default target is `None`, so a session made without one starts with today's
`SIDE_SYSTEM_PROMPT` exactly (the test of `chat.rs` that compares the started prompt with
it keeps holding). The quick chat's prompt does not change.

**Accepted cost of the document section.** The section goes into every document
conversation whenever a page engine exists (about 850 tokens). The system prompt is part
of every request of a conversation; the engine caches it, so after the first message it
costs little, but it is there. Adding it only once the writer mentions drawing would
change the system prompt in the middle of a conversation and restart the process (about a
second, and the cache lost), and would miss "map this out" and its kin. So it is always
there, and it says to draw **only when asked** ("When the writer asks you to draw…"), so
it does not invite drawings into a writing chat. If real use shows unasked drawings, the
fallback is to drop the document section and keep the whiteboard one.

The base side prompt changes in two clauses: the whiteboard paragraph's last sentence
becomes "Answer about it, sum it up and suggest changes, and do not offer text to put into
it." (was "… suggest changes in words …"), and "No code blocks around prose." gains "(a
whiteboard block, below, is the one exception)" when a drawing section follows (the
builder writes the base prompt with that clause only for the targets that have one, so
`side_system_prompt(t, DrawingTarget::None)` is today's text). The sentence the existing
test holds ("It cannot be changed by you: only its owner draws on it, in the app.")
stays: it is still true, the owner applies.

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

Earlier drawings of this conversation may appear above as a note in square brackets, "[Inner Word replaced a whiteboard block here: …]". That note is the app's, not a way to answer: always write a new drawing as a whiteboard block.

For example, with this listing:
{EXAMPLE_LISTING}
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

Earlier drawings of this conversation may appear above as a note in square brackets, "[Inner Word replaced a whiteboard block here: …]". That note is the app's, not a way to answer: always write a new drawing as a whiteboard block. To change a drawing you wrote before, write it again whole.

For example, asked who serves whom in the chapter:

```whiteboard
{"title": "Who serves whom", "add": [
  {"type": "ellipse", "id": "keeper", "x": 0, "y": 0, "width": 180, "height": 80, "label": {"text": "The keeper"}},
  {"type": "ellipse", "id": "ada", "x": 320, "y": 0, "width": 180, "height": 80, "label": {"text": "Ada"}},
  {"type": "arrow", "id": "serves", "x": 180, "y": 40, "start": {"id": "keeper"}, "end": {"id": "ada"}, "label": {"text": "serves"}}]}
```
````

### 3.5 Budget, and the history

- **Budget**: the whiteboard section (with the shared part) at most 4,600 characters, the
  document section at most 3,600 (about 1,150 and 900 tokens at four characters a token;
  each grew by the paragraph about earlier drawings), held by a test. The base side
  prompt is about 5,300 characters, so a whiteboard's conversation has about 2,500 tokens
  of system prompt. **The system prompt is part of every request of the conversation**
  (cached by the engine after the first), not sent once. The builder may shorten the
  wording to fit; the facts of 3.2 to 3.4 stay.
- **The example listing is the real format**: `{EXAMPLE_LISTING}` is not written by hand.
  `drawing_prompt(OpenWhiteboard)` makes it each time it builds the section (once per
  process start): the example scene (`Scene::empty()` with the rectangle skeleton `{"type":
  "rectangle", "id": "api", "x": 100, "y": 80, "width": 200, "height": 90, "label":
  {"text": "API"}}` added by `innerword_board::add` with a fixed `HashedMaking`) listed by
  `Scene::listing` with the header `board:12 “Plan”`. A change of the listing's format
  changes the example with it.
- **The examples are tested**: a test of `innerword-assistant` takes the ```` ```whiteboard
  ```` block out of each section with `answer_drawing`, and draws it with
  `innerword_board::draw` (the whiteboard example on the example scene above, the document
  example on an empty scene) without an error and with the counts the example's prose
  claims. A prompt that teaches a field the converter refuses fails.
- **The history carried into a new process** (`Transcript::history`): an answer's drawing
  blocks are replaced by a note that reads as the app's, not as a way to answer:
  "[Inner Word replaced a whiteboard block here: it added an ellipse and an arrow and
  changed api.]" (`drawing_sentence` without names, in the past tense:
  `drawing_history_note`), and both prompt sections say what the note is (above). **The
  newest answer with a usable drawing keeps its block as written** when its blocks
  together are at most `HISTORY_DRAWING_CHARS` (8,000) characters, so "make the boxes
  blue" after a relaunch or a change of model, in a document's chat where no listing is
  sent, has the JSON to rework; older drawings and a larger one are the note. The JSON of
  every drawing would crowd the 60,000 characters out, and on a whiteboard the listing sent
  with the next message says what is there now. The stored transcript keeps every answer
  as written.
- **The stored conversation** keeps an answer whole, its JSON included, so large drawings
  use up the 120,000 characters a conversation keeps (CLAUDE.md, "Conversations") sooner:
  a drawing of a few hundred elements is tens of thousands of characters and pushes the
  earlier messages out (an answer is at most 200,000 characters; how the store's cut
  treats one longer than 120,000 is today's rule and does not change). Accepted: such
  drawings are rare, and
  what is drawn is on the whiteboard and in its versions.

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
    /// skeletons; or an `add` entry without a type the converter makes: what is wrong, in
    /// words.
    NotADrawing { message: String },
    /// An id, wherever the drawing names one (2.1), that is not 1 to `MAX_ID_CHARS` of
    /// `[A-Za-z0-9_-]`: the id as written.
    BadId { id: String },
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
    /// counted apart. Only the eight types of 2.1 can occur (others are refused by
    /// `from_json`).
    pub fn added_types(&self) -> Vec<(&'static str, usize)>;
    /// The ids its changes name, each once, in order.
    pub fn changed_ids(&self) -> Vec<String>;
    /// The ids its removals name, each once, in order.
    pub fn removed_ids(&self) -> Vec<String>;
    /// The addresses its adds and changes set as `link`, each once, in order; and the
    /// ids whose link a change removes (`"link": null`).
    pub fn links(&self) -> (Vec<String>, Vec<String>);
    /// The drawing as compact JSON (the seed of its `HashedMaking`, and tests).
    pub fn to_json(&self) -> String;
}

/// Whether `text` is an id the converter takes: 1 to `limits::MAX_ID_CHARS` of
/// `[A-Za-z0-9_-]` (was `skeleton::fields::is_id`, `pub(crate)`).
pub fn is_id(text: &str) -> bool;

/// Which changes of an update change something, each judged against the scene as it
/// stood just before it (moved here from `innerword-agent/src/boards.rs`, with
/// `left_as_it_was`, `same_element`, `same_value` and `STAMPS`; the agents call it from
/// here, unchanged in behaviour). `apply` runs some of the changes on a scene.
pub fn changes_that_change<E>(
    changes: &[ElementChange],
    before: &Scene,
    after: &Scene,
    apply: impl FnMut(&mut Scene, &[ElementChange]) -> Result<(), E>,
) -> Result<Vec<bool>, E>;

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
    /// Every element its changes changed, followers included (`Updated::changed`), after
    /// the changes that set what an element already had were left out.
    pub changed: Vec<String>,
    /// Changes left out because they set what their element already had (by index in
    /// `update`).
    pub unchanged: Vec<usize>,
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

impl Drawn {
    /// Whether the scene is as it was: nothing added, every change left out (it set what
    /// its element had) and every removal gone already. Then `draw` left `scene`
    /// untouched (byte for byte its `to_stored`), and the caller keeps no version, saves
    /// nothing and adopts nothing.
    pub fn is_nothing(&self) -> bool;
}

/// Why nothing was drawn.
#[derive(Clone, Debug, PartialEq)]
pub enum DrawError {
    /// It names elements the scene does not have: `changed` by the ids of its changes,
    /// `named` by references (an arrow's `start` or `end`, a frame's `children`, in `add`
    /// or in `update`) that are neither in the scene nor among its own adds.
    Gone { changed: Vec<String>, named: Vec<String> },
    /// The converter's refusal of one entry: "add" or "update", its index there.
    Entry { list: &'static str, index: usize, field: Option<String>, message: String },
    /// The converter's refusal of the whole: the whiteboard's limits for agents.
    Board(BoardError),
}

/// The open whiteboard's words: "the answer changes api and db, which the whiteboard does
/// not have" / "the answer joins arrows or frames to cache, which the whiteboard does not
/// have"; "the element at index 3 of add (fillStyle): …" / "the change at index 0 of
/// update (label): …"; `BoardError`'s words. (A new whiteboard's words are the app's, 6.)
impl std::fmt::Display for DrawError { /* … */ }

/// Applies `drawing` to `scene`, whole or not at all (on an error the scene is as it was):
/// 1. removals of ids the scene does not have are left out (`already_gone`);
/// 2. a change of an id the scene does not have, or a reference (`start`, `end`,
///    `children`, in `add` and in `update`) to an id that is neither in the scene nor
///    among the drawing's own adds, is `DrawError::Gone` (every such id, each once);
/// 3. an id of `add` that the scene has is renamed (below), with every reference to it
///    inside `add` (`start`, `end`, `children`); `update` and `remove` always name what was
///    on the whiteboard: their `id`, **and their `start`, `end` and `children`**, mean the
///    whiteboard's element when the whiteboard has that id, also when `add` gave a new
///    element the same id (that one was renamed); an id only `add` has means the new one;
/// 4. [`add`] with no pictures, then, for a drawing that stands on its own, [`place`] and
///    the added elements moved by its offset (one rigid move: they name nothing else);
/// 5. [`update`] with only the changes that change something
///    ([`changes_that_change`], judged as the agents judge them, `Drawn::unchanged` the
///    others), then [`remove`];
/// 6. the warnings of frames computed again after the move.
///
/// **Renaming** (step 3): the new name is `<stem>-<n>` for the smallest `n` from 2 that
/// is free against every id of the scene, every id written in `add` (renamed or not) and
/// every name given by an earlier renaming of the same drawing; `<stem>` is the id cut
/// (at a character boundary; ids are ASCII) so that `<stem>-<n>` is at most
/// `MAX_ID_CHARS` (64). The converter never sees a name it refuses.
///
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

`lib.rs`: `mod drawing; mod unchanged; pub use drawing::{DrawError, Drawing, DrawingError,
Drawn, Placement, PLACE_GAP, PLACE_MARGIN, draw, place}; pub use unchanged::changes_that_change;
pub use skeleton::fields::is_id;` and the crate docs gain a line: "What the assistant
proposes: a drawing ([`Drawing`]) applied whole or not at all ([`draw`]), a drawing that
stands on its own put where it is seen ([`place`])."

**The move out of `innerword-agent`** (A's, the one change outside A's two crates;
nobody else works in `innerword-agent` meanwhile): `changes_that_change`,
`left_as_it_was`, `same_element`, `same_value` and `STAMPS` go to
`innerword-board/src/unchanged.rs` as they are (only the error type made generic);
`innerword-agent/src/boards.rs` calls `innerword_board::changes_that_change` and keeps
whatever else of them it still uses through that crate. Its tests do not change and must
pass as they are (`CARGO_TARGET_DIR=target-pure cargo test -p innerword-agent`); its suite
through the app as well.

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

/// How an element of the open whiteboard is named in the line (2.3).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ElementName {
    /// Its type as the listing says it ("rectangle", "arrow", "image", …).
    pub kind: String,
    /// Its text, or its label's, when it has one.
    pub text: Option<String>,
}

/// The names of the open whiteboard's elements, given by the host (the app answers from
/// the scene it holds). `None` for an id it does not have.
pub trait ElementNames {
    fn name(&self, id: &str) -> Option<ElementName>;
}

/// The answer as the side chat shows it (2.3): each block replaced by one line in
/// italics, the unfinished last line that could be a fence left out while `streaming`.
/// Borrowed when there is no block and nothing is left out.
pub fn shown_markdown<'a>(
    markdown: &'a str,
    streaming: bool,
    target: DrawingTarget,
    names: Option<&dyn ElementNames>,
) -> std::borrow::Cow<'a, str>;

/// The summary of a drawing (2.3), already escaped Markdown: "Drawing: adds an ellipse
/// and an arrow; changes the rectangle “API”." The title only for `NewWhiteboard`.
pub fn drawing_sentence(
    drawing: &Drawing,
    target: DrawingTarget,
    names: Option<&dyn ElementNames>,
) -> String;

/// The note an answer's drawing becomes in the history (3.5): "[Inner Word replaced a
/// whiteboard block here: it added an ellipse and an arrow and changed api.]"
pub fn drawing_history_note(drawing: &Drawing) -> String;

/// Markdown made plain text: a backslash before every character the importer reads as
/// formatting, line ends made spaces (2.3).
pub fn escape_markdown(text: &str) -> String;
```

`answer_drawing` keeps a small cache (a `Mutex<HashMap>` of at most 32 entries, keyed by
the hash of a closed block's body) of what `Drawing::from_json` made of it, so the side
chat's views, which ask again after every delta of a streamed answer, read each closed
block once; an open block is never read (it is "Drawing…").

**`src/prompts.rs`**:

```rust
/// What both drawing sections say about elements (3.2).
pub const DRAWING_ELEMENTS: &str = "...";

/// The section of the side prompt for `target` (3.3, 3.4), `None` for `DrawingTarget::None`.
/// (A1: `None` for every target; A2 fills it.)
pub fn drawing_prompt(target: DrawingTarget) -> Option<String>;

/// The system prompt of the side chat, with or without the tools of the library, and with
/// the drawing section of `target`. (Was `side_system_prompt(tools)`.) For
/// `DrawingTarget::None` exactly today's text. (A1: the base prompt for every target.)
pub fn side_system_prompt(tools: bool, target: DrawingTarget) -> String;
```

The sections are built by `format!` from `DRAWING_ELEMENTS` and the example listing
(`concat!` takes literals only). The order of the side prompt: the base prompt, the
drawing section after a blank line, the note without tools last.

**`src/session.rs`** (`ChatSession`; **in A1**, so B can finish on A1 alone):

```rust
/// The same chat offering the drawings of `target` (its system prompt follows).
pub fn with_drawing_target(self, target: DrawingTarget) -> Self;
/// Offers the drawings of `target` from the next message on: a running conversation
/// started for another target is retired, and the next message starts one with the
/// conversation so far, as a change of tools does.
pub fn set_drawing_target(&mut self, target: DrawingTarget);
/// What the chat offers (`DrawingTarget::None` unless told).
pub fn drawing_target(&self) -> DrawingTarget;
/// Records that answer `id` was drawn at `at` (ms since 1970): `Message::drawn_at`.
/// Returns whether there was such an answer.
pub fn mark_drawn(&mut self, id: MessageId, at: i64) -> bool;
```

`Running` records its target; `ensure_running` (`session.rs:342`) restarts when it
differs (as for the selection), building the prompt with `side_system_prompt(tools,
target)`; a quick chat ignores the target. A system prompt given with
`with_system_prompt` wins as today.

**`src/transcript.rs`**:

- `Message::drawn_at: Option<i64>` (`#[serde(default, skip_serializing_if =
  "Option::is_none")]`; in A1): when the answer's drawing was last applied, kept in the
  stored conversation, so "Draw Again" survives a relaunch. `Transcript::mark_drawn(id,
  at) -> bool`.
- `Transcript::history` (A2) replaces each drawing block of an answer by its
  `drawing_history_note`, except the blocks of the newest answer with a usable drawing
  when they are at most `HISTORY_DRAWING_CHARS` (8,000) together (3.5);
  `pub const HISTORY_DRAWING_CHARS: usize = 8_000`.

**`src/lib.rs`**: `pub mod drawing;` and `pub use drawing::{DRAWING_INFO, Drawing, DrawingBlock,
DrawingError, DrawingProblem, DrawingTarget, ElementName, ElementNames, answer_drawing,
drawing_blocks, drawing_fits, drawing_history_note, drawing_sentence, escape_markdown,
shown_markdown, without_drawings};`, `pub use prompts::{DRAWING_ELEMENTS, drawing_prompt,
side_system_prompt, ...}`, `pub use transcript::HISTORY_DRAWING_CHARS`. The crate docs
gain "Drawings" in the list.

### 4.3 `innerword-assistant-ui`

**`src/actions.rs`**:

```rust
actions!(assistant, [ /* the existing ones */
    /// Draw the answer's drawing: on the open whiteboard, or as a new whiteboard beside
    /// the open document.
    DrawAnswer,
]);
// key_bindings(): in the panel's context only. InsertAtCaret is bound a second time in
// `Editor && assistant_side` because the editor binds `cmd-enter` itself; nothing binds
// `cmd-shift-d` in an editor, so the panel's binding reaches the composer and the answers.
KeyBinding::new("cmd-shift-d", DrawAnswer, panel),
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

`AnswerView` caches the drawing and `has_text` with its fingerprint (the parse of each
closed block is cached by its body in `answer_drawing`, 4.2, so a streamed answer's
deltas do not read its closed blocks again). A disabled action's button carries its reason
as the tooltip, its menu item is dimmed with the reason after its label (cut at 60
characters, 2.4); `invoke_action` acts only on `Enabled`. The keys of the side chat (an
action's `key_of`) are unchanged in kind.

`AnswerAction` gains `.menu_last()`: in the answer's menu the action comes after the
others (the button row keeps the order of `sync_actions`). The drawing action is first in
the row and last in the menu, so `enter enter` on a marked answer is Insert at Caret as
before.

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

pub struct DocumentRef {
    pub id: Option<i64>,
    pub title: String,
    /// What drawings the item offers (`None` unless told).
    pub drawing_target: DrawingTarget,
    /// With `DrawingTarget::None`: what the key and the command say (else "Open a
    /// whiteboard or a document to draw the answer."), e.g. the whiteboard's
    /// `board_not_ready_sentence` while it cannot be drawn on.
    pub drawing_unavailable: Option<SharedString>,
    /// The empty state's line about where the item came from ("Drawn from an answer about
    /// “Chapter 2”."), for a whiteboard made from an answer, until its first message.
    pub origin_note: Option<SharedString>,
}

impl DocumentRef {
    /// The item offers the drawings of `target` (`why_not` with `None`).
    pub fn with_drawing(self, target: DrawingTarget, why_not: Option<SharedString>) -> Self;
    /// The empty state names where the item came from.
    pub fn with_origin_note(self, note: impl Into<SharedString>) -> Self;
}

impl AssistantPanel {
    // `set_document(document, stored, window, cx)` keeps its signature: every chat it makes
    // or brings back from the parked ones is given `document.drawing_target`
    // (`ChatSession::set_drawing_target` before anything is prepared), so each item's
    // conversation starts with its own prompt.

    /// A change of what the open item offers while the same item stays open (a page engine
    /// found or gone, the whiteboard drawable or not). Sets the shown chat's session, the
    /// drawing action's label, tooltip and state, and the empty state.
    pub fn set_drawing_target(
        &mut self,
        target: DrawingTarget,
        why_not: Option<SharedString>,
        cx: &mut Context<Self>,
    );
    /// What the host said.
    pub fn drawing_target(&self) -> DrawingTarget;
    /// The names of the open whiteboard's elements for the line of 2.3 (`None` over a
    /// document, or when the host has none). Lines already shown are made again when it
    /// is set; the host sets it when the open whiteboard's scene it holds changed.
    pub fn set_element_names(&mut self, names: Option<Arc<dyn ElementNames + Send + Sync>>, cx: &mut Context<Self>);
    /// Tells the host to apply answer `message`'s drawing ([`AssistantPanelEvent::Draw`]).
    /// Nothing happens for an answer that is being written or has no usable drawing for
    /// the target; then a toast says why, when `say` (the key, the command). An answer
    /// drawn before (`drawn_at`) is drawn by the button at once; by the key or the command
    /// (`say`) only at a second press of the same answer within `DRAW_AGAIN_WITHIN` (5 s),
    /// the first press saying so. Returns whether the host was told.
    pub fn draw(&mut self, message: MessageId, say: bool, cx: &mut Context<Self>) -> bool;
    /// What `cmd-shift-d` and the command do: [`AssistantPanel::draw`] with the marked
    /// answer while the keyboard is in the message list, else with the latest answer.
    pub fn draw_target_answer(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool;
    /// The answer `cmd-shift-d` would draw now (the marked one, else the latest), if any.
    pub fn draw_target(&self, window: &Window, cx: &App) -> Option<MessageId>;
    /// Marks answer `message` in the message list (as `up`/`down` would) and scrolls it
    /// into view, without taking the keyboard: the palette's first press (section 7).
    pub fn show_answer(&mut self, message: MessageId, cx: &mut Context<Self>);
    /// Records that answer `message`'s drawing was applied (`ChatSession::mark_drawn`),
    /// relabels its action ("Draw Again"), and emits `TranscriptChanged`, so it is stored.
    pub fn mark_drawn(&mut self, message: MessageId, cx: &mut Context<Self>);
    /// Puts `text` into the composer when it is empty (the keyboard stays where it is).
    /// Returns whether it did. The host offers the writer a message to send.
    pub fn offer_message(&mut self, text: &str, cx: &mut Context<Self>) -> bool;
}

/// How soon a second press of the key or the command draws an answer drawn before.
pub const DRAW_AGAIN_WITHIN: Duration = Duration::from_secs(5);
```

- `sync_actions` puts the drawing action first in the row and last in the menu
  (`menu_last`): `AnswerAction::new(ACTION_DRAW, "Draw").tooltip("Draw on Whiteboard")`
  for `OpenWhiteboard`, `("New Whiteboard", "New Whiteboard from Answer")` for
  `NewWhiteboard`, `.key_of(DrawAnswer).state_for(..)`: `Hidden` without a drawing or for
  `DrawingTarget::None`; `Disabled(reason)` for a problem or a drawing that does not fit
  the target (`drawing_fits`); `Enabled` otherwise. **Under an answer drawn before**
  (`Message::drawn_at`) the label is "Draw Again" with the tooltip "Already drawn once:
  drawing it again adds its elements again and moves again what it moves", or "New
  Whiteboard Again" with "Already made into a whiteboard once: this makes another one";
  the menu item likewise. Insert and Replace get `state_for`: `Disabled("The answer has
  nothing but a drawing")` when `!has_text`, else as today. The panel's list is made with
  `.drawings(true)`, and shows lines with the target and the names it was given.
- `apply` (Insert, Replace, Copy) sends `without_drawings(text)` as `markdown`.
- **Toasts of the key and the command** (`draw(.., say: true)`): no answer, or the target
  answer has no drawing: "The answer has no drawing."; a problem: "Nothing was drawn:
  {reason}."; `DrawingTarget::None`: the host's `drawing_unavailable`, else "Open a
  whiteboard or a document to draw the answer."; an answer drawn before, at the first
  press: "This answer was drawn already. Press again to draw it once more." (over a
  document: "This answer was made into a whiteboard already. Press again to make another
  one.").
- **The empty state** follows the target. `OpenWhiteboard`: title "Ask about this
  whiteboard", text "The assistant reads the whiteboard's elements as they are when you
  send a message. It can propose a drawing; you decide whether it goes on the
  whiteboard.", examples "Group these into frames by topic.", "Add a legend that explains
  the colours.", "What is missing from this diagram?"; the composer's placeholder "Ask
  about this whiteboard"; the `origin_note` as a line above the examples when given.
  Documents keep theirs.
- `render.rs`: the root's `on_action(DrawAnswer)` and the global listener
  (`global!(DrawAnswer => …draw_target_answer…)`), as InsertAtCaret has.
- `debug_state` gains `draw=whiteboard|new|none` and, for the latest answer, `drawn=yes|no`.

### 4.4 `innerword-app`

- **`workspace/assistant.rs`**: `fn drawing_target_now(&self, cx: &App) ->
  (DrawingTarget, Option<SharedString>)`: `DrawingTarget::of(kind of the open item,
  pages::engine(cx).is_some())`, except that an open whiteboard that cannot be drawn on
  now (`!board_drawable()`: no engine, the page failed or stopped) is `None` with
  `board_not_ready_sentence` as the reason. It goes **on the `DocumentRef`** that
  `assistant_document_changed` hands to `set_document` (`DocumentRef::new(..)
  .with_drawing(target, why_not)`), and `follow_the_text` calls `set_drawing_target` when
  it changes while the item stays open. A whiteboard made from an answer is opened with
  `.with_origin_note(..)` on its `DocumentRef` until its conversation has a message.
  `on_side_chat_event` handles `AssistantPanelEvent::Draw` by target. **The names**: an
  `ElementNames` over the open whiteboard's scene (`BoardNames`, parsed from the shared
  copy `feed_the_side_chat` keeps, once per new pointer, only when the panel asks a name:
  most answers have no `update` or `remove`), set with `set_element_names` when the open
  whiteboard or its held scene changes, `None` over anything else.
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
- **`workspace/board.rs`**:
  - `BoardState::drawing_pending: bool`: set at the press (5.1 step 1), cleared on every
    way out of `draw_answer_on_board` (each early return, the task dropped, the
    whiteboard closed: `discard_page` clears it with the rest of the state);
  - `BoardState::assistant_drawing: Option<PendingDrawing>` (`{ seq: u64, sentence:
    String, keyboard_was_in_chat: bool, message: MessageId }`), set only when an adopt was
    really sent, read in `board_adopted` and `board_adopt_failed` (5.4);
  - `adopt_stored_scene` returns `Option<u64>`: the seq of the adopt it sent, `None`
    when it sent none (an empty delta, no board, a scene it cannot read);
    `adopt_in_flight()` (exists, `pub(crate)`) is what the checks of 5.1 read;
  - `pub(crate) fn board_view_bounds(&self) -> Option<Bounds>` (5.3, from the status and
    the page element's bounds).
- **`workspace/live.rs`**: `pub(crate) fn writer_changed_the_board(&mut self, board:
  DocumentId, cx)`: the last agents' run on that whiteboard offers no Undo any more and
  cannot get one back (its `undo` is `None` and the run is marked as drawn into, as a
  gesture of the writer between two of its changes marks it), and the note, when up, is
  shown again. Called where the drawing's adopt is sent (5.1), so it does not rest on
  `board_moved_on` seeing a rev change.
- **`workspace/render.rs`**: `capture_action(DrawAnswer)`: when the side chat **is
  shown** (in the right slot and in front), the action goes on to the panel as today.
  When it is not (closed, or behind the comments), it brings the side chat
  (`bring_the_side_chat_for_its_command`), marks the answer the key would draw
  (`draw_target` then `show_answer`), stops the action, and says "The drawing is shown in
  the side chat. Run the command again to draw it." (a toast). Nothing is drawn the
  writer has not seen; the second run, with the chat shown, draws. Without an answer
  with a drawing it only brings the chat, as `NewChat` does.
- **`workspace/mod.rs`** (the palette, `toggle_command_palette`): after the existing
  arms, one more: `if self.drawing_target_now(cx).0 == DrawingTarget::None {
  left_out.push(gpui::Action::name(&DrawAnswer)) }`. That covers a page, nothing open
  (the existing code has no arm of its own for nothing open: with no item
  `commands_not_for_the_open_page` leaves nothing out, and this line does), no page
  engine, and a whiteboard that cannot be drawn on. `board_title_of` names it "Draw answer
  on whiteboard".
- **`boards.rs`** (the fake engine): levers `FakePages::hold_adopts()` /
  `release_adopts()` (an adopt is answered only at the release: 5.4's test), and `zoom`,
  `scroll_x`, `scroll_y` in the fake status as tests set them (`set_status`); the page
  element's bounds are the test window's layout. (`center` is not used: 5.3.)
- **Scripted runs**: in a run with `INNER_WORD_AUTOMATION` the workspace prints one line per
  drawing, `[automation] draw: 2 added, 1 changed, 0 removed, version kept, placed as
  written, change 3.1 ms, adopt answered in 40 ms, keyboard to the canvas after the adopt
  answered` (or `draw: nothing: <reason>`), and
  `draw: new board:14 “Who serves whom” in Part one, 5 added, change 2.0 ms`.

## 5. Drawing on the open whiteboard

### 5.1 The steps

`Workspace::draw_answer_on_board(drawing)`:

1. **Can it be done, and mark it under way** (in the update of the press): a whiteboard is
   open (`open_board_id`), else "Open a whiteboard to draw on it." It can be drawn on
   (`board_ready()`), else `board_not_ready_sentence` (still loading, not drawn,
   stopped). A drawing of the assistant is pending on it (`drawing_pending`, or
   `assistant_drawing` not answered yet), or an adopt is in flight
   (`adopt_in_flight().is_some()`): "The whiteboard is taking in another change. Press
   again in a moment." Otherwise **`drawing_pending = true` at once**, so a second press
   (the button and then the key, two presses of `cmd-shift-d`) within the awaits below
   gets the busy toast and draws nothing. One drawing at a time. Whether the keyboard is
   in the side chat now is noted (`keyboard_was_in_chat`, for step 4).
2. **Ask the page** (a task of the window, as `add_to_board` does): `collect` when the
   status says the page holds more (`board.status.holds_more()`), awaited at most
   `VIEW_COLLECT_PATIENCE`. Nothing else is asked: the view comes from the status (5.3).
3. **In one update** (no await from here to the adopt call, so no gesture, agent request
   or key comes between):
   - **everything is checked again**, because the await let other turns run:
     `open_board_id() == Some(id)` and it is the same session (else `drawing_pending` is
     cleared and the toast "Nothing was drawn: the whiteboard was closed before the
     drawing was made." goes up, so a press is never dropped without a word);
     `board_ready()` (else its sentence); `adopt_in_flight().is_none()` (else the busy
     toast: an agent's request carried out during the await has started its own adopt,
     and a second adopt now would bump `adopt_seq` and leave one of the two answers
     unmatched). An agent's request still **waiting** in `must_wait` is not a reason to
     refuse: when it goes on it finds this drawing's adopt in flight and waits for it,
     as it waits for any adopt (`must_wait`'s second wait), and is then merged on the
     scene with the drawing; so no new check of the channel is needed (the critic's
     suggestion to read `must_wait` here is not taken: `must_wait` is a stateful step of a
     request, not a question, and the order it gives is the right one);
   - `save_board_now(id)`: the scene the page handed over is saved (a flush; with no
     adopt in flight it is the plain path, never `flush_board_while_adopting`); when it
     cannot be: "Nothing was drawn: the whiteboard as it is now could not be saved first."
   - `S` = the stored scene (`board.base`, the flush's), `stamp` = `board.stamp`;
   - the view (5.3), `making` = `HashedMaking::new(&[stamp hex, rev, now, drawing.to_json()],
     now)` (as the Add commands seed theirs);
   - `S2 = S.clone(); innerword_board::draw(&mut S2, &drawing, view, &mut making)`; an error
     ends here with its toast (5.5): no version, no save, nothing in the view;
   - **`drawn.is_nothing()`** (every removal gone already, every change setting what its
     element had): no version, no save, no adopt, the answer is not marked drawn, and the
     toast "Nothing was drawn: everything it removes is gone already." (only removals) or
     "Nothing was drawn: it changes nothing on the whiteboard." (otherwise);
   - the version and the save, **one outcome**: `create_labeled_snapshot` commits on its
     own, so a save that fails deletes it again in the same closure:

     ```rust
     let content = prepared(&s2).content();               // saver::prepared_of, as Restore
     let (result, _) = library.change_outside(|store| {
         let snapshot = match store.create_labeled_snapshot(id, ASSISTANT_DRAWING_LABEL) {
             Ok(meta) => meta,
             Err(error) => return (Err(DrawFailure::Version(error)), Vec::new()),
         };
         match store.save_board(id, &content, Some(stamp)) {
             Ok(saved) => (
                 Ok(saved),
                 vec![OutsideChange::SnapshotsChanged(id), OutsideChange::DocumentSaved(id)],
             ),
             Err(error) => {
                 // The version would say "Before the assistant's drawing" over nothing.
                 let gone = store.delete_snapshot(snapshot.id).is_ok();
                 let changes = if gone { Vec::new() } else { vec![OutsideChange::SnapshotsChanged(id)] };
                 (Err(DrawFailure::Save(error)), changes)
             }
         }
     }, cx);
     ```

     A version that fails: "Nothing was drawn: the whiteboard as it was could not be kept
     as a version." `StoreError::Stale` (cannot happen after the flush in the same turn,
     but is said): "Nothing was drawn: the whiteboard changed while the drawing was made.
     Press again."; any other error of the save: "Nothing was drawn: it could not be
     saved (…)." `SnapshotsChanged` is reported only for a version that stays;
   - the answer is marked drawn (`AssistantPanel::mark_drawn(message)`, from here on the
     library has it, whatever the view does next);
   - `writer_changed_the_board(id)` (4.4): an agents' run's Undo on this whiteboard ends
     here, explicitly;
   - `let seq = adopt_stored_scene(id, &content, true, cx)`: the delta from `S` to `S2`,
     by the base rule, **one step of Excalidraw's undo**, the posts of the view held until
     it answers (4.6 of the whiteboard design). With `Some(seq)`:
     `board.assistant_drawing = Some(PendingDrawing { seq, sentence, keyboard_was_in_chat,
     message })`. With `None` (no delta reached the view: cannot happen when
     `is_nothing()` is false, but is handled) the toast of step 4 goes up at once, without
     Undo, and the keyboard moves as in step 4;
   - `drawing_pending = false` (the pending adopt is `assistant_drawing` from here);
   - when `Drawn::placement` says `in_view: false` with a view known:
     `zoom_board(calls::Zoom::Fit)` (the page runs its calls in order, after the adopt).
   - **The keyboard does not move here.** The glue's `adopt` awaits `addFiles` and
     `loadFontsFor` before its `updateScene`; a `cmd-z` on the canvas before that would
     take out the writer's own last gesture. Until the answer the keyboard stays where it
     was (in the side chat, where `cmd-z` is the composer's).
4. **When the view answers** (`board_adopted` with that `seq`): `assistant_drawing` is
   taken; then the keyboard goes to the canvas (`give_the_board_the_keyboard`) when it is
   still where it was at the press (in the side chat, or already on the canvas), so the
   next `cmd-z` takes the drawing out; a writer who went elsewhere meanwhile keeps the
   keyboard there. The toast (5.5) with the conflicts the view reports; `Undo` offered
   while the canvas is at the answered rev. When the adopt fails (`board_adopt_failed`
   with that seq), the existing path runs (the merge in Rust, a reload of the view), the
   keyboard moves as above, and the toast says "Drew on “Plan”. The whiteboard was loaded
   again to show it." without Undo. A view that is closed or reloaded before it answers
   (`forget_adopt`) drops `assistant_drawing` and says "Drew on “Plan”." without Undo
   when the whiteboard is still open.

This is the path of Restore (6.5) and of an agent's change (6.7) with the writer as the one
who asked: the library has the drawing the moment it is applied, a version has the
whiteboard as it was, and the view merges it by the base rule. The Add commands'
`insertElements` path is not used: it has no removals, no base rule against a gesture
made between the collect and the call, and no version.

### 5.2 Ids

- **A removal of an element that is gone** is left out: what it asked for is so. The
  toast counts it ("1 element to remove was gone already.").
- **A change of an element that is gone, or an element that names one** (an arrow's
  `start` or `end`, a frame's `children`, in a new element or in a change): nothing is
  drawn, because half a diagram (an arrow with no end) is worse than none. The toast
  names the ids (at most five, then "and N more") and says what to do: the assistant is
  sent the whiteboard as it is with the next message, so asking again works. A reference
  in a change goes through this check before the converter, so it gets these words and
  not the converter's "the whiteboard has no element x (it has …)".
- **An id of a new element that the whiteboard has** (the assistant reused "title" in a
  second drawing; the writer's own ids are Excalidraw's random ones) is renamed with its
  references inside `add`, never refused: the writer cannot fix an id. The new name is
  free against the whiteboard, the drawing's own ids and the names given before, and is
  never longer than 64 characters (4.1, `draw`, "Renaming"). In `update` and `remove` an
  id the whiteboard has always means the whiteboard's element, also in `start`, `end` and
  `children`.
- **A change that sets what its element already has** is left out, as for agents (their
  `changes_that_change`, moved into `innerword-board`), so it adds no step and no
  version bump; a drawing left with nothing draws nothing (5.1).
- **Labels**: the listing names a shape, not its label; a change of `text` on a shape
  changes its label (7.3), and removing a shape removes its label.

### 5.3 Placement

A drawing **stands on its own** when it has no `update` and no `remove` and its adds name
nothing that was on the whiteboard. Only such a drawing is ever moved, as a whole, by one
offset (a rigid move: its arrows' points are relative, its bindings and frames are among
its own elements). A drawing that touches what is there stays where the answer put it.

**The view** in scene coordinates, from **one source and the element's size**: the page's
status (`board.status`, the one the collect answered when it ran, else the last posted:
a scroll or a zoom posts one within 250 ms) has Excalidraw's `scrollX`, `scrollY` and zoom
`z`; the page element's bounds of the last frame (`PageSession::bounds`) give its size
`w x h` in CSS pixels. Excalidraw's scene point at the view's top left is `(-scrollX,
-scrollY)` (its `viewCenter` is `width / 2 / zoom - scrollX`), so `view = (-scrollX,
-scrollY, w/z, h/z)` (`board_view_bounds`); `None` when there is no status yet or the
element has no size. `center` is not asked (one await less).

**What is accepted**: the element's size is GPUI's, not Excalidraw's `appState.width` and
`height`. With Excalidraw's shape library docked, its canvas may be narrower than the
element, so the right edge of `view` can lie under the library's sidebar, and a drawing
placed there is partly hidden until the sidebar closes. A status at most 250 ms old can
miss the end of a scroll. Both only move the drawing a little from the middle of what is
seen; it still lands free. The exact way (the glue's `center` answering `width`, `height`
and `zoom`, a serde-default addition) needs a rebuild of `dist/` with npm and a fourth
crate in the plan; it is the fix if the docked sidebar proves a nuisance (12).

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
   drawn.top`); `in_view: false`, and the view is brought to it after the adopt (section
   14: scrolled, zoomed out only when it cannot be seen whole).

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
  Once the view has answered (the step is on its undo stack), the keyboard goes to the
  canvas, where `cmd-z` is Excalidraw's (a fixed binding in `BoardView`), so `cmd-z` then
  takes the whole drawing out; the page posts the result as a change of the writer's,
  which the autosave saves compare-and-swap. Before the answer the keyboard has not moved,
  so a quick `cmd-z` cannot reach the canvas and take out the writer's last gesture in
  the drawing's place.
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
| gone ids (changes) | "Nothing was drawn: the answer changes api and db, which the whiteboard does not have. Ask again: the assistant is sent the whiteboard as it is now." (an id may have been removed meanwhile or never been there) |
| gone ids (named by new elements) | "Nothing was drawn: the answer joins arrows or frames to cache, which the whiteboard does not have. Ask again: …" |
| the converter refused | "Nothing was drawn: the element at index 3 of add (fillStyle): …" or "… the change at index 0 of update (label): …" (`DrawError::Entry`, the list named; `BoardError`'s words for the whole); and the composer, when empty, is offered "Your drawing could not be used: <the same words>. Please write it again." (`offer_message`), so one `enter` asks |
| the flush failed | "Nothing was drawn: the whiteboard as it is now could not be saved first." |
| the version failed | "Nothing was drawn: the whiteboard as it was could not be kept as a version." |
| stale | "Nothing was drawn: the whiteboard changed while the drawing was made. Press again." |
| the save failed | "Nothing was drawn: it could not be saved (…)." (the version is deleted again) |
| nothing to do | "Nothing was drawn: everything it removes is gone already." / "Nothing was drawn: it changes nothing on the whiteboard." |
| closed meanwhile | "Nothing was drawn: the whiteboard was closed before the drawing was made." |
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
   "Nothing was drawn: …" (and the composer offered the message, as in 5.5). On an empty
   scene `DrawError::Gone` can only be a reference to an id the drawing does not add, so
   the app words it for this case, not with `Display`'s "which the whiteboard does not have":
   "Nothing was drawn: the answer joins arrows or frames to cache, which the drawing does
   not have. Ask again." (`NEW_BOARD_NAMES_MISSING`).
3. **Where**: the folder of the open document (`document_meta(id).folder_id`), at its end;
   **at the root when the document is in the trash, when its folder or a folder above it
   is** (a document opened from the trash must not make a whiteboard in the trash), or
   when that folder is gone (as `new_board` falls back).
4. **Title**: the drawing's `title`, else "<document title> (drawing)", else, for a
   document with no title (shown "Untitled"), "Drawing"; a chosen name (`custom_title`
   flag, as `new_board` sets it for a title given).
5. `library.change_outside(|store| store.create_board(folder, &title, &prepared(scene)))`
   (the first whiteboard of a library takes it to schema 5, as any). The answer is marked
   drawn (`AssistantPanel::mark_drawn`) right after, before step 6 switches the chat.
6. **It opens** (`open_document(id, true, ..)`): the keyboard on its canvas, fitted to its
   drawing (a whiteboard without a stored view opens fitted, `board.jsx`; checked). Its
   `DocumentRef` carries the origin note "Drawn from an answer about “Chapter 2”." for the
   empty state of its own conversation. The document's conversation was stored with the
   document (with the answer's `drawn_at`) and comes back when the document is opened.
   The writer goes on about the drawing on the whiteboard itself.
7. The toast: "Made the whiteboard “Who serves whom” in Part one from the answer." (at the
   root: "… from the answer."), with the converter's warnings after it, and the button
   **Back to “Chapter 2”**, which opens the document again, with its conversation and the
   answer in it. The keyboard's way back is Quick Open (`cmd-p`), where the document is
   the most recent item; no new command is added for it (a command would be one more row
   of the list of shortcuts for a shortcut that already exists). No Undo: the whiteboard
   is a new item, and Trash removes it.

No version is kept (there was nothing before). The document is not changed. `update` and
`remove` are never applied over a document: a drawing with them is shown, and its button
disabled with the reason (2.4).

## 7. Keyboard and mouse

| Way | Over a whiteboard | Over a document |
|---|---|---|
| The button under an answer with a drawing (first in the row; on the latest answer always, on older ones under the pointer and on the marked one) | "Draw", tooltip "Draw on Whiteboard" ("Draw Again" once drawn) | "New Whiteboard", tooltip "New Whiteboard from Answer" ("New Whiteboard Again") |
| The answer's menu (right-click on the answer; `enter`, `ctrl-enter`, `shift-f10` on the marked message) | "Draw on Whiteboard" with its key, last in the menu | "New Whiteboard from Answer" with its key, last |
| Key, in the side chat (the composer, the answers, the message list, anywhere in the panel) | `cmd-shift-d`: the marked answer, else the latest; an answer drawn before at a second press | the same |
| Palette (category "Assistant") | "Draw answer on whiteboard": with the side chat shown it draws; with the chat closed or behind the comments the first run brings it with the answer marked and says to run it again, the second draws | "New whiteboard from answer", the same |
| Menu bar | none (as Insert, Replace and Copy) | none |

- **Over a page, with nothing open, without a page engine, or on a whiteboard that cannot
  be drawn on**: no button, the palette leaves the command out, and the key says the
  host's reason (`board_not_ready_sentence` on such a whiteboard) or "Open a whiteboard
  or a document to draw the answer."
- **One action**, `assistant::DrawAnswer`: its meaning follows the open item, as "Rename
  document" is "Rename whiteboard" on a whiteboard. The list of shortcuts shows the base
  title "New whiteboard from answer" (keywords "draw", "whiteboard"), as it shows "Rename
  document".
- **`cmd-shift-d`** is free in the side chat's places: the panel (`AssistantPanel`) and its
  editors (`Editor && assistant_side`) bind nothing on it, the editor and the window
  neither; the list of shortcuts binds it to "Reset all" in `KeymapView`, another place
  (keymap::places keeps them apart). It is bound in `AssistantPanel` only (its editors
  are inside it and bind nothing on it; InsertAtCaret's second binding exists only
  because the editor binds `cmd-enter`). It is a `cmd` chord, so Vim mode lets it
  through, and the whiteboard's own keys are not in play (the keyboard is in the chat).
- **The list of shortcuts**: one row more, "New whiteboard from answer", "In the
  assistant's chats" (the existing place; no new key context, so `keymap::places` and
  `shortcuts::Place::of_predicate` do not change). Expected: 443 rows with the assistant
  shown (was 442); 412 without it (its rows are not listed; `pages.rs`'s count is
  unchanged); the shortcuts file 425 entries (one key context; was 424). **C derives the
  numbers, it does not copy these**: a scripted run (`keys:cmd-/; state`, with and
  without `INNER_WORD_ASSISTANT_FAKE=ready`, and the file of "Export…") gives them, and
  only then go they into CLAUDE.md and the tests that pin them. The checks that derive
  their numbers (`keybindings.rs`, the row-uniqueness check,
  `every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file`) must pass unchanged.
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
  claims; it names a picture it removes as "a picture" and every address it links to by
  its host; the counts come first and every word of the model's in the line is escaped
  (2.3), and every id in it was checked to be plain letters, digits, `-` and `_` when the
  block was read (2.1), so the model's text cannot change what the line says or add a
  link to it; the button's tooltip says what it applies to. The palette command never
  draws an answer whose line the writer has not had on screen (section 7).
- **Not twice by accident**: an answer drawn before says so on its button, and the key
  and the command need a second press (12a).
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
  list, an id in `remove` not a string, empty, an image, a type it cannot draw ("star", no
  type), 2,001 adds; each `Display` as 2.4
- `every_id_is_checked_when_the_block_is_read`: `"**all** [x](https://e.x)"` as an add's
  id, as `start.id`, in `children`, as an update's id and its `end`, in `remove`, and 65
  characters: `BadId`
- `blocks_join_in_order_with_the_first_title`, and over the limit after joining
- `a_drawing_adds_changes_and_removes_whole_or_not_at_all` (a bad change leaves the scene
  as it was)
- `removing_what_is_gone_is_left_out`, `changing_what_is_gone_draws_nothing`,
  `an_arrow_to_what_is_gone_draws_nothing`, `a_change_that_joins_to_what_is_gone_draws_nothing`
  (`update` with `start` naming a gone id: `Gone`, not the converter's `NotFound`)
- `an_id_the_whiteboard_has_is_renamed_with_its_references` (`add` renamed, `update` and
  `remove`, their `start`, `end` and `children` included, keep naming the whiteboard's);
  `a_renaming_is_free_against_everything` (the scene has `a-2`, the drawing adds `a` and
  `a-2`: `a-3` and `a-2-2`); `a_renaming_stays_within_64_characters` (a 64-character id
  the scene has)
- `a_change_that_sets_what_an_element_has_is_left_out` (`Drawn::unchanged`, versions not
  bumped) and `a_drawing_of_nothing_is_nothing` (only gone removals; only such changes:
  `is_nothing()`, the scene byte for byte as it was)
- the agents' own tests of unchanged changes still pass after the move (`innerword-agent`)
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
  the end; a last line "\`\`\`whi" while streaming is left out, and comes back as text when
  it ends as no fence); `drawing_sentence` (plurals, "a"/"an", three ids then "and N
  more", the counts before the title, the title only for `NewWhiteboard`, names from an
  `ElementNames` with "a picture", links by host, `"link": null`)
- `escape_markdown` puts a backslash before each of its characters and makes line ends
  spaces (the importer's side of it is B's test, 9.3)
- `answer_drawing` reads a closed block once across many calls (the cache)
- prompts: the section of each target is in `side_system_prompt` and only there (a page,
  nothing, the quick prompt: none), with and without tools; `side_system_prompt(t,
  DrawingTarget::None)` is exactly today's text; the budget (4,600 and 3,600 characters);
  **the examples in the prompts draw** and the whiteboard section contains
  `Scene::listing` of the example scene verbatim (3.5); the existing tests still hold
  (the whiteboard sentence they quote is kept), and
  `the_side_prompt_is_a_collaborator_who_reads_and_advises` runs its withheld-tools check
  over `side_system_prompt(t, target)` for all three targets
- session: `FakeBackend::last_started().system_prompt` has the section for the target; a
  change of target restarts the conversation with its history; `prepare` after
  `set_drawing_target` starts with the target's prompt; a session made without a target
  starts with `SIDE_SYSTEM_PROMPT` (`chat.rs:342` unchanged); `mark_drawn` sets
  `drawn_at`, which survives the transcript's JSON
- history: `Transcript::history` has the "[Inner Word replaced a whiteboard block here:
  …]" note for an older block, the newest drawing answer's JSON under 8,000 characters
  and the note above it, the stored transcript the blocks
- the callers of `side_system_prompt`: `session.rs`, `prompts.rs`'s unit tests and
  `tests/suite/context.rs:463-464` take the new argument

### 9.3 `innerword-assistant-ui` (`tests/suite/panel.rs`, `components.rs`, `markdown.rs`, `rebinding.rs`)

- an answer with a drawing shows its line, never its JSON; while it is written "Drawing…"
  and no actions; the quick chat's list (drawings off) shows the block as code, as today
- **the line cannot be steered** (`markdown.rs`, through `answer_document`): a title `x”:
  adds nothing. [Safe](https://e.x)` (over a document), an element name and a link host
  with `*`, `_`, `[` and backticks, and serde_json's message with backticks all come out
  as one italic paragraph of plain text with no link and no other formatting, the app's
  counts first
- the drawing button: only under answers with a drawing, first in the row, its label and
  tooltip by target, hidden for `None`, dimmed with the reason for an unusable drawing and
  for changes over a document; the answer's menu item likewise
- the button, the menu item and `cmd-shift-d` emit `Draw { message, target, drawing }` with
  the parsed drawing; the marked answer, else the latest; the global action reaches a shown
  panel from elsewhere in the window
- the key's toasts: no drawing, a problem, no target (the default sentence and the host's
  `drawing_unavailable`)
- **drawn before**: after `mark_drawn` the button reads "Draw Again" with its tooltip; the
  button draws at once; `cmd-shift-d` the first time says so and emits nothing, a second
  press within `DRAW_AGAIN_WITHIN` (the test clock) emits `Draw`, a second press after it
  says so again; a transcript stored with `drawn_at` and given to `set_document` shows
  "Draw Again"
- the target is per chat: `set_document` with two `DocumentRef`s of different targets
  starts each chat's process once, with its own prompt (`FakeBackend` counts starts); a
  parked chat keeps its own
- the drawing item is last in the answer's menu, and `enter enter` on a marked answer is
  still Insert at Caret
- Insert, Replace and Copy carry the answer without its blocks, and are dimmed under an
  answer that is only a drawing
- the empty state and the placeholder over a whiteboard, with the origin note;
  `offer_message` fills only an empty composer; `show_answer` marks without taking the
  keyboard; `debug_state` has `draw=` and `drawn=`
- rebinding: `DrawAnswer`'s key changed with `keymap::replace_key` is the key of the button's
  hint and works, the old one does nothing

### 9.4 `innerword-app` (FakeBackend and the fake page engine only)

`tests/suite/boards.rs`:

- `the_assistant_draws_on_the_open_whiteboard_in_one_step_of_its_undo`: a reply with a
  drawing over a whiteboard, the button pressed: the stored scene has the elements, a
  version labelled "Before the assistant's drawing", the fake's `adopt` was called once
  with `undoable: true`, the keyboard is on the canvas once it answered, the toast's
  words, the answer marked drawn and stored; the Edit menu's Undo then calls `undo 1`,
  and the canvas is as before
- `the_keyboard_moves_only_when_the_canvas_has_the_drawing` (`hold_adopts`: after the
  press the keyboard is still in the side chat and a `cmd-z` there undoes nothing on the
  canvas; at `release_adopts` the keyboard is on the canvas); and a writer who clicked
  into the library meanwhile keeps the keyboard there
- `a_drawing_waits_for_what_the_page_holds` (`draw_unposted`, then the press: `collect`
  first, the writer's stroke kept beside the drawing)
- `two_presses_draw_once` (`hold_collects`, the button then `cmd-shift-d`: one adopt, one
  version, the busy toast) and `an_agent_change_during_the_collect_is_waited_for`
  (`hold_collects`, an agent's `board add` carried out meanwhile starts its adopt: the
  press ends with the busy toast, nothing drawn, no version, and the agent's note keeps
  its Undo)
- `an_agent_run_loses_its_undo_to_a_drawing` (an agent's change, a drawing, another agent
  change joined to the note: the note offers no Undo)
- `a_drawing_that_changes_what_is_gone_draws_nothing` (no version, no adopt, the toast)
- `a_drawing_of_nothing_keeps_no_version` (only gone removals; only changes that set what
  is there: no version, no save, no adopt, the toast, not marked drawn)
- `a_failed_save_leaves_no_version` (`fail_board_saves`: the toast, no row "Before the
  assistant's drawing" in the version history)
- `closing_the_whiteboard_during_the_collect_says_so`
- `a_drawing_the_converter_refuses_offers_the_message` (the composer's text; a composer
  with text is left alone)
- `a_drawing_that_covers_the_drawing_lands_free_in_view` (a zoom and a scroll in the
  status, the page element's bounds) and `one_larger_than_the_view_zooms_to_fit`
- `conflicts_are_said_once_the_view_answered` (the fake's base rule with a writer's change
  between the collect and the adopt)
- `the_toast_offers_undo_while_the_canvas_has_not_moved_on`
- `a_whiteboard_that_is_not_ready_is_not_drawn_on` (the key says
  `board_not_ready_sentence`; the palette leaves the command out), `one_drawing_at_a_time`
- `an_answer_drawn_before_needs_a_second_press` (the key; the button draws at once)
- `the_palette_shows_before_it_draws` (the side chat closed, then behind the comments: the
  first run brings it with the answer marked and draws nothing; the second draws)
- `without_a_page_engine_no_drawing_is_offered` (target `None`; the system prompt without the
  section)
- `the_line_names_the_whiteboards_elements` (an answer that removes a picture and changes
  a labelled rectangle: the shown line has "a picture" and "the rectangle “API”")

`tests/suite/assistant.rs`:

- `an_answer_over_a_document_makes_a_new_whiteboard_in_its_folder` (title from the block and
  without one, "Drawing" for an untitled document, the folder, opened, the keyboard on
  the canvas, the origin note in the new chat's empty state, the toast's "Back to
  “Chapter 2”" opens it, the document's conversation stored with `drawn_at` and back when
  the document is opened again)
- `a_document_in_the_trash_makes_its_whiteboard_at_the_root` (the document trashed, and
  its folder trashed)
- `an_answer_that_names_what_it_does_not_add_says_so_over_a_document` (the new
  whiteboard's words, not "which the whiteboard does not have")
- `each_item_starts_its_chat_with_its_own_prompt` (a document, then a whiteboard, then a
  page: one process start each, none restarted)
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
kept, placed as written, …, keyboard to the canvas after the adopt answered`; the `state`
after it has `focus=board` and the side chat's `drawn=yes`; `board:scene` lists `db` and
`reads` and `api` with `fill #a5d8ff solid`; after `cmd-z` the listing is the rectangle
alone; the version history's row says "Before the assistant's drawing". A second
`keys:cmd-3; keys:cmd-shift-d` logs the "drawn already" toast and no `draw:` line (the
builder adds it to the script). Looked at: the italic line and the "Draw" button
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
- **"Changing keys"**: the counts as C's scripted run gave them (expected 443 rows, 412
  without the assistant, 425 entries) with "and the assistant's drawing, one row and one
  entry" in the account of how they grew.
- **Agents**: the workspace table's `innerword-board` row names `changes_that_change` as
  its own now (the agents' rule of a change that changes nothing, used by agents and the
  assistant's drawings).
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

- **A1, first, committed as soon as it is done** (B builds against it and needs nothing
  else): `innerword-board`'s `Drawing`, `DrawingError` (with `BadId`), `from_json` (the
  type and id checks of 2.1 included), `join`, `is_empty`, `has_changes`, `added_types`,
  `changed_ids`, `removed_ids`, `links`, `to_json`, `is_id`, with their tests.
  `innerword-assistant`'s dependency on `innerword-board` and the whole `drawing` module
  (`DRAWING_INFO`, `DrawingTarget`, `DrawingBlock`, `DrawingProblem`, `ElementName`,
  `ElementNames`, `drawing_blocks`, `answer_drawing` with its cache, `drawing_fits`,
  `without_drawings`, `shown_markdown`, `drawing_sentence`, `drawing_history_note`,
  `escape_markdown`) with its tests, and the re-exports. **`ChatSession`'s target** (the
  field, `with_drawing_target`, `set_drawing_target`, `drawing_target`, the restart in
  `ensure_running`) with `side_system_prompt(tools, target)` returning the base prompt for
  every target and `drawing_prompt` returning `None` (A2 fills them), and its callers
  (`session.rs`, `prompts.rs`'s tests, `tests/suite/context.rs:463-464`) changed;
  `Message::drawn_at`, `Transcript::mark_drawn`, `ChatSession::mark_drawn`. Everything
  working, nothing stubbed but the two prompt functions' A2 content.
- **A2**: `draw`, `Drawn::is_nothing`, `place` and the move of `changes_that_change` (with
  `innerword-agent` calling it) with their tests; the prompts (3.1 to 3.5: the clauses of
  the base prompt, `DRAWING_ELEMENTS`, the two sections with the generated example
  listing, the budget test, the examples drawn, the three-target check of the withheld
  tools); `Transcript::history`'s note and the kept newest block.
- Done when `CARGO_TARGET_DIR=target-pure cargo test -p innerword-board`, `-p
  innerword-assistant` and `-p innerword-agent` pass (the agents' suite through the app
  too), and the crates that call `side_system_prompt` compile (only `innerword-assistant`
  does).

### Package B: `innerword-assistant-ui` (GPUI, `target/`)

- Starts at once with what needs nothing of A (the action, its key and command,
  `ActionState`, `AnswerAction::state_for` and `menu_last`, the per-answer cache), then
  uses A1's items (`answer_drawing`, `shown_markdown`, `drawing_sentence`,
  `without_drawings`, `drawing_fits`, `DrawingTarget`, `ElementNames`, `Drawing`,
  `ChatSession::set_drawing_target` / `with_drawing_target` / `mark_drawn`,
  `Message::drawn_at`) once A1 is committed. It needs nothing of A2 (its tests of the
  started prompt compare against `side_system_prompt(tools, target)`, whatever A2 puts
  in it).
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
| A (board) | A (assistant), A (agent), C | `Drawing` (and its methods), `DrawingError`, `is_id`, `changes_that_change`, `draw`, `Drawn` (`is_nothing`, `unchanged`), `DrawError`, `place`, `Placement`, `PLACE_MARGIN`, `PLACE_GAP` (4.1) |
| A (assistant) | B, C | `DrawingTarget::of`, `answer_drawing`, `drawing_fits`, `without_drawings`, `shown_markdown(md, streaming, target, names)`, `drawing_sentence(drawing, target, names)`, `ElementName`, `ElementNames`, `escape_markdown`, `DrawingProblem::reason`, `ChatSession::set_drawing_target` / `with_drawing_target` / `drawing_target` / `mark_drawn`, `Message::drawn_at`, the re-exported `Drawing` (4.2) |
| B | C | `DrawAnswer`, `ACTION_DRAW`, `DRAW_AGAIN_WITHIN`, `AssistantPanelEvent::Draw { message, target, drawing }`, `DocumentRef::with_drawing` / `with_origin_note`, `AssistantPanel::set_drawing_target(target, why_not)` / `drawing_target` / `set_element_names` / `draw` / `draw_target_answer` / `draw_target` / `show_answer` / `mark_drawn` / `offer_message`, `ActionState`, `AnswerFacts`, `AnswerAction::state_for` / `menu_last`, `MessageListOptions::drawings`, `debug_state`'s `draw=` and `drawn=` (4.3) |
| C | the writer | the behaviour of sections 1, 5, 6 and 7 |

Nothing changes in `innerword-board-web` (no glue change, no new call, no rebuild of
`dist/`: placement uses the status's scroll and zoom and the page element's bounds, 5.3,
with the error that is accepted there; the undo is `adopt`'s), in `innerword-cli`,
`innerword-mcp`, the skill, or `innerword-assistant-quick`. `innerword-agent` changes only
by calling `changes_that_change` from `innerword-board` (A2).

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
  step. (2) The placement's view is the status's scroll and zoom and GPUI's size of the
  page element (5.3): with the shape library docked its right part can be under the
  sidebar, and a status up to 250 ms old can miss the end of a scroll; the drawing still
  lands free, a little off the middle. The exact view is a glue change (`center` answering
  `width`, `height`, `zoom`) with a rebuild of `dist/`. (3) The system prompt grows by
  about 1,150 tokens on a whiteboard and 900 on a document, in every request of the
  conversation (cached by the engine). (4) A model may copy the history's note instead of
  writing a block; the note is worded as the app's and both sections say so (3.5), and a
  reply with no block simply has no button.

## 13. After the review

The design was reviewed against the code before building; it takes nearly every point.
Where it does not, or chose between two fixes:

- **The agent channel at step 3 of 5.1** is not asked again. `must_wait` is a step of a
  request that keeps its own clocks, not a question; an agent's request that waits finds
  the drawing's adopt in flight and waits for it as for any adopt, then merges on the
  scene with the drawing. What is checked again is the adopt in flight, which is the real
  collision (5.1).
- **The view for placement** is taken from the status and the element's size (the
  critic's second option), not from an extended `center`: no rebuild of `dist/`, no fourth
  crate; the error with a docked shape library is accepted and written down (5.3, 12).
- **The document section of the prompt** stays in every document conversation; its cost
  is accepted and written down, with the fallback (3.1).
- **"Back to the document"** is a button of the toast and Quick Open for the keyboard, not
  a new command (6).
- **`cmd-shift-d`** is bound once, in `AssistantPanel`; the file has 425 entries, not 426
  (7).

## 14. After the review of the build

Two reviewers read the built feature. What changed:

- **A link is read one way.** `innerword_board::link_host` is the host the converter judges
  a link by and the line names it by: the converter refuses an `http(s)` link with a
  backslash (a browser reads it as `/`, `https://evil.example\@good.example/` goes to
  evil.example) and a link whose host is empty (`https:///evil.example`, which a browser
  opens as evil.example), has a `%` or white space; the line names the whole address,
  cut, should a link without a host come all the same, never nothing (section 8).
- **The toast's Undo is bound to one load of the page** (`DrawnAt`: the whiteboard, its
  session, `BoardState::loads`, counted at every `ready`, and the rev): the page's revs
  start again at 0 at every load, so after a reload, a crash, a failed adopt or the
  whiteboard opened again a stroke of the writer's was at a rev below the drawing's and
  Undo would have taken it out (5.4).
- **No second adopt after the flush**: the flush of 5.1 can itself send an adopt (a stale
  save merged); the press then says `BOARD_BUSY` as before the flush.
- **New Whiteboard from Answer that cannot open** the whiteboard says where it is
  ("Made the whiteboard “…” in Part one from the answer. It could not be opened: it is in
  the library.", `NOT_OPENED`).
- **A refusal names its list** (`DrawError::Entry`): "the element at index 1 of add (id)",
  "the change at index 0 of update (label)" (5.5).
- **The history note is plain text**: ids and hosts reach the model as written (an
  Excalidraw id has `_` and `-`); only the shown line escapes Markdown (3.5).
- **The list of shortcuts finds the key by "draw"**: the command's title is "Draw answer
  (on the whiteboard, or as a new whiteboard)" (`DRAW_ANSWER_TITLE`, as the zoom's
  Automatic names both its meanings); the palette names the one that applies, "Draw
  answer on whiteboard" or "New whiteboard from answer" (7).
- **Several blocks are said to be drawn together**: a second block's line ends "Drawn
  together with the drawing above." (`DRAWN_TOGETHER`), and a block that adds an id a
  block above adds cannot be used, before the press: "its second block adds cache again"
  (`DrawingError::AddedAgain`; an id written twice in one block, `IdTwice`) (2.2).
- **The prompts** say "plain JSON (no comments, no trailing commas), and its fences start
  their own lines, never inside a list item or a quote" and that "a change never has
  \"label\"" (4,369 and 3,230 characters, within 4,600 and 3,600) (3.5).
- **An id the whiteboard does not have** is said in the line before the press, with the
  names the host gives: "…; the whiteboard has no api." (2.3).
- **With no answer yet** the key says "There is no answer to draw yet." (`NO_ANSWER_YET`).

### After the acceptance run

The acceptance run's screenshots showed six things; each was fixed and has a test.

- **A drawn answer's line reads as it did when it was drawn.** The line was worked out
  against the live scene, so after Draw removed "Old note" it read "…; removes old; the
  whiteboard has no old." The press keeps the names the line gives what the drawing
  changes, removes or takes a link from, as the whiteboard had them before the drawing
  (`names_the_drawing_uses` over the flushed scene), with the answer
  (`Message::drawn_names`, stored with the conversation); the line of a drawn answer reads
  them first, then the live names (`NamesWhenDrawn`). An id that was not on the whiteboard
  at the press is still said to be missing. After `cmd-z` the line keeps the names of the
  press. The history note is unchanged (2.3, 3.5).
- **What a drawing added and changed is brought into view.** A drawing that names what is
  there stays where it was written (5.3), and "Postgres" landed under Excalidraw's side bar,
  half outside the canvas. After the adopt (the page runs its calls in order) the app asks
  `innerword_board::reveal` whether the bounds of `Drawn::shown_ids` (what was added with
  its labels, and what was changed) are seen whole in the canvas less Excalidraw's islands
  (`EXCALIDRAW_ISLANDS`: 72 at the top, 52 at the right, 64 at the bottom, 12 at the left, CSS
  pixels, measured in the run's window with a little room), and when not, sends the zoom
  call with a scroll (`calls::Zoom::View`, `{"to", "scrollX", "scrollY"}`; the glue's
  `zoom` sets the three in one `updateScene`, `dist/` rebuilt): at the same zoom as little
  as brings it in, `REVEAL_MARGIN` (24) from the edges, and only a drawing that cannot be
  seen whole at the zoom zooms out, to fit, centred; never in. The elements never move for
  it, and the toast and its Undo are as before. This replaces the zoom to fit of a drawing
  put beside the drawing (5.3, step 3): that one zooms out to the new elements only.
- **The black band over a modal on a whiteboard was holes that overlap.** The view's
  mask (`WebPage::set_holes`) is a path of the view's bounds and every hole, filled
  even-odd: a point two holes share is not cut. The toast "Drew on “Plan”…" stood over the
  version history's footer, its hole met the modal's, and the covered view (dimmed, dark)
  showed in the band where they met; it stayed there at 1 s (a scripted run on a whiteboard,
  a toast, a second toast, then the version history), so it was not a stale hole. The holes
  are made disjoint before the path is built (`frame::disjoint`: bands between their edges,
  the spans in each merged). Pages had the same mask and are fixed with it.
- **The toast with "Back to “Chapter 2”" goes when it has nothing more to do**: when the
  document is opened any way (`Workspace::open_document`, which quick open, the library and
  the button all go through) and when the new whiteboard is trashed (also with its folder)
  or deleted (`back_toast_follows`, `AssistantState::back_toast`).
- **The palette's first run marks the answer where it is seen.** The mark of the message
  list was drawn only while the keyboard was in the list. `MessageList::mark_shown` (what
  `AssistantPanel::show_answer` does) draws it while the keyboard stays where it was (on
  the canvas), and the key and the command draw that answer, until another message is
  marked or a new one comes; drawing it takes the mark away (7).
- **Over a page the side chat says "page"**: "Ask about this page" in its empty state and
  its composer (`DocumentRef::with_kind`, `ItemKind`); a whiteboard that cannot be drawn on
  now is still a whiteboard there.
