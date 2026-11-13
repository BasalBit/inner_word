# The Writer

The Writer is a writing app for macOS. Its library holds the user's folders and documents (books, chapters, drafts, posts, notes) as rich text. `writer` is its command line tool: it reads and changes that library, whether the app is running or not.

## Calling the tool

The tool is not on the PATH. Call it by its full path, written exactly as it is here (the quotes are needed when the path contains a space):

```
writer --help
```

Every example here is a complete command line. Only the ids, names and files in them are made up.

- `--help` lists every command. After a command (`edit --help`) it explains that command, with examples.
- `--json` after any command prints the result as JSON on standard output. Errors go to standard error, and the exit code says what kind of failure it was.
- A document or a folder is named by its id (`doc:12`, `folder:3`) or by its path in the library (`"The Salt Road/Part one/Chapter 1"`). Every output shows ids. Once you have an id, use it: a name can fit several items, an id cannot.
- Text goes in and comes out as Markdown. Give it with `--content "..."`, with `--file <path>`, or on standard input with `--stdin`. `--content` is taken as it is: a `\n` in it is a backslash and an n, not a line break. For anything longer than a line, write a file and pass `--file`: that avoids shell quoting mistakes.

## While The Writer is open

You can read and change the library whether the app is open or closed.

While the app is open, a change is not written behind its back: the tool hands it to the app, the app makes it, and the user sees it in the window at once, with a short note that names you. They can take it back there with Undo, and the version from before the change is in the version history. A read gives you the text as it is on their screen at that moment, also what they typed a second ago. So:

- Say what you changed and in which document, every time. The user sees the result arrive, not your commands.
- Pass `--author` with your name. The note in the app and the version kept before the change carry it.
- Read right before you change, and keep each change small and exact: the user may be writing in the same document.

Exit code 5 means that the app is open and did not make the change. The message says which case it is and what to do:

- Changes by agents are switched off in the app (the switch "Agents can make changes while The Writer is open" in its Agents panel), or the app is a version before 0.6.0. Nothing was changed. Do not retry, and do not quit the app yourself: tell the user what the message says, and wait. Reading in the meantime is fine.
- The app was busy for a moment (the user is in the middle of typing a character, a dialog is open, the app is quitting). Nothing was changed: run the command again after a few seconds.
- The message says that it is not known whether the change was made. Read the document again before you repeat anything.

## Commands

Look around:

```
writer list
writer list "The Salt Road/Part one"
writer search "lighthouse keeper" --limit 5
writer recent
writer info doc:12
```

`list` prints the library as a tree with ids, `search` is full-text with snippets, `info` gives the path, the word and character counts, the dates, the headings and the settings of one item.

Read a document:

```
writer get doc:12
writer get doc:12 --format text
```

Markdown is the default. `--format text` is the plain text without markup. `--section "<heading>"` prints only what stands under that heading, and `--comments` adds the comment threads below the text.

Write. Every command that changes something takes `--dry-run`, which reports what would change and changes nothing:

```
writer create "Chapter 4" --in "The Salt Road/Part one" --file chapter4.md
writer edit doc:12 --old "teh lighthouse" --new "the lighthouse"
writer append doc:12 --file scene.md
writer insert doc:12 --after "Chapter 2" --content "*Three days later.*"
writer replace-section doc:12 "Synopsis" --file synopsis.md
writer set doc:12 --file rewritten.md
```

`edit` replaces an exact piece of text. `--old` is the text as `get --format text` prints it (a piece copied from the Markdown is found too) and must stand in the document exactly once: quote enough of the words around the change to make it so. The words you quote and leave as they are keep their formatting, and so does everything around them, with its comments. When the text occurs several times the error lists the places with their numbers; `--occurrence <n>` takes one of them and `--all` every one. `find doc:12 "some words"` shows where a text stands without changing anything. `insert` also takes `--at-start`, `--before <heading>` and `--end-of <heading>`. `replace-section` replaces what stands under a heading and keeps the heading. `set` replaces the whole text and is the last resort.

Images. A document can hold images. When you read a document as Markdown an image is a line `![description](image:<id>)`. Keep that line as it is and the image stays, with its size. Leave it out of a text you write back and the image is removed (the version kept before your change still has it). To add an image, give a local file:

```
writer image add doc:12 ~/Pictures/harbour.jpg --alt "The harbour at dusk"
writer image list doc:12
writer export doc:12 --to ~/Desktop/chapter.md
```

