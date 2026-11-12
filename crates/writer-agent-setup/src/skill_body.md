# The Writer

The Writer is a writing app for macOS. Its library holds the user's folders and documents (books, chapters, drafts, posts, notes) as rich text. `writer` is its command line tool: it reads and changes that library directly, whether the app is running or not.

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

While the app is open you can read (list, search, recent, info, get, comments, history, export), but every change is refused with exit code 5 and nothing is changed. That is deliberate: the app has the library in its hands, and the two would overwrite each other.

When a change is refused, do not retry it and do not quit the app yourself. Tell the user: "The Writer is open, so I can read your library but not change it. Quit The Writer (Cmd-Q, it saves by itself) and tell me to go on." Then wait. Reading in the meantime is fine.

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
6. Tell the user what you changed and in which document. They find every earlier version in the app's version history, and you can restore one with `history restore`.

## Exit codes

- 0: done.
- 1: failed. The message says why.
- 2: the command line is wrong. Read the command's `--help`.
- 3: not found (a document, a folder, a heading, a comment thread, a version). The message says what there is; check the id or the path with `list` or `search`.
- 4: a name fits several items. The message lists them with their ids: use the id. For a heading that occurs several times, pass `--occurrence <n>`.
- 5: The Writer is open and the change was refused. See above.
- 6: the text to replace or to comment on is not in the document. `get` it again and copy the text exactly.
- 7: the text occurs more than once. Quote more of the words around it, or pass `--occurrence <n>`, or `--all` to `edit`.
- 8: erasing for good needs `--permanently`. Only when the user asked for exactly that.
- 9: the item is in the trash (`restore` it first), or it is not and the command needs one that is.
- 10: there is no library at that place. The Writer creates it on its first run.
- 11: another tool is writing to the library at this moment. Run the command again.
- 12: the file an export would write exists. Choose another path, or pass `--overwrite`.
- 13: the stored document cannot be read as a document. Leave it alone and tell the user.

Installed by The Writer {{VERSION}}. The app's Agents panel updates or removes this skill.
