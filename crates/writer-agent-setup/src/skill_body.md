# The Writer

The Writer is a writing app for macOS. Its library holds the user's folders and documents (books, chapters, drafts, posts, notes) as rich text. This skill gives you `writer`, a command line tool that reads and changes that library directly. The app does not have to be running.

## Calling the tool

The tool is not on the PATH. Call it by its full path, in quotes because the path contains a space:

```
{{WRITER_QUOTED}} list
```

The examples below write `writer` for short. Always run the full quoted path.

- `writer --help` lists every command, and `writer <command> --help` explains one, with examples.
- Add `--json` to any command for JSON on standard output. Errors go to standard error and set the exit code.
- A document or a folder is named by its id (`doc:12`, `folder:3`) or by its path in the library (`"The Salt Road/Part one/Chapter 1"`). Every output shows ids. Once you have an id, use it: a path can fit several items, an id cannot.
- Text goes in and comes out as Markdown. Give it with `--content "..."`, with `--file <path>` or on standard input. For anything longer than a line, write a file and pass `--file`: that avoids shell quoting mistakes.

## While The Writer is open

While the app is open you can read (list, search, recent, info, get, comments, history, export), but every change is refused with exit code 5 and nothing is changed. That is deliberate: the app has the library in its hands and the two would overwrite each other.

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

`list` prints the library as a tree with ids, `search` is full-text with snippets, `info` gives the path, the word and character counts, the dates and the settings of one item.

Read a document:

```
writer get doc:12
writer get doc:12 --format text
```

Markdown is the default; `--format text` is the plain text without markup.

Write. Every command that changes something takes `--dry-run`, which reports what would change and changes nothing:

```
writer create "Chapter 4" --in "The Salt Road/Part one" --file chapter4.md
writer edit doc:12 --old "teh lighthouse" --new "the lighthouse"
writer append doc:12 --file scene.md
writer insert doc:12 --after "Chapter 2" --content "*Three days later.*"
writer replace-section doc:12 "Synopsis" --file synopsis.md
writer set doc:12 --file rewritten.md
```

`edit` replaces an exact piece of text: `--old` must stand in the document exactly once (or pass `--all`), and everything around it keeps its formatting and its comments. `insert` also takes `--at-start` and `--before <heading>`. `replace-section` replaces what stands under a heading. `set` replaces the whole text and is the last resort.

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

`--quote` is the exact text the comment is anchored to, and it must stand in the document exactly once.

History. A version of the document is kept before every change this tool makes, so anything can be taken back:

```
writer history list doc:12
writer history show doc:12 87
writer history restore doc:12 87
```

Export a document, or a folder as one file, to Markdown, DOCX or plain text (the ending of the path chooses the format):

```
writer export "The Salt Road" --to ~/Desktop/salt-road.docx
```

## Working well

1. Read before you change. `get` the document first and work from what it says now, not from memory of an earlier turn: the user may have written since.
2. Change as little as you can. Use `edit` with exact text, or `append`, `insert` and `replace-section`. Do not rewrite a whole document with `set` to change a sentence: it discards formatting that Markdown cannot carry and the comment anchors in the text.
3. When the user asks for feedback, a review or suggestions, comment (`comment add`) and leave the text alone. Change the text when they ask for the change.
4. When you are not sure what a command will do, run it with `--dry-run` first.
5. Use the ids that earlier output gave you. After `create`, `duplicate` or `mkdir`, the output has the new id.
6. Tell the user what you changed and in which document. They can see every version in the app under "Version history".

## Exit codes

- 0: done.
- 1: failed (the message says why).
- 2: the command line is wrong. Read `writer <command> --help`.
- 3: not found. Check the id or the path with `list` or `search`.
- 4: a name fits several items. The message lists them with their ids: use the id.
- 5: The Writer is open and the change was refused. See above.
- 6: the text to replace, the quote or the heading is not in the document. `get` it again and copy the text exactly.
- 7: the text occurs more than once. Give more of the surrounding text, or pass `--all` to `edit`.
- 8: not possible as asked (a name that is taken, a folder moved into itself, an item that is not in the trash).

Installed by The Writer {{VERSION}}. The app's Agents panel updates or removes this skill.