or write `![what it shows](/absolute/path.png)` in the Markdown you send. PNG, JPEG, GIF, HEIC, TIFF, WebP and BMP work; an address on the web stays a link. `image add` takes `--width <percent>` (a share of the text column), `--align left|center|right` and `--at` with a place; `image list` prints the images of a document with their sizes. `edit` refuses an `--old` that runs across an image: replace the text before and after it separately, or give `--old` as Markdown with the image's line in it. You cannot see a picture through this tool: to see an image, export the document as Markdown to a file. The pictures are written into a folder beside it.

Organise:

```
writer mkdir "The Salt Road/Part two"
writer rename doc:12 "Chapter 1: The Harbour"
writer move doc:12 "The Salt Road/Part two"
writer duplicate doc:12
writer trash doc:12
writer list --trash
writer restore doc:12
```

`trash` moves a document or a folder to the trash, from where `restore` brings it back. Do not pass `--permanently` unless the user asked, in so many words, for something to be erased for good.

Comment, to review a text instead of rewriting it. The comments appear in the app's comments panel under the author name you give:

```
writer comment list doc:12 --open
writer comment add doc:12 --quote "the keeper could not swim" --body "Set this up earlier?" --author Claude
writer comment reply doc:12 3 --body "Done in chapter 1." --author Claude
writer comment resolve doc:12 3
writer comment reopen doc:12 3
writer comment delete doc:12 3
```

`--quote` is the exact text the comment is anchored to, as `get --format text` prints it, and it must stand in the document exactly once. The number after the document is the id of the thread, which `comment list` prints.

History. A version of the document is kept before every change this tool makes to its text (and before a comment is deleted), so anything can be taken back. The app shows these versions with their reason, such as "Before an edit by Claude", when you pass `--author`:

```
writer history list doc:12
writer history show doc:12 87
writer history restore doc:12 87
```

Export a document, or a folder as one file, to Markdown, DOCX or plain text. The ending of the path chooses the format (`.md`, `.docx`, `.txt`). A file that exists is left alone unless you pass `--overwrite`:

```
writer export "The Salt Road" --to ~/Desktop/salt-road.docx
```

## Working well

1. Read before you change. `get` the document first and work from what it says now, not from what you remember of an earlier turn: the user may have written since.
2. Change as little as you can. Use `edit` with exact text, or `append`, `insert` and `replace-section`. Do not rewrite a whole document with `set` to change a sentence: that discards the formatting Markdown cannot carry and the places comments are anchored to.
3. When the user asks for feedback, a review or suggestions, comment (`comment add`) and leave the text alone. Change the text when they ask for the change.
4. When you are not sure what a command will do, run it with `--dry-run` first.
5. Use the ids that earlier output gave you. After `create`, `duplicate` or `mkdir`, the output has the new id.
6. Tell the user what you changed and in which document. While the app is open they see the change arrive and can undo it there; they find every earlier version in the app's version history, and you can restore one with `history restore`.

## Exit codes

- 0: done.
- 1: failed. The message says why.
- 2: the command line is wrong, or what it asks for cannot be done as asked (a file that is no image, a text to replace that runs across an image). The message says which; the command's `--help` has the rest.
- 3: not found (a document, a folder, a heading, a comment thread, a version). The message says what there is; check the id or the path with `list` or `search`.
- 4: a name fits several items. The message lists them with their ids: use the id. For a heading that occurs several times, pass `--occurrence <n>`.
- 5: The Writer is open and did not make the change. The message says why, whether anything was changed, and what to do; see "While The Writer is open".
- 6: the text to replace or to comment on is not in the document. `get` it again and copy the text exactly.
- 7: the text occurs more than once. Quote more of the words around it, or pass `--occurrence <n>`, or `--all` to `edit`.
- 8: erasing for good needs `--permanently`. Only when the user asked for exactly that.
- 9: the item is in the trash (`restore` it first), or it is not and the command needs one that is.
- 10: there is no library at that place. The Writer creates it on its first run.
- 11: another tool is writing to the library at this moment. Run the command again.
- 12: the file an export would write exists. Choose another path, or pass `--overwrite`.
- 13: the stored document cannot be read as a document. Leave it alone and tell the user.

Installed by The Writer {{VERSION}}. The app's Agents panel updates or removes this skill.
