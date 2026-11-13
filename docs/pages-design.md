# Pages: design

Status: the spec the builders work from. Written 2026-11-13 against `main` at 94b72f2
(0.7.0: printing and images). Every name of existing code below was read in that commit.

The feature in one paragraph. A **page** is an HTML page (with its CSS, JavaScript,
fonts, pictures, sounds) kept in the library as an item beside the documents. It is shown
**rendered**, never as code, in a WebKit view inside the window, and it works **offline
for good**: everything it needs is downloaded once, when it is added, and stored in the
library; showing it never touches the network. A page that cannot be stored whole is not
stored at all, and the reason names what is missing. Agents add pages with
`writer page add` and the MCP tool `add_page`, several at once; each page remembers the
address it came from, so a later "copy everything" adds only the new and the changed
ones. The text the page shows (after its scripts ran) feeds library search, agents'
reads and the assistant. The writer adds pages with File > Import Page… and by dropping
`.html` files or folders, prints them and exports them.

## 0. Words used here

| Word | Meaning |
|---|---|
| page | A library item of kind `page`: a row of `documents` with `kind = 'page'`. |
| document, text | A library item of kind `text`: what the library held until now. |
| input | What a page is made from: one `.html` file, or one folder with an entry. |
| entry | The HTML file a page opens with (`index.html` of a folder by default). |
| original files | The input as it was given, byte for byte. |
| served files | What the view loads: the original files with every external reference rewritten to a stored copy, plus those copies. |
| external | A piece the page names on the network (`https://fonts.gstatic.com/...`). Fetched once at add time, stored under `_external/<host>/...`. |
| blob | Stored bytes, named by their hash. A font used by ten pages is one blob. |
| file id | `FileId`: the first 16 bytes of the SHA-256 of a blob, 32 lowercase hex digits (the same rule as `ImageId`). |
| manifest | The JSON a page row holds in `documents.content`: paths, file ids, media types, externals, source address, the rendered text. |
| the check | A headless WebKit load of the served files with the network blocked, at add time. It decides "whole or not" and reads the rendered text. |
| the view | The live `WKWebView` in the app window that shows the open page. |
| bundler | The new pure crate `writer-page`: input to served files. |
| WebKit crate | The new crate `writer-web`: everything that talks to WebKit (no GPUI). |

## Decisions at a glance

1. A page is a row of `documents` with a new column `kind = 'page'`. `library.db` goes to
   schema 4. Tree, folders, order, trash, versions, search and the sidebar work for pages
   without a second table.
2. The bytes go into `images.db`, in a second table `files`, content addressed. The copy
   among the backups, the damaged-file handling and the collector cover it. `images.db`
   goes to its schema 2.
3. Per file 32 MB, per page 64 MB of distinct bytes (served and original together), one
   page per request on the live channel. Reading Orbitarium (17 MP3s, about 12 MB) fits
   five times over.
4. The bundler is a pure crate. It fetches with `/usr/bin/curl` (no TLS stack compiled),
   with a Safari user agent so Google Fonts answers with WOFF2.
5. Whole or nothing is decided by a headless WebKit load with a
   `Content-Security-Policy` header on every response of the page and a listener for
   `securitypolicyviolation`: every request that would leave is blocked **and named**.
   Tried on this Mac (Appendix A): 13 kinds of request, all blocked, all reported, zero
   hits on a local server. A content rule list blocks the same again.
6. The check runs in a child process of the tool (`writer __page-check <dir>`), so the
   MCP server's main thread keeps reading stdin. In the app it runs in-process on the
   main thread (WebKit does the work in its own processes).
7. While the app is open the tool still does all the preparing (reading, fetching, the
   check). The request carries the blobs as base64 (protocol version 3). The app's
   listener thread stores the blobs before the request is queued, so the main thread only
   writes the row.
8. In the window the view is a native subview above GPUI's view, kept on the bounds of a
   GPUI element. GPUI overlays (palette, modals, context menus) are shown over a
   **snapshot** of the page while they are open; toasts are let through **holes** cut in
   the view's layer. Snapshots are `RenderImage`s released with `Window::drop_image`.
9. Keys: GPUI's focus on the page element and AppKit's first responder on the view are
   kept in step. Command keys reach GPUI first; the rest goes to the page, except the
   few keys the keymap binds in the page's own key context (`ctrl-enter`, `shift-f10`).
10. A page cannot be edited. Text commands are greyed out (as with no document open) and
    left out of the palette; the commands of a document that make no sense for a page
    say so in a toast. Agents get exit code 15 (`wrong_kind`) for a text operation on a
    page and 14 (`page_incomplete`) for a page that could not be kept whole.
11. Print goes through WebKit's print operation into a PDF, then through the existing
    `printing::PrintTarget`. Export writes the served files as a folder (or the
    original files). No single-file export in v1.
12. Tests never reach the internet (a server on 127.0.0.1) and the default runs of GPUI
    crates never open WebKit: the app has a page engine seam like `printing::PrintTarget`.

---

## 1. What a page is in the library

### The kind of item

A page is a row of `documents`. Two columns are added (schema 4, below): `kind` (`'text'`
or `'page'`) and `source_url` (the address the page came from, pages only).

What a page row holds:

| Column | For a page |
|---|---|
| `kind` | `'page'` |
| `title` | The page's title (rule below). Renamed like a document. |
| `folder_id`, `position` | As for a document: pages sit among the documents of a folder, in one order. |
| `content` | The manifest JSON (section 2). Never parsed as a `writer_core::Document`. |
| `plain_text` | The rendered text: what the page showed after its scripts ran. Indexed by `documents_fts` through the existing triggers. |
| `word_count`, `char_count` | Counted from the rendered text by the rule of a text: `writer_core::Fragment::from_plain_text(text).to_document().stats()`. |
| `source_url` | The address, when one was given (`--source-url`); `NULL` otherwise. |
| `view_state` | Unused (`NULL`). |
| `created_at`, `updated_at`, `trashed_at`, `trashed_with` | As for a document. `updated_at` moves when a re-add changed the page. |

Why a row of `documents` and not a table of its own: the tree (`writer_store::LibraryTree`),
ordering among siblings (`folders::Siblings::Documents`), the trash with
`trashed_with`, version snapshots (`snapshots.document_id`), full-text search, the
sidebar, quick open, the library search, `writer list`, `recent` and every result type of
`writer-agent` already work on document rows. A second table would have to be joined
into every one of them.

What keeps a page from being read as a text: two choke points parse stored content,
`writer_app::document_io::load_document` and `writer_agent::content::parse`. Both are
only reached through code that now checks `DocumentMeta::kind` first, and as the last
line of defence **the store refuses to save text over a page and a page over a text**:
`Store::save_document` on a page row and `Store::save_page` on a text row fail with
`StoreError::WrongKind`. A code path that forgets the check fails loudly; it cannot
overwrite a page with the plain-text recovery of its manifest.

### Schema 4 of `library.db`

Appended to `MIGRATIONS` in `crates/writer-store/src/schema.rs`:

```sql
-- Version 4: library items may be pages (kind 'page'; their files are in images.db).
ALTER TABLE documents ADD COLUMN kind TEXT NOT NULL DEFAULT 'text';
ALTER TABLE documents ADD COLUMN source_url TEXT;
DROP INDEX documents_meta;
CREATE INDEX documents_meta ON documents(
    trashed_at, position, id, folder_id, title, word_count, char_count, created_at,
    updated_at, kind, source_url
);
CREATE INDEX documents_by_source_url ON documents(source_url)
    WHERE source_url IS NOT NULL;
```

- `DOC_META_COLS` gains `kind, source_url`. The listings stay answered by the covering
  index alone (`listings_of_live_documents_do_not_read_the_text` keeps holding, with the
  new columns).
- No `CHECK` on `kind`: an unknown value is read as an error of that row
  ("not a kind of library item"), as an unreadable id is.
- Snapshots get no column: a document never changes kind, and a snapshot's kind is its
  document's.

**What an older version does.** 0.7.0 (schema 3) and its `writer` tool refuse a library
of schema 4 with `StoreError::SchemaTooNew`, exactly as 0.6.0 refused schema 3 (the note
on `V3` in `schema.rs`). That is wanted: a version that does not know pages would show a
page's manifest as plain text (`load_document`'s recovery) and save that over it. The
migration runs at the first open by 0.8.0, whether or not a page is ever added, as V3 did.
Tests: `version_3_upgrades_to_4_and_its_documents_are_unchanged`,
`a_build_that_knows_schema_3_refuses_a_library_of_schema_4`.

### Title

In this order: `--title` (the app's import has none); the entry's `document.title` after the scripts ran, trimmed, at most 200
characters; the text of the first `<h1>` of the entry as the check saw it; the name of
the input (the file name without `.html`, or the folder name). A rename sets the flag
"somebody chose this title" (`document.custom_title.<id>`, the same setting documents
use); a re-add then keeps the chosen title and otherwise takes the new page's title.

### Folder, trash, versions

- Folder: `--folder` (with `--create-folders`), else the top of the library (question 1).
  The app's import puts pages into the folder selected in the sidebar, as a new document
  goes.
- Trash: as for documents. Deleting for good frees the page's blobs only through the
  collector (section 2).
- Versions: a re-add that changed the page first keeps the page as it was as a version
  (`Store::create_labeled_snapshot`, label "Before an update by Claude" with the
  `--author`). "Restore" of a version makes that manifest current again after keeping the
  current one as a version, as for documents. A snapshot's `content` is the manifest; the
  manifest holds the rendered text, so the version history can show it.
- Duplicate: the copy keeps `kind`, content and text; it gets **no** `source_url` (a copy
  is not "the page at that address", and a re-add must find one page).

### The source address and the original

- `source_url` is a column (for the lookup of a re-add) and is written into the manifest
  too (so a version says where it came from).
- The original files are kept beside the served files: the manifest lists both
  (`original` and `files`). Blobs are shared, so the original costs only the bytes of the
  files the bundler rewrote (the HTML, CSS and JS that named externals). Export can write
  either set.

---

## 2. Where the bytes are

### One file: `images.db`, a second table

The blobs of pages go into `images.db` beside `library.db`, in a new table `files`. The
reasons for a second file are the reasons `images.rs` gives for pictures (a daily backup
is a copy of `library.db`, seven are kept; a library row changes with every autosave);
the reasons for **this** second file and not a third: everything that makes `images.db`
safe already exists and is tested, and works per file, not per table: opening and
making it (`file.rs`, `open`, `make`), WAL with `synchronous=FULL`, the background
connection (`ImageFile`), damage detection and moving aside (`is_damage`,
`replace_damaged`, `salvage`), noticing a replaced file (`FileId`/`file_id` of the path),
the copy in the backups directory (`copy.rs`), the collector with its two halves
(`collect.rs`), the app's timers and notes (`writer_app::images::collect_in_background`).
A third file would need all of it twice, and the app would have twice the notes. The
name stays `images.db` (renaming a file of a library is a migration of its own; the docs
say "pictures and the files of pages").

`images.db` schema 2 (`crates/writer-store/src/images/file.rs`):

```sql
CREATE TABLE files (
    id                 TEXT    NOT NULL UNIQUE,  -- 32 lowercase hex digits (FileId)
    byte_len           INTEGER NOT NULL,
    created_at         INTEGER NOT NULL,         -- unix ms
    unreferenced_since INTEGER,                  -- the collector's stamp, as for images
    bytes              BLOB    NOT NULL          -- last, as in `images`
);
```

- `IMAGES_SCHEMA_VERSION` becomes 2. `make` creates both tables in a new file. A file of
  version 1 is migrated (the `CREATE TABLE files` and `user_version = 2` in one
  transaction) by the first connection that opens it with `Mode::Write` or
  `Mode::Create`, under `MAKING`. A `Mode::Read` connection to a version-1 file answers
  "not there" for every file id (there are none). The copy in the backups directory is
  migrated the same way when `back_up_images` opens it for writing.
- No media type in the row: the same bytes can be served under two paths with two
  types. The manifest carries the type per path.
- No compression in v1. The archive's pages are mostly fonts, pictures and MP3s, which
  are compressed already.

### Content addressing and sharing

`FileId::of(bytes)` is the first 16 bytes of the SHA-256 (the rule of `ImageId::of`, a
type of its own so the two are never mixed). `Store::put_file` stores the bytes once;
bytes that are there are not written again, and the stamp of the collector is cleared,
exactly as `put_image` does (`write_row`'s `ON CONFLICT`). A font that ten pages use is one
row. The original and the served copy of an MP3 are one row.

### The manifest (the content of a page row)

```json
{
  "page": 1,
  "entry": "index.html",
  "files": [
    {"path": "index.html", "file": "3f2a9c1b5d6e7f80a1b2c3d4e5f60718", "type": "text/html; charset=utf-8", "size": 72275},
    {"path": "audio/01-intro.mp3", "file": "…", "type": "audio/mpeg", "size": 602876},
    {"path": "_external/fonts.gstatic.com/s/geist/v5/gyByhwUxId8gMEwRGFWNOITddY4.woff2", "file": "…", "type": "font/woff2", "size": 14200}
  ],
  "original": [
    {"path": "index.html", "file": "…", "type": "text/html; charset=utf-8", "size": 70011},
    {"path": "audio/01-intro.mp3", "file": "…", "type": "audio/mpeg", "size": 602876}
  ],
  "externals": [
    {"url": "https://fonts.googleapis.com/css2?family=Geist:wght@400..700&display=swap", "path": "_external/fonts.googleapis.com/css2__a31d67e5de.css"}
  ],
  "source_url": "https://claude.ai/artifact/ExampleArtifactId00001",
  "fingerprint": "64 hex digits",
  "added_at": 1794306980000,
  "checked": {"engine": "WebKit 621.1", "at": 1794306980000, "requests": 31, "warnings": []},
  "text": "Orbitarium\nas built at a3c2306 · 10 Nov 2026\n…"
}
```

- Every blob is named under the key **`"file"`** with 32 hex digits. That is the token
  the store reads (below); **do not rename it**, as `"image"` is not renamed.
- `files` and `original` are sorted by path. `type` is the media type served for that
  path. `size` is the byte length.
- `fingerprint` is the SHA-256, in hex, of the original input: for each original file in
  path order, its path, a NUL byte, the SHA-256 of its bytes. The same input gives the
  same fingerprint; a re-add compares it before fetching anything (section 3).
- `text` is the same text as `plain_text`. It is here so that a version (a snapshot holds
  only `content`) can be shown and searched in `history show`.
- The type of this JSON is `writer_page::Manifest` (section 12, WP2). Only `writer-page`
  writes it; everyone else reads it through that type.

### What the store reads in a manifest

`Store::referenced_files()` collects every `"file"`, optional white space, `:`, optional
white space, `"`, 32 hex digits, `"`, from `documents.content` (live and trashed) and
`snapshots.content`, with the `instr(content, '"file"') > 0` filter that
`scan::referenced` uses for images. No document of `writer-core` writes a `"file"` key
(read: `grep '"file"' crates/writer-core/src` finds nothing); a false hit only keeps a
blob alive.

### The backups

`back_up_images` (`copy.rs`) copies the rows of `files` the copy does not have, in the
same slices as images (`IMAGE_BACKUP_SLICE_BYTES` counts both tables together), and
`back_up_all_images` brings both all the way up to date. `ImagesBackedUp` gains
`files`, `files_copied`, `file_bytes_copied`, `files_waiting`. A file missing from
`images.db` and present in the copy is read from there and written back
(`read_from_copy` for files). A row leaves the copy only when the collector deleted it
and no kept backup names it (`named_by_kept_backups` scans for `"file"` too). So a backup
of `library.db` that is brought back finds the pages' files, as it finds its pictures.

### A damaged file

Nothing new: `replace_damaged` moves the file aside, `salvage` copies what can still be
read **from both tables**, and the rest comes back from the other file as it is asked
for. Bytes whose hash is not their id are not believed (`FileId::of(bytes) != id`), in
either file, and are taken from the other one. The app's notes about a file set aside
(`images_moved_aside`, `images.moved_aside_told`) cover it as they are.

A page whose file is in neither: the view's scheme handler answers 404 for that path, and
the app, when it opens the page, counts the files the library does not have
(`Store::file_info` for each, no bytes read) and says once: "Some files of “Reading
Orbitarium” are missing from the library (3). The page may not show as it did."

### The collector

`collect_unused_images` and `mark_unused_images` stamp, clear and delete rows of `files`
by the same two conditions (unnamed for `IMAGE_RETENTION_MS`, and since before the
oldest kept backup). Who else can hold a file id: nobody (no undo history, no clipboard,
no register names one), so the table at the top of `collect.rs` gains one row: "a page's
manifest, live, trashed or in a version: the scan finds it". `ImagesCollected` gains
`files`, `files_referenced`, `files_newly_unreferenced`, `files_deleted`,
`file_bytes_freed`. Deleting a page frees its space 30 days later, at a launch, as for
pictures.

### Limits

| Limit | Value | Where it is enforced | Why that much |
|---|---|---|---|
| One file | 32 MB | bundler (input and each external), and again where a request is carried out | The archive's largest file is 1.95 MB (a PNG of BasalBit), its largest MP3 1.37 MB. 32 MB leaves room for a short video and keeps one blob within one slice of the backup copy. |
| One page | 64 MB of distinct bytes, served and original together | bundler; again in `pages::add_page` | Archive: Lantern Redesign Concepts 16.2 MB served (125 files), Reading Orbitarium 11.8 MB (30 files). The 17 MP3s are about 11 MB. |
| Files per page | 2,000 | bundler | The archive's largest has 180 (Landing Variants). |
| Externals per page | 500 | bundler | The archive has at most 133. |
| One request on the channel | one page; `live::MAX_PAGE_BYTES` = 64 MB before base64 | tool and app | Base64 makes 86 MB, under the 256 MB a line may have (`live` module docs). |
| Rendered text | 2,000,000 characters (`writer_page::MAX_TEXT_CHARS`) | bundler (`prepare`) | Reading Orbitarium has 23,429. The rest is cut, with a warning. |
| Fetch | 30 s per address, 180 s per page, 6 at a time, 5 redirects | `CurlFetcher` | Google Fonts answers in well under a second. |
| The check | 60 s in all (`CHECK_LIMIT`); the tool kills the child at 75 s | `writer-web`, `ProcessChecker` | The archive's checker settled every page in 3 s. |

`images.db` itself has no limit, as for pictures.

---

## 3. Ingestion: the bundler (`writer-page`)

### Inputs

- **An `.html` or `.htm` file.** The page is that file, the local files it names
  (recursively, through HTML, CSS, JS, SVG), and every file under the file's folder whose
  relative path occurs as a string anywhere in the page's HTML, CSS or JS. The last rule
  is for pages that build a path at run time from a table of literal paths (Reading
  Orbitarium: `fetch(NARR.chapters[next.id].src)` with the chapters' `src` strings in the
  same file). The root of the page is the file's folder; a reference that leaves it
  (`../vendor/a.css`) is refused with: "index.html names ../vendor/a.css, outside its
  folder: add the folder that holds both and say which page opens with --entry."
- **A folder.** Every file under it, recursively, except names that begin with `.`,
  `__MACOSX`, `Thumbs.db`. The entry: `--entry <path in the folder>`, else `index.html`,
  else `index.htm`, else the one `.html` file at the top; several and none of these
  names: refused, "the folder has 3 pages at its top and no index.html: say which one
  opens with --entry".
- **Several inputs**: each is a page of its own, prepared and stored on its own (one
  operation, one version, one outcome each). One failing does not stop the others.
- **No address as input in v1.** `writer page add https://…` is refused: "a page is added
  from a file or a folder on this Mac; fetching a live site is not supported. Save the
  page first (the skill says how)". Rendering a live site whole (logins, single-page apps,
  endless scripts) is another feature; the claude.ai artifacts arrive as files through
  the Artifact tool (section 9).
- Files are read as bytes. Text files are scanned as bytes too, and only the bytes of a
  reference are replaced, so a file in any encoding comes out as it went in.

### Finding every reference

One small tokenizer for HTML and SVG (not a full HTML5 parser): comments, doctype, CDATA,
the raw-text elements (`script`, `style`, `textarea`, `title`), attributes quoted or not,
character references decoded for reading and written back escaped. What is read:

| Where | What |
|---|---|
| Attributes | `src`, `href`, `poster`, `data`, `xlink:href`, `action`; `srcset` and `imagesrcset` (each candidate's URL); `style` (as CSS); `srcdoc` (as HTML, written back escaped) |
| Elements | `script`, `link`, `img`, `source`, `video`, `audio`, `track`, `iframe`, `embed`, `object`, `image` and `use` (SVG), `input type=image`, `meta http-equiv=refresh` (its `url=`), `base` |
| `<link rel=…>` | `stylesheet`, `icon`, `apple-touch-icon`, `preload`, `modulepreload`, `manifest`, `prefetch`: fetched and rewritten. `preconnect`, `dns-prefetch`, `prerender` to another origin: the element is removed (it would open a connection and serves nothing offline) |
| `<base href>` | Every relative reference of the file is resolved against it; then the element is removed |
| `<style>`, `.css`, `style=""` | `@import "…"`, `@import url(…)`, `url(…)` (quoted or not), the strings of `image-set(…)` and `-webkit-image-set(…)` |
| `<script>`, `.js`, `.mjs` | Static `import … from "…"`, `import "…"`, `export … from "…"`; `import("…")` and `` import(`…`) `` without `${`; `new Worker("…")`, `new SharedWorker("…")`, `importScripts("…")`; string literals that are an absolute `http(s)` address on a known CDN (`cdnjs.cloudflare.com`, `cdn.jsdelivr.net`, `unpkg.com`, `esm.sh`, `cdn.skypack.dev`, `ga.jspm.io`, `cdn.tailwindcss.com`, `code.jquery.com`, `fonts.googleapis.com`, `fonts.gstatic.com`); `//# sourceMappingURL=` lines of fetched files are removed |
| `<script type=importmap>` | The values of `imports` and of every scope in `scopes` |
| Inside a fetched external | The same rules, resolved against the external's final address (an ES module on `esm.sh` that imports `/react@18.3.1/es2022/react.mjs` gets that file from `esm.sh`) |

Not read: inline event handlers (`onclick="…"`), addresses assembled at run time. The
check finds those when the page asks for them while it loads.

`<a href>` to another site is left as it is: it is a link the reader may click, which the
view opens in the default browser (section 5). `<a href>` and `<iframe src>` to a file of
the page are references like any other.

`data:`, `blob:`, `about:`, `javascript:`, `mailto:`, `tel:` and `#…` are not references.

### Fetching each external once

- `writer_page::Fetcher` (trait), `CurlFetcher` the default: `/usr/bin/curl --silent
  --show-error --location --max-redirs 5 --proto =http,https --proto-redir =http,https
  --connect-timeout 10 --max-time 30 --max-filesize 33554432 --user-agent <Safari 17.5 on
  macOS> --dump-header <file> --output <file> --write-out '%{http_code}\t%{content_type}\t%{url_effective}' <url>`.
  Why curl: it is on every Mac, speaks TLS through the system, and costs nothing to
  build; a Rust TLS stack would be compiled into both target directories on a disk that
  is nearly full. The user agent is the one `_tools/archive.py` used
  (`Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like
  Gecko) Version/17.5 Safari/605.1.15`), which makes Google Fonts serve WOFF2.
- GET only, status 200 only. The media type comes from `Content-Type`, else from the
  ending of the final address, else `application/octet-stream`.
- Each address (without its `#fragment`) is fetched once per command: `CachingFetcher`
  wraps the fetcher for all pages of one `writer page add` and one import.
- Six at a time, a budget of 180 s per page, at most 500 externals, depth 8 for nested
  `@import` and module graphs.
- `MapFetcher` (address to bytes, in memory) is the fetcher of every test of every crate
  that does not run a 127.0.0.1 server.

### Rewriting

An external is stored at `_external/<host>/<path segments>` with every character outside
`[A-Za-z0-9._@+-]` written as `_`, a missing ending added for CSS (`.css`) and JS
(`.js`), and `__<first 10 hex of the SHA-1 of the query>` before the ending when the
address has a query (the scheme of `archive.py`'s `local_path_for`, which produced the
archive). Two addresses that come out the same get `__<8 hex of the address>`. The
reference is replaced by the **relative** path from the file that names it to the stored
copy (`../_external/fonts.gstatic.com/...`), so the served files also work as an exported
folder. A reference to a local file is left as it is.

### Whole or nothing: the check

After rewriting, the served files are loaded in WebKit with the network blocked (section 4
says where WebKit runs, section 5 how the network is blocked). The check:

1. loads the entry at `writer-page://check/<entry>`, waits for `load`, then until no
   request was answered or blocked for 500 ms, at least 1 s and at most 3 s after `load`;
2. reads `document.title` and `document.body.innerText` of the main frame;
3. loads every other HTML file of the served files the same way (at most 40, settling
   0.3 s to 1 s), and reads their titles and texts;
4. reports per load: whether `load` came, a navigation error, a crash of the WebContent
   process, every path the scheme handler answered 404, every request that was blocked
   (from `securitypolicyviolation`: its address and its directive; and every navigation
   the policy cancelled), the first 20 script errors.

The page is **not saved** (exit 14, `page_incomplete`) when:

- an external could not be fetched ("https://fonts.gstatic.com/s/x.woff2 could not be
  fetched: HTTP 404");
- a file is over 32 MB, or the page over 64 MB, or over 2,000 files or 500 externals;
- the entry did not finish loading in 60 s, failed to load, or its process crashed;
- any load asked for a path the page does not have ("the page asks for audio/18.mp3,
  which is not among its files");
- any load asked the network ("the page asks for https://api.example.com/grades at run
  time (data it fetches), which cannot be kept"; the directive gives the words: a font,
  a picture, a script, a style sheet, data it fetches, a frame, media; a cancelled
  navigation is "it goes to").

The sentence names the first five problems and counts the rest, and always ends "Nothing
was saved." Script errors are warnings in the result, not reasons (pages written for
claude.ai call `window.claude` and catch the error; Quiz Marking Desk does).

What the check cannot see: a request made later than its settling (a click, a timer of a
minute), and a request made inside a worker (Appendix A: blocked, but reported to the
worker's own scope, which the probe cannot reach). Both are blocked when the page is
viewed. The skill tells agents that a page whose data lives on a service is not kept.

### Extracting the text, in the same load

`text` is the entry's `innerText`, then for each other HTML file loaded a blank line, its
title on a line, and its `innerText`. Cut at `MAX_TEXT_CHARS`. That is `plain_text` and
the manifest's `text`.

### Re-adding

With a source address:

1. The tool reads the input and computes the fingerprint before fetching anything.
2. It looks up the live page with that `source_url` (`Store::page_by_source_url`: the
   most recently updated one, read-only connection, works while the app is open).
3. Same fingerprint: the outcome is **unchanged**. Nothing is fetched, checked or
   written. The request that goes on (to the app or to the direct write) carries no
   blobs, and the operation only confirms that the stored fingerprint is still the same.
4. Another fingerprint: the page is prepared and checked as a new one would be; when it
   is whole, the old one is kept as a version ("Before an update by Claude") and the row
   gets the new manifest, text, counts and (unless chosen by a rename) title. Folder and
   position stay. The outcome is **updated**.
5. No live page with that address (none, or only in the trash): **added**, as new.

Without a source address every add is a new page. The app's import gives none (question
6).

---

## 4. Where WebKit runs

| Who | What | Where WebKit runs |
|---|---|---|
| The app | the view of the open page | the app's main thread, a `WKWebView` in the window |
| The app | the check of File > Import Page… and of a drop | the app's main thread, an offscreen `WKWebView` (`writer_web::check`) |
| `writer page add` | the check | a child process: `writer __page-check <dir>` |
| `writer mcp` (`add_page`) | the check | the same child process |
| Tests of GPUI crates | nothing | the fake page engine (section 6, "The test seam") |

### The child process

`writer_agent::ProcessChecker` (WP5) runs `<program> __page-check <dir>`, where
`program` is `std::env::current_exe()`: the tool runs itself. The MCP server is the same
binary, so it does the same; its main thread waits for the child and goes on reading
stdin afterwards. The MCP tool call is synchronous anyway.

The directory (made by `writer_page::write_check_dir`, mode 0700, under
`$TMPDIR/the-writer-<uid>/check-<pid>-<n>/`, removed afterwards):

```
request.json      writer_page::CheckRequest
files/<path>      every served file, at its path
```

The child (`crates/writer-cli/src/page_check.rs`, WP8) reads it, builds a
`writer_web::MemorySource`, calls `writer_web::check_blocking` (which makes
`NSApplication` with the activation policy "prohibited", so no Dock icon and no menu,
runs the check and returns), maps `writer_web::CheckOutcome` to `writer_page::CheckReport`,
prints it as one line of JSON on stdout and exits 0. Exit code not 0 or no JSON: the
checker failed ("the page could not be checked: <last line of its standard error>",
exit 1). The parent kills the child after 75 s.

`writer __page-check` is not in the command table (`spec.rs`) and not in the help: it is
handled in `main.rs` before parsing, like a program of its own.

`THE_WRITER_PAGE_CHECK=fake` in the environment makes `ProcessChecker` answer in the
process itself with `writer_page::static_check` (the text of the entry with its tags
taken out, its `<title>`, nothing blocked): for the tests of `writer-cli` and
`writer-mcp`, which run the built binary and must not open WebKit for every case.

### While the app is open

- **Who runs the check:** the tool, always, in its child process. The app never runs a
  check for an agent: the app does not read the files an agent names (the rule of
  `attach.rs`), and a check holds a WebContent process for up to a minute.
- **What travels:** `AddPageRequest` with `prepared: Some(PreparedPage::Page(…))`, which
  holds the blobs as base64, the served and original paths, the externals, the text, the
  title found, the fingerprint and the check's summary (types in section 12). An
  unchanged page travels as `PreparedPage::Unchanged { page, fingerprint }`, without
  bytes.
- **Payload size:** one page per request, at most `live::MAX_PAGE_BYTES` (64 MB) before
  base64. The tool refuses more before sending ("the page weighs 71 MB; a page can be 64
  MB at most"), and the app refuses it again.
- **Main-thread cost:** the app's listener thread (`crates/writer-app/src/live.rs`,
  the loop at `listener.accept()`) stores the blobs of an `add_page` that is not a dry run
  through an `ImageFile` of its own **before** it queues the request
  (`writer_agent::live::store_page_files`). Each blob is put once, its `data` is emptied
  and it is marked `stored`. On the main thread `pages::add_page` then checks each stored
  blob with `Store::file_info` (a row read, no bytes) and writes the row. A blob stored by
  a request that is then refused (switch off, too late) is unnamed and goes with the
  collector. This is the path CLAUDE.md names for images ("the channel's listener … stores
  what a request brings through an `ImageFile` of its own before the request is queued");
  for pages it is built, because a page is larger than a picture.
- **Protocol version 3** (`live::PROTOCOL_VERSION`). A request reads past fields it does
  not know; a version-2 app (0.7.0) would not know `add_page` at all. With the number
  changed the two refuse each other whole (`Refusal::Protocol`), as for version 2. (0.7.0
  cannot open a library of schema 4 anyway.)
- A dry run prepares fully (fetches, checks) and stores nothing, here or in the app. It
  reports what would be added and whether the page can be kept whole.

---

## 5. Isolation and the offline guarantee at view time

The page is untrusted content: it may have been written by a model that read anything.
The view gives it no way to the network, to the user's files or to the app.

### How a page is served

- Scheme `writer-page`, host `page-<document id>` in the view (`check` in the check):
  each page is its own origin, so pages do not share storage.
- `WKURLSchemeHandler` (`writer_web`): `webView:startURLSchemeTask:` is called on the main
  thread; the path is percent-decoded, looked up in the manifest, the bytes are read on a
  background queue (`PageSource::file`) and the answer is given back on the main queue.
  A task that `webView:stopURLSchemeTask:` stopped meanwhile is not answered (answering a
  stopped task raises an exception).
- `Range: bytes=a-b`, `a-`, `-n` answered with 206 and `Content-Range` (media needs it,
  spike finding b); `Accept-Ranges: bytes` on every answer; 416 for a range outside the
  file; 404 for a path the page does not have.
- Media types from the manifest. HTML without a `<meta charset>` and without a BOM is
  served with `charset=utf-8` when its bytes are valid UTF-8, otherwise without a charset
  (WebKit then reads the page's own declaration).
- Every answer for HTML and JavaScript carries
  `Content-Security-Policy: default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'; form-action 'none'; base-uri 'self'`
  (`writer_web::CONTENT_SECURITY_POLICY`; on JavaScript too, so a worker has it) and
  `X-DNS-Prefetch-Control: off`.

### Every way out, and the policy

| Way | Policy | By |
|---|---|---|
| Subresources (scripts, style sheets, fonts, pictures, media, `@import`) from `http(s)`, `ws(s)`, `ftp`, `file` | blocked | CSP, and a `WKContentRuleList` that blocks `^https?:`, `^wss?:`, `^ftp:`, `^file:` (`writer_web::BLOCK_RULES`) |
| `fetch`, `XMLHttpRequest`, `WebSocket`, `EventSource`, `navigator.sendBeacon` | blocked | CSP `connect-src` (default), rule list |
| `<iframe src>` to another site | blocked | CSP `frame-src`, and the navigation policy |
| Navigation of the page itself (`location = …`, `<meta refresh>`) | cancelled; allowed only to the page's own files, `about:blank`, `about:srcdoc` | `WKNavigationDelegate decidePolicyForNavigationAction` |
| A link the reader clicks to `http(s)` or `mailto` (`navigationType` link activated) | cancelled in the page, **opened in the default browser** (`cx.open_url`) | navigation policy, `PageEvents::open_externally` |
| `target=_blank`, `window.open` | own file: loaded in the same view; another site after a click: the default browser; otherwise nothing | `WKUIDelegate createWebViewWithConfiguration…` returns nil |
| Form submission | blocked | CSP `form-action 'none'`, navigation policy |
| Service workers | not possible on a custom scheme; the data store is not persistent | WebKit |
| Downloads (`<a download>`, attachments, a response WebKit cannot show) | cancelled (question 8) | `decidePolicyForNavigationResponse`, `navigationAction.shouldPerformDownload` |
| Media autoplay | never; a click starts it | `mediaTypesRequiringUserActionForPlayback = WKAudiovisualMediaTypeAll` (spike: 9 s of narration played aloud without it) |
| AirPlay | off | `allowsAirPlayForMediaPlayback = NO` |
| Camera, microphone | denied without asking | `WKUIDelegate requestMediaCapturePermissionForOrigin…` answers deny; the app declares no camera or microphone use |
| Geolocation | denied | the app declares no location use, so WebKit cannot ask |
| `<input type=file>` | the panel does not open; the page gets nothing | `WKUIDelegate runOpenPanelWithParameters…` answers nil |
| `alert`, `confirm`, `prompt` | `alert`: its text as a toast "The page says: …" (at most one per 5 s); `confirm`: Cancel; `prompt`: nothing | `WKUIDelegate` JavaScript panels, answered at once |
| Clipboard | a page cannot read it; it can write only from a click the reader made (WebKit's own rule); Copy of a selection works | `javaScriptCanAccessClipboard = NO` (default) |
| Link previews (force click) | off: they would load the address | `allowsLinkPreview = NO` |
| Back and forward swipes, magnification | off | `allowsBackForwardNavigationGestures = NO`, `allowsMagnification = NO` |
| Fraud warnings (Safe Browsing lookups) | off | `preferences.fraudulentWebsiteWarningEnabled = NO` |
| Full screen of an element | off | `preferences.elementFullscreenEnabled = NO` |
| Opening windows by script | off | `preferences.javaScriptCanOpenWindowsAutomatically = NO` |
| Web Inspector | off | `inspectable = NO` (default) |
| A bridge to the app | **none** | the view adds no `WKScriptMessageHandler` and no `WKUserScript` (the check has both, and only the check) |
| Storage (`localStorage`, `IndexedDB`, cookies) | kept while the app runs, per page, gone at quit (question 2) | one `WKWebsiteDataStore.nonPersistentDataStore` per app run, shared by every view; each page its own host |
| `window.claude` (claude.ai capabilities) | not there; pages that need it say so or fail quietly | nothing to do |

The rule list is compiled once into a `WKContentRuleListStore storeWithURL:` at
`<data directory>/WebKit/` for the app (so a test or a scripted run with
`THE_WRITER_DATA_DIR` writes only there) and at `$TMPDIR/the-writer-<uid>/WebKit/` for the
tool's child. In the spike the first contentful paint came after 346 ms instead of 161 ms
when the list had to be compiled first; the view loads the page only once the list is
there (`compile_rules` and `add_rules_and_load` in the spike), and the app compiles it in
the background at launch so that the first page opens at the faster time.

---

## 6. The view in the app

### What in the workspace assumes an open editor

Read in `crates/writer-app/src/workspace/`:

| Part | What it assumes | For a page |
|---|---|---|
| `Workspace::session: Option<Session>` (`session.rs`) | the open item is a text in `self.editor` | stays `None` while a page is open |
| autosave, `flush`, `saver.rs`, view state | the session | nothing to save; untouched |
| `render_editor_area` (`render.rs`) | `None` session: the empty state | a page: the page element |
| `render_title`, `display_title`, window title | `session.title` | the page's title, with the title menu for a page |
| `status_line`, `render_status_bar` | the editor's counts, save state, Vim chip | the page's items (below) |
| text commands at the root (`text_commands`, `forward_to_text`) | registered only `if self.session.is_some()` | not registered: greyed out in the menu bar, as with no document |
| spelling word commands | `session.is_some() && spell.is_some()` | not registered |
| `zoom_actions` (`zoom.rs`) | the editor's zoom | handlers for a page: the view's page zoom |
| `print_actions` (`print.rs`) | the editor's print pass | a page's print (section 7) |
| `on_export` (`export.rs`) | `ExportTarget::Open` is a text | a page: the page's export rows |
| `on_version_history` (`version_history.rs`, `load_document` in the preview) | a text | a page's versions (section 7) |
| `on_document_settings`, `on_choose_spelling_language`, page view toggle, comment commands, `on_ask_assistant` | a text | a toast that says what a page is (below) |
| `edit_where_the_keyboard_is` (`mod.rs`) | the editor or a text field | the page: its own `copy:`, `cut:`, `paste:`, `selectAll:`, `undo:`, `redo:` |
| `take_over_open_document` (`live.rs`) | `load_document` of the open text | an open page that changed is reloaded |
| `assistant_document_changed`, `assistant_open_document` (`assistant.rs`) | the session and the editor's Markdown | the page's text (section 8) |
| Vim mode | attached to the editor | does nothing on a page; its chip is not shown (it shows only with a session today) |

### The shape: a second kind of open item, beside the session

The least invasive shape keeps `Session` as it is and adds one field:

```rust
pub struct Workspace {
    // ...
    pub(crate) session: Option<Session>,          // an open text (unchanged)
    pub(crate) page: Option<page::PageSession>,   // an open page; never both
}
```

- `open_document(id)` reads the meta first. A page goes to `open_page(id)`: flush and
  close the text session (as `close_document` does: the editor gets an empty read-only
  document), set `self.page`, mark the row active in the sidebar, store
  `LAST_DOCUMENT`, emit `WorkspaceEvent::DocumentOpened(id)`, tell the assistant.
  Opening a text while a page is open drops the page session (the view goes, its
  WebContent process with it).
- `open_document_id()` keeps meaning "the open text". New: `open_page_id()` and
  `open_item_id()` (either). The sidebar's active row, `LAST_DOCUMENT`, the agents' note
  (Open, Undo) and the library events that close the open item use `open_item_id()`.
- `restore_session` opens a page like a document (the last open item may be one).
- A page that goes to the trash or is deleted closes as a document does
  (`open_document_left`).
- `crates/writer-app/src/workspace/page.rs` holds everything else of the view: the
  session, the element, focus, overlays, the title and status items, the page's
  commands. `crates/writer-app/src/pages.rs` holds the engine seam, the page source and
  (WP9) the import.

```rust
pub(crate) struct PageSession {
    pub id: DocumentId,
    pub title: String,
    pub source_url: Option<String>,
    pub manifest: writer_page::Manifest,     // read once when the page opens
    pub words: u64,
    pub view: Option<pages::PageViewHandle>, // made at the first prepaint
    pub state: PageState,                    // Loading | Loaded | Failed(String) | Crashed { at: Instant, again: bool }
    pub zoom: u16,                           // percent, 100 at open; not stored
    pub find: Option<FindStrip>,             // the find strip, while open
    pub overlay: Overlay,                    // None | Waiting | Snapshot(Arc<RenderImage>) | Holes(Vec<Bounds<Pixels>>)
    pub focus: FocusHandle,                  // the page element's
}
```

### Where the `WKWebView` sits and how it follows

- It is a subview of the window's `contentView`, ordered **above** GPUI's view
  (`addSubview:positioned:NSWindowAbove relativeTo:<GPUI view>`). The GPUI view is
  `crate::screenshot::native_view(window)`. As a sibling (not a subview of GPUI's view)
  key equivalents reach GPUI first (spike finding d) and GPUI's own drawing is not
  disturbed.
- The page element is a `gpui::canvas` that fills the editor area (below the find strip
  when it is open). In its prepaint it hands its bounds to the view: GPUI's window
  coordinates (top left, logical pixels) to the content view's (bottom left unless the
  view is flipped), as the spike's `frame_in` does. Same bounds: nothing is sent.
- The element that is not painted (another item opened, the window closing) drops the
  view: `PageViewHandle`'s `Drop` removes the web view from its superview, gives the
  first responder back to GPUI's view when the web view had it (spike `remove`), and
  releases it. One live web view at a time; the spike saw the WebContent process (130
  to 270 MB per open page) given back on close, and no leak over 20 switches. No cache
  of several views in v1.
- The window's `screenshot:` step captures native subviews (spike finding a), so visual
  QA sees the page.

### Focus and keys

- **One focus, kept in step.** The page element has a `FocusHandle` (key context
  `PageView` inside `Workspace`). In every frame (`render`), after
  `keep_focus_on_screen`: when the page element is focused and the web view is not the
  window's first responder, `makeFirstResponder:` the web view; when it is not focused
  and the web view is, `makeFirstResponder:` GPUI's view (the spike's `sync_focus`). The
  other way: the web view subclass's `becomeFirstResponder` (a click into the page while
  the keyboard was in the library) calls `PageEvents::wants_focus`, and the workspace
  focuses the page element.
- **Command keys go to GPUI first.** Spike finding d: GPUI's view is asked for key
  equivalents first, WebKit does not pass on the command keys it does not handle, and
  with the web view as a sibling of GPUI's view (not its subview) the key equivalents
  reach GPUI. GPUI dispatches the keystroke on the
  focused path (`Workspace`, `PageView`); the editor's bindings (context `Editor`) do not
  apply. What GPUI has no binding for goes on: to WebKit, then to the menu bar, which
  "answers its key through AppKit when GPUI has no binding for it where the keyboard is"
  (CLAUDE.md, "Commands for the text work wherever the keyboard is").
- **Every other key goes to the page** (letters, space, arrows, page keys, tab, escape):
  the page scrolls and runs its own keyboard handlers (Reading Orbitarium: space plays,
  arrows step). Except: the web view subclass's `keyDown:` asks
  `PageEvents::takes_key(&KeyPress)`; the app answers true when the installed keymap has
  a binding for that keystroke in the `PageView` context or a context around it
  (`crate::keymap::binds_at(&keystroke, &["Workspace", "PageView"])`, a new query of the
  installed keymap), and the subclass hands the event to GPUI's view (`keyDown:`)
  instead of WebKit. In the defaults that is `ctrl-enter` and `shift-f10` (the page's
  context menu). A key the user gives a page command later is taken the same way.
- **Escape** belongs to the page (a page closes its own dialogs with it). The ways out of
  a page are the window's keys: `cmd-0` (library), `cmd-shift-p`, `cmd-p`, clicks.
- **`cmd-1` (Go to text)** focuses the page element when a page is open.
- **Edit menu and its keys.** `cmd-c` and the others reach the root's captures
  (`edit_where_the_keyboard_is`). With the page element focused they are sent to the
  web view: `copy:`, `cut:`, `paste:`, `selectAll:`, `undo:`, `redo:` (WebKit acts on a
  selection, or on a field inside the page).
- **Vim mode** does nothing on a page: it is attached to the editor, which has no focus.
  Its chip is not shown. The page takes `j` and `k` as the page wishes.
- **Not checked by a person yet** (spike finding d): all of this in a real, active key
  window. The first milestone of WP7 is that check (section 13, check 1; section 14,
  risk 1).

### Overlays: snapshot while open, holes for toasts

GPUI draws everything in one layer; the web view lies above it, so whatever GPUI draws
over the page's bounds is hidden (spike finding c). One approach for each case:

- **A modal or a context menu over the page** (`modal_layer.has_modal()`,
  `ContextMenu::is_open`, a picker): the view takes a snapshot
  (`takeSnapshotWithConfiguration:`, 55 to 90 ms in the spike), and when it arrives the
  page element draws it and the web view is hidden. Until then the web view stays (the
  overlay shows 55 to 90 ms late rather than over a blank area). When the overlay
  closes, the web view is shown again and, in the next prepaint, the snapshot is handed
  to `window.drop_image` and dropped.
- **Memory.** The snapshot comes back as BGRA pixels (`writer_web::Bitmap`, decoded from
  the `NSImage`'s `CGImage` in `writer-web`, no PNG on the way) and becomes a
  `gpui::RenderImage` that the element paints itself (`window.paint_image`), never through
  `img(Arc<Image>)`: GPUI's asset cache keeps every decoded image of `img()` forever (the
  spike's leak, 388 MB to 1162 MB; `docs/gpui-notes.md`, "Images"). A test counts the
  textures dropped against the snapshots made.
- **Toasts** (agent notes, print notes, the page's alerts): the page stays live and a
  hole is cut in the web view's layer for each toast (a `CAShapeLayer` mask with the
  even-odd rule; spike `set_toast_hole`), at the bounds the toast layer drew them
  (`ToastLayer::toast_bounds`, new in `writer-ui`, WP4). No toast: no mask.
- **Tooltips** that reach into the page's bounds are hidden under it. Accepted (the only
  GPUI elements with tooltips near the page are the title and the status bar).
- A failed snapshot: the web view is hidden and the element draws the canvas colour
  with the page's title.

### Scroll, zoom, theme

- Scrolling is WebKit's (trackpad, wheel, keys while focused).
- The app's zoom commands work on a page: `zoom::ZoomIn`, `ZoomOut`, `ActualSize` and
  `ZoomTo50` … `ZoomTo400` set the view's `pageZoom` on the steps of
  `writer_editor::ZOOM_STEPS`, from 50 to 300 %. `FitWidth`, `FitPage`, `LifeSize`,
  `Automatic` say "A page is drawn at its own size: it has no fit." The zoom is not
  stored in v1; a page opens at 100 %. The pinch and `cmd` with the wheel do nothing on
  a page in v1 (`Workspace::pinch` ignores them; the view has `allowsMagnification = NO`).
- Theme: the view's `appearance` follows the app's theme (Paper: `NSAppearanceNameAqua`,
  Night: `NSAppearanceNameDarkAqua`, System: nil), so a page that follows
  `prefers-color-scheme` follows the app. `underPageBackgroundColor` is the theme's
  canvas. A change of theme reaches an open view at once.

### The title and the status bar for a page

- Title: the page's title. The chevron and a right-click open the title menu of a page
  (`MenuKind::Title`, a branch in `workspace/menus.rs`): Rename…, Reload Page, Find in
  Page…, Copy Page Address, Open Page Address in Browser (both only with a source),
  Print…, Export…, Version History…, Reveal in Library.
- Status bar, from the left: the source's host ("claude.ai"; a click or a right-click
  opens Copy Page Address and Open Page Address in Browser), the words of its text
  ("4,212 words", from `word_count`), the zoom ("100 %", its menu with the steps). No save
  state, no Vim chip, no character limit.

### Commands that do not apply to a page

- The commands that act on a text (`text_commands()`, the image commands, spelling's
  word commands) have no handler while a page is open: the menu bar greys them out, as
  with no document. The palette leaves them out while a page is open
  (`PaletteDelegate` is given the commands without those whose action is in that set and
  not available on the focused path, `window.is_action_available`).
- The commands of a document that would do something else on a page say what a page is,
  in one toast (`workspace::page::NOT_FOR_A_PAGE` and its siblings, wording below):

| Command | Toast |
|---|---|
| Document settings, page view on or off, orientation, body text size, line spacing | "A page is shown as it was made: it has no document settings." |
| Spelling language | "A page is not checked for spelling." |
| Save now | "A page is stored as it was added; there is nothing to save." |
| Add comment, next and previous comment, go to comments | "A page has no comments." |
| Ask the assistant (quick chat, `cmd-j`) | "The quick chat works in a text. Ask about this page in the side chat." (with the side chat's key from the keymap) |
| Fit width, fit page, life size, automatic zoom | "A page is drawn at its own size: it has no fit." |

### Find in page

`writer-editor-tools`' `Find`, `FindAndReplace`, `FindNext`, `FindPrevious` get root
handlers while a page is open. Their keys and their menu items stay what they are (the
keymap's one source). `Find` opens a strip at the top of the page area: a
`writer_ui::TextField` and "Not found" when nothing matches. Typing searches as one
types; `enter` and `FindNext` go to the next match, `shift-enter` and `FindPrevious` to the
one before (`findString:withConfiguration:completionHandler:`, wrapping, case
insensitive). The field's `Cancel` (`escape`) closes the strip and gives the keyboard
back to the page. `FindAndReplace` opens the same strip (a page cannot be changed). The
strip is GPUI's and lies above the web view's frame, not over it. A hit of the library
search on a page opens the page with the strip on the query.

### The context menu (product rule 1)

- **Right-click** in the page shows WebKit's own menu (native, so above everything), cut
  down and extended in the web view subclass's `willOpenMenu:withEvent:`. Removed, by
  their `WKMenuItemIdentifier…` constants: `OpenLink`, `OpenLinkInNewWindow`,
  `OpenImageInNewWindow`, `OpenFrameInNewWindow`, `OpenMediaInNewWindow`,
  `DownloadLinkedFile`, `DownloadImage`, `DownloadMedia`, `Reload`, `InspectElement`,
  `ToggleFullScreen`, `ToggleEnhancedFullScreen`, `ShareMenu`. Kept: Copy, Copy Link, Copy
  Image, Look Up, Back, Forward, Speech, Paste in a field, the media controls. Added at
  the end, after a separator: "Open Link in Browser" (on a link to another site), then
  the app's items: Find in Page…, Reload Page, Copy Page Address, Open Page Address in
  Browser, Print…, Export…, Version History…. The items come from
  `PageEvents::menu_items()` and run through `PageEvents::menu_chosen(id)`, which
  dispatches the item's action, so each shows the key the keymap has for it.
- **`ctrl-enter` and `shift-f10`** (`page::OpenContextMenu`, bound in `PageView`) open the
  same app items as a `writer_ui::ContextMenu` at the top left of the page, with Copy and
  Select All first. It is a GPUI overlay: the snapshot applies.
- The library row of a page has the panel's own items (Open, Rename, Move…, Duplicate,
  Move to Trash) and the app's (Export…, Version History…, and for a page with a source
  Copy Page Address, Open Page Address in Browser).

### A crashed WebContent process

`webViewWebContentProcessDidTerminate:` (spike `did_terminate`) calls
`PageEvents::crashed`. The first time the view reloads by itself. Again within 10 s: the
element shows "This page stopped working." with a "Reload Page" button (and the command),
and the web view is hidden until then.

### The test seam: the page engine

`crates/writer-app/src/pages.rs`, an app global like `printing::PrintTarget` and the spell
service:

```rust
pub enum PageEngine {
    /// The real view and check (`writer-web`). `main` installs it in every run of the
    /// real binary, scripted or not, unless `THE_WRITER_PAGES=off`.
    WebKit,
    /// Tests: a GPUI element that draws the page's text, and a check that answers
    /// from the HTML (`writer_page::static_check`). Records what was asked.
    Fake(Rc<FakePages>),
}
pub fn use_webkit(cx: &mut App);
pub fn use_fake(cx: &mut App) -> Rc<FakePages>;     // a test, before it opens a workspace
pub fn engine(cx: &App) -> Option<PageEngine>;      // none: a page shows its text in a quiet column and says "Pages are not shown in this run."

pub struct FakePages { /* RefCell'd */ }
impl FakePages {
    pub fn opened(&self) -> Vec<(DocumentId, String)>;   // (page, entry) in order
    pub fn finish_loading(&self);                         // the open view reports "loaded"
    pub fn crash(&self);                                  // the open view reports a crash
    pub fn zoom(&self) -> Option<u16>;
    pub fn finds(&self) -> Vec<(String, bool)>;           // (query, backwards)
    pub fn snapshots_made(&self) -> usize;
    pub fn snapshots_dropped(&self) -> usize;
    pub fn first_responder_is_page(&self) -> bool;
    pub fn edits(&self) -> Vec<&'static str>;             // "copy", "select_all", ...
    pub fn set_check(&self, report: writer_page::CheckReport); // the next check's answer
    pub fn press(&self, key: &str);                       // a key the view would hand on (takes_key)
}
```

Default test runs never make a `WKWebView`. Real WebKit is tested in `writer-web`'s own
program (WP3) and in two end-to-end cases of `writer-cli` (WP8).

### Scripted runs

Steps (grammar in `automation.rs`):

| Step | What it does |
|---|---|
| `page:make:<folder>[\|<entry>]` | stores a folder that is offline already as a page, with no fetching and no WebKit check (QA only; WP7) |
| `page:import:<path>` | imports a file or folder as File > Import Page… does once it is chosen (WP9); logs `page: added page:12 “Orbitarium” 31 files 1.0 MB` or the sentence of the refusal |
| `page:wait[:<ms>]` | waits until the open page reports loaded (fails after `ms`, 10 s unless given) |
| `page:text` | logs the open page's `innerText`, its length first |
| `page:find:<query>` | opens the find strip with that query |
| `page:js:<script>` | evaluates a script in the open page and logs the answer (scripted runs only) |
| `page:menu` | opens the page's context menu as `ctrl-enter` does |

`state` gets `page=[id=12 state=loaded zoom=100% overlay=none|waiting|snapshot|holes:2 focus=page|elsewhere responder=web|gpui find="harbour":found missing=0]`
while a page is open, and `page=[none]` otherwise.

---

## 7. Print and export of a page

### Print

- "Print…" (the existing `workspace::Print`, its menus, `:print` is Vim's and does not
  apply) on a page: the view makes the PDF with WebKit's print operation as a save job
  into a temporary file (`printOperationWithPrintInfo:`, both panels off, run with
  `runOperationModalForWindow:…`, which the spike found is the call that draws), on the
  paper of the user's default print info (`NSPrintInfo.sharedPrintInfo`, A4 on this Mac),
  in print colours (the spike: A4 pages). The bytes become a
  `writer_print::Pdf::from_bytes(bytes, pages)` (new, WP4), the job a
  `printing::PrintJob { title, paper, pdf }`, and it goes to the installed
  `PrintTarget` as a text's job does: the print panel as a sheet (`SystemPanel`), the
  test's `Recorder`, a scripted run's `Directory` (`<title>.pdf` in
  `THE_WRITER_EXPORT_DIR`). So a page prints through the same panel, with the same
  "Save as PDF", and tests record its job.
- No slices: WebKit paginates in its own process; the job is "preparing" with the same
  note and Cancel until the PDF is there.
- The fake engine answers with a fixed one-page PDF, so the Recorder has a job.
- `print:pdf:<path>` on a page writes that PDF and logs `print: 3 pages, 595.28 x 841.89 pt (a page, WebKit), made in 0.21 s`.
- `window.print()` inside a page does nothing in v1.

### Export

- `Export…` (`cmd-shift-s`, the palette, the menus, the row) on a page opens the export
  modal with two rows instead of Markdown, DOCX and text: "Web page folder (opens
  offline)" and "Original files (as they were added)". The save panel suggests a folder
  named after the title. The folder gets the served files (or the original files) at
  their paths. A folder that exists gets the files written into it (same names
  replaced). The toast names the folder (`export::short_path`) and counts the files.
- Writing a folder from a manifest is `writer_page::write_folder` (WP2), used by the app
  and by `writer export`: no store in it, the caller hands a function that gives the
  bytes of a file id.
- An exported folder opens in a browser from the Finder for most pages. ES modules and a
  page's `fetch` of its own files do not work from `file://` in Safari (the archive's
  note says so); the export toast says "Open index.html in a browser; a few pages need to
  be served (python3 -m http.server)."
- No single-file HTML in v1 (inlining fonts, modules and media as `data:` addresses
  breaks module graphs and makes files of hundreds of megabytes).
- A folder export of the library (Markdown, DOCX, text) leaves pages out and says so:
  "2 pages were left out: a page is exported on its own, as a folder." (the app's
  `export.rs` and `writer-agent`'s `export.rs`).
- The version history of a page: each version shows its date, the label, "N files" and
  its text (the manifest's `text`) in the preview, muted. Restore keeps the current page
  as a version first, then makes that manifest current, and the view reloads.

---

## 8. Search, agents' reads, the assistant

- **Where the text lives:** `documents.plain_text` (and the manifest's `text`). Written
  when the page is added or updated, never again: a page does not change.
- **Full-text search:** `documents_fts` follows `plain_text` by its triggers; pages are
  found by `Store::search` like documents. `SearchHit` gains `kind`. The library search
  shows a page's hits with the page's icon; opening one opens the page with the find
  strip on the query. Quick open lists pages, marked "Page".
- **`writer get <page>`**: the text as the page showed it (formats `markdown` and `text`
  give the same text; a page has no Markdown). `--format json` gives the manifest without
  the text. The result (`DocumentText`) says `kind: "page"` and has `page:
  Some(PageSummary)` (source, entry, files, bytes, externals, added and checked).
- **MCP reads:** `read_document` on a page answers as `writer get`; `list_library`,
  `search_library`, `recent_documents`, `document_info` carry `kind` and `source_url`;
  `find_in_document` finds in the text; `list_versions` and `read_version` work (a
  version's text is the manifest's `text`). `list_comments` of a page is empty.
- **The side chat**, with a page open: the open item is sent as it is for a text, with
  its text in place of the Markdown and one sentence before it in the system prompt:
  "The open item is a page: a web page saved in the library, shown as it was made. It
  cannot be changed. Its text, as the page showed it:". `writer_assistant::OpenDocument`
  gains `kind: OpenKind` (`Text`, `Page { source_url }`; WP4). The answer buttons that
  change the text (Insert at Caret, Replace Selection) are off, as without a caret
  (`follow_the_text` says there is no caret and no selection); Copy works. The
  conversation is per page, as per document (`assistant.conversation.<id>`).
- **The quick chat** does not open on a page in v1 (toast above).

---

## 9. Agents

### Command line

```
writer page add <file|folder>... [--entry <path>] [--title <title>] [--source-url <address>]
                [--folder <folder>] [--create-folders] [--author <name>] [--dry-run] [--json]
writer page add --list <file.json|->  [--folder …] [--create-folders] [--author …] [--dry-run] [--json]
writer pages [<folder>] [--json]
writer get <page> [--format text|json]
writer export <page> <directory> [--original] [--overwrite]
writer __page-check <dir>            (not in the help; the child of the check)
```

- `--entry`, `--title` and `--source-url` need exactly one input; with several the command
  is refused (exit 2) and the hint says to use `--list`.
- `--list` reads `[{"path": "...", "entry": "...", "title": "...", "source_url": "..."}]`
  (only `path` required) from a file or standard input: several pages, each with its own
  address. This is what an agent uses to copy many artifacts in one command.
- Each input is one operation (`Library::add_page`) and one line of output, in order:

```
added page:41 “Reading Orbitarium” in Claude Pages (30 files, 11.4 MB, 12 pieces from the web)
unchanged page:12 “Orbitarium” (https://claude.ai/artifact/ExampleArtifactId00002)
updated page:13 “Nimbus Release Notes”: the page as it was is kept as version 88
not added “Nimbus Planning Doc”: it could not be kept whole. The page asks for https://claude.ai/api/… at run time (data it fetches), which cannot be kept. Nothing was saved.
```

- Exit code: 0 when every page was added, updated or unchanged; otherwise the code of
  the first page that failed (the others are still tried and printed). `--json` prints
  `{"pages": [<PageAdded or error report>, …]}`.
- While the app is open the tool prepares the page itself (reading, fetching, the check)
  and hands it to the app, which stores it; with the owner's switch off the change is
  refused with exit code 5, as every change is.
- A page's reference is `page:<id>` in every output. `doc:<id>` and `page:<id>` are both
  accepted wherever a document is (`Target::parse` reads `page:` as `DocumentId`).
- `writer pages`: `page:41  Claude Pages/Reading Orbitarium  https://claude.ai/artifact/ExampleArtifactId00001  updated 2026-11-13 21:04  30 files, 11.4 MB`.
  The skill uses it to see what is in already.
- `writer export <page> <dir>`: the served files as a folder (`--original`: the input as
  added). A text format for a page (`.md`, `.docx`) is refused (exit 15) and `.txt` writes
  its text.
- Other commands on a page: `info`, `history list|show|restore`, `find`, `rename`, `move`,
  `duplicate`, `trash`, `restore`, `delete --permanently` work. `edit`, `append`,
  `insert`, `replace`, `replace-section`, `image add`, `comment add|reply|resolve|…` fail
  with exit 15 (`wrong_kind`). `comment list` lists nothing.

### Error codes and wording

Two new kinds in `writer_agent::ErrorKind` (in the order of their codes):

| Code | Exit | When | Message (written by `writer-agent` for both front ends) |
|---|---|---|---|
| `page_incomplete` | 14 | the page cannot be kept whole (any reason of section 3) | "“Reading Orbitarium” was not added: it could not be kept whole. <up to five reasons>; and 3 more. Nothing was saved." |
| `wrong_kind` | 15 | a text operation on a page, or a page operation on a text | "“Orbitarium” is a page: it is kept as it was saved, and its text cannot be changed. To bring a newer copy, add it again with its source address." / "“Chapter 2” is a document, not a page." |

Who adds what (CLAUDE.md, "Who says what"): the command line adds the option to type
next (`--entry` for a folder with several pages, `--list` for several pages with their own
addresses, `writer page add <folder> --source-url <address>` after `wrong_kind`); the MCP
server names the tool (`add_page` with its `source_url`). A failure to fetch says the
address and the reason curl gave. The checker failing (no window server, WebKit gone) is
`failed` (1): "the page could not be checked: <reason>".

`writer-agent-setup/tests/skill_against_binary.rs` fails until the skill explains 14 and
15 (CLAUDE.md, "The skill's list of exit codes is tested against the help").

### MCP

- New tool **`add_page`** (title "Add pages", an "add" in the table of `tools.rs`, not
  read-only, not idempotent):

```json
{
  "pages": [{"path": "/abs/folder", "entry": "index.html", "title": "…", "source_url": "https://claude.ai/artifact/…"}],
  "folder": "Claude Pages",
  "create_folders": true,
  "author": "Claude",
  "dry_run": false
}
```

  At most 10 pages per call (the description says why: each is checked in WebKit; for more,
  call again or use the command line's `--list`). Paths must be absolute (the server has
  no folder of its own). The answer lists each page's outcome; the tool fails only when
  no page could be added.
- `export` takes a page with `format: "html"` (a folder) and `original: true|false`.
- Hidden for the command line: `list_pages` (a `hidden_read`, like `list_images`), behind
  `writer pages`. `list_library` already gives kinds and addresses to a model.
- Counts: **37 tools, 12 of them read-only** (36 and 12 today; CLAUDE.md's row of
  `writer-mcp` and the server's tests change). The assistant's set of read-only tools is
  unchanged (it reads pages through `read_document`).

### The operations table and the channel

- One row in `crates/writer-agent/src/operations.rs`:
  `add_page / AddPage(AddPageRequest) -> PageAdded = pages::add_page;`. The macro's
  `Prepare::prepared` call is where the input is read, fetched and checked
  (`impl Prepare for AddPageRequest` in `pages.rs`, beside `attach.rs`'s images), so the
  app never reads an agent's path and never fetches for an agent.
- `Library::pages(&PagesRequest) -> Result<PagesList>` is a reader (the test of the table
  that knows the readers gets it).
- `pages::add_page(session, request)`: refuses a request without `prepared` ("the page was
  not prepared"); re-checks the limits; stores the blobs (or checks the stored ones);
  builds the manifest (`writer_page::Prepared::manifest`); finds the page by
  `source_url`; adds, updates (version first) or confirms unchanged; notes
  `Change::DocumentCreated` or `Change::DocumentText` (+ `DocumentRenamed` when the
  title changed). The dry run writes nothing (the connection is `query_only`, as for
  every dry run).
- The note in the app: "Claude added “Orbitarium”", "Claude updated “Orbitarium”"
  (`workspace::sentence` gets the words of `add_page`). Its button is Open for a page
  that is not open; an open page that was updated reloads and the note has no button.
- `writer-agent`'s suite runs a second time through a stand-in app
  (`WRITER_AGENT_TESTS_THROUGH_THE_APP`): `add_page` is run both ways and the two
  libraries are compared row by row, `images.db` included. `writer-cli` runs `page add`
  against a stand-in (`LiveApp`), as it runs every command that changes the library.

### The skill (`crates/writer-agent-setup/src/skill_body.md`)

A paragraph "Pages" after "Images", and the recipe. The text, to be held by its test:

> Pages. A page is a web page kept in the library as it was made, with its styles,
> scripts, fonts, pictures and sounds, so that it shows the same offline and for good.
> Add one from a file or a folder on this Mac: `writer page add ~/Downloads/explainer
> --source-url https://claude.ai/artifact/… --folder "Claude Pages" --create-folders`.
> A folder is taken whole and opens with its `index.html` (`--entry` names another file).
> Everything the page loads from the web (fonts, libraries) is downloaded now and stored;
> a page that needs something that cannot be stored (data from a service, an embedded
> site) is not added, exit code 14 says why, and nothing is saved. With `--source-url`,
> adding the same address again changes nothing when the page is the same and updates it
> (keeping the old one as a version) when it changed. `writer pages` lists the pages with
> their addresses. `writer get page:12` gives the text the page shows. A page cannot be
> edited (exit code 15).
>
> To copy the user's claude.ai artifacts into The Writer (Claude Code): list them with the
> Artifact tool (`action: "list"`, `scope: "mine"`, `limit: 200`); run `writer pages
> --json` and skip every artifact whose address is there with an `updated` time later
> than the artifact's last update. For each other one: list its files (`action: "list"`,
> `scope: "files"`, its `url`) and read them all into one new folder (`action: "read"`, its
> `url`, `paths` with every published path, `out_dir` a new empty folder); a single-page
> artifact is its page alone (`action: "read"` names the saved file). Write a list
> `[{"path": "<folder>", "source_url": "<url>", "title": "<title>"}, …]` and run `writer
> page add --list <file> --folder "Claude Pages" --create-folders --author Claude`. Report
> what was added, updated, unchanged and not added, with the reasons. An artifact made from
> a type (a Claude Docs document, a deck) shows its content by connecting to claude.ai:
> such a page is not added; export its content as HTML first and add that. A page that
> uses claude.ai's own storage (`window.claude`) is added, and its saved data stays on
> claude.ai.

---

## 10. Keyboard and mouse (product rules 1 to 3)

### Commands

| Command (palette, category "Pages") | Key | Menu bar | Context menus | Available |
|---|---|---|---|---|
| Import page… (`workspace::ImportPage`) | none | File > Import Page… (after New Folder) | the library's empty space and folder rows: "Import Page…" | always |
| Reload page (`workspace::ReloadPage`) | none | View > Reload Page | page (both menus), title | a page is open |
| Copy page address (`workspace::CopyPageAddress`) | none | File > Copy Page Address | page, title, status host item, library row | a page with a source |
| Open page address in browser (`workspace::OpenPageAddress`) | none | File > Open Page Address in Browser | the same | a page with a source |
| Show the page's context menu (`page::OpenContextMenu`) | `ctrl-enter`, `shift-f10` (bound in `PageView`) | (no item: a key) | | a page is focused |

The existing commands that work on a page: Find (`cmd-f`), Find and replace (`cmd-opt-f`,
as Find), Find next and previous (`cmd-g`, `cmd-shift-g`), Print, Cancel printing, Export
(`cmd-shift-s`), Version history, Rename document, Reveal in library, Zoom in, out, actual
size and the steps, Go to text (`cmd-1`), Copy, Select all. No new default key: a page is
read, not worked in, and the keys a reader needs are WebKit's own.

Copying the address puts text on the clipboard: scripted runs must not run it (the rule
of CLAUDE.md for every copy).

### Mouse

Everything above has a pointer way: the title's chevron and right-click (the title menu
of a page), the status bar's host and zoom items, right-click in the page (WebKit's menu
with ours), the library row's menu, File > Import Page…, and drops:

- `.html` / `.htm` files and folders dropped on the **sidebar** are imported (into the
  folder under the pointer, else the selected folder; `LibraryPanelEvent::DropPaths`,
  WP6).
- Dropped on the **page area**: the web view does not take drops
  (`unregisterDraggedTypes`), so GPUI's view gets them and the page element's
  `on_drop::<ExternalPaths>` imports them.
- Dropped on the **text area** of an open document: the editor hands dropped files to its
  provider (`Editor::drop_files`); `images::LibraryImages` gives `.html`, `.htm` and
  folders to the workspace's import (a hook it is given) and takes the rest as pictures.
- Other files among them are left out, with a toast that counts them.

### The list of keyboard shortcuts

- New key context `PageView`: a line in `keymap::places` (it lies inside `Workspace`;
  text is not typed there) and the place name **"On a page"** in
  `shortcuts::Place::of_predicate`. The namespace `page` goes into `KEY_GROUPS`, and
  `page::OpenContextMenu` gets its title in `KEY_TITLES` ("Show the page's context
  menu"), as the other context-menu keys have.
- New rows: Import page… (Anywhere), Reload page, Copy page address, Open page address in
  browser (Anywhere: they are window commands), and Show the page's context menu (On a
  page): **5 rows, 5 entries**. With every part shown the list has **377 rows** (372
  today) and the shortcuts file **402 entries** (397); without the assistant **347
  rows**, the same 402 entries. The builder counts again in a scripted run
  (`keys:cmd-/; state` and "Export…") and writes the real numbers into CLAUDE.md.
- `every_key_of_the_keymap_is_a_row_and_an_entry_of_the_file` and
  `two_rows_under_one_place_name_are_in_the_same_place` must hold with the new context.

---

## 11. Scope

### In v1

- A page as a library item (schema 4), its blobs in `images.db` (schema 2), the backups,
  the damaged-file handling and the collector covering them.
- The bundler: a file or a folder in, every reference found, externals fetched once,
  rewriting, limits, the whole-or-nothing check, the text, re-adding by address.
- WebKit: the check (in the tool's child process and in the app), the view (one at a
  time), the isolation of section 5.
- Agents: `writer page add` (with `--list`), `writer pages`, `get`, `export`, `info`,
  versions and the item commands on pages; MCP `add_page` and the reads; the operations
  table; protocol 3 with the blobs stored off the main thread; exit codes 14 and 15; the
  skill and its recipe.
- The app: opening a page, focus and keys, overlays, title and status bar, find, zoom
  (not stored), theme, a crashed process, the context menus, the commands of section 10,
  File > Import Page… and drops, print, export as a folder, the version history of a
  page, the side chat's context, scripted steps and `state`, the fake engine for tests.

### Not in v1 (deferred)

- A page added from an address (fetching a live site).
- Single-file HTML export; Markdown, DOCX or text export of a page beyond its text.
- A page's storage kept across launches (question 2); a page's own downloads (question
  8); its `<input type=file>`; `window.print()` from inside a page.
- The quick chat on a page; pinch and `cmd`-wheel zoom on a page; a zoom stored per page;
  the scroll position kept per page.
- More than one live web view (a cache for switching).
- Editing a page, comments on a page, annotations.
- Re-importing a file in the app to update the page it made (the app gives no address).
- External iframes and runtime data (refused, question 7); letting a page in with what
  is missing.
- Finding across every HTML file of a page with several (find works in the one shown).
- Requests made inside workers reported by the check (they are blocked; section 3).
- An Intel Mac (not run; nothing in the design is arm64 only).

---

## 12. Work packages

Three waves. In a wave every package builds against the interfaces written here, not
against each other's commits. One owner per crate per wave.

| Id | Title | Crates | Wave | Depends on | Effort |
|---|---|---|---|---|---|
| WP1 | Store: pages and their files | writer-store | 1 | none | xhigh |
| WP2 | Bundler | writer-page (new) | 1 | none | xhigh |
| WP3 | WebKit | writer-web (new) | 1 | none | xhigh |
| WP4 | Small additive changes | writer-ui, writer-print | 1 | none | medium |
| WP5 | Agents' command layer | writer-agent | 2 | WP1, WP2 | xhigh |
| WP6 | Library model and sidebar | writer-library | 2 | WP1, WP4 | medium |
| WP7 | The page view in the app | writer-app | 2 | WP1, WP2, WP3, WP4 | xhigh |
| WP8 | Tool, MCP server, skill | writer-cli, writer-mcp, writer-agent-setup | 3 | WP2, WP3, WP5 | high |
| WP9 | Import, agents and the assistant in the app; docs | writer-app, writer-assistant, CLAUDE.md, docs/gpui-notes.md, scripts/bundle.sh | 3 | WP5, WP6, WP7 | xhigh |

### Rules for every package

- The root `Cargo.toml`: WP2 adds `writer-page = { path = "crates/writer-page" }` and WP3
  `writer-web = { path = "crates/writer-web" }` to `[workspace.dependencies]`, each in its
  first commit. A `git commit -- Cargo.toml` that takes the other line too is fine: both
  are wanted. `Cargo.lock` is committed by whoever's build changed it (one checkout: it
  holds everyone's additions).
- Target directories: `writer-page` and `writer-web` build and test in `target-pure`
  (like `writer-image`, which also calls system frameworks): `CARGO_TARGET_DIR=target-pure
  cargo test -p writer-page`, `… -p writer-web`. They compile again in `target/` when the
  app links them: both are small (no new third-party crate that is not in `Cargo.lock`
  already: `objc` 0.2, `block` 0.1, `dispatch` 0.2, `core-foundation` 0.10, `sha2` 0.10,
  `base64` 0.22, `url` 2.5, `percent-encoding` 2.3, `regex` 1, `serde`, `serde_json`).
  CLAUDE.md's working rules list both among the crates that use `target-pure`.
- Test binaries: `writer-page` one integration binary (`tests/main.rs`); `writer-web` one
  program with `harness = false` (`tests/webkit.rs`: WebKit wants the main thread, as
  `writer-print`'s `tests/save_job.rs` does) plus unit tests; the app adds
  `tests/suite/pages.rs` to its one binary. No new example.
- No test reaches the internet. A test that fetches runs a server on 127.0.0.1 in a
  thread (`std::net::TcpListener`); every other test uses `writer_page::MapFetcher`.
- No test of a GPUI crate makes a `WKWebView`. The app's tests install the fake engine.
- The archive at `<page archive>/` is read only. A test that reads
  it is `#[ignore]` and copies nothing there.
- Commit only your paths; the two lines of attribution end every message.

### WP1. Store: pages and their files (writer-store)

Owns `crates/writer-store`. Interface (everything else of the crate stays as it is):

```rust
// lib.rs re-exports, beside the existing ones
pub use crate::documents::{DocumentKind, PageContent};
pub use crate::images::{FileId, FileInfo};

// documents.rs
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DocumentKind {
    #[default]
    Text,
    Page,
}
impl DocumentKind {
    pub fn as_str(self) -> &'static str;              // "text" | "page", the column's values
    pub fn parse(text: &str) -> Option<DocumentKind>;
}

pub struct DocumentMeta {
    // ... every existing field, unchanged ...
    /// What the item is. `Text` for every row written before schema 4.
    #[serde(default)]
    pub kind: DocumentKind,
    /// The address a page came from; `None` for a text and for a page without one.
    #[serde(default)]
    pub source_url: Option<String>,
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct PageContent<'a> {
    pub manifest: &'a str,        // writer_page::Manifest as JSON, stored in `content`
    pub title: &'a str,
    pub plain_text: &'a str,      // the rendered text
    pub word_count: u64,
    pub char_count: u64,
    pub source_url: Option<&'a str>,
}

impl Store {
    /// A page in `folder` (or at the root), after its sibling documents. NotFound for a
    /// missing or trashed folder.
    pub fn create_page(&mut self, folder: Option<FolderId>, page: &PageContent<'_>) -> Result<DocumentMeta>;
    /// New content of a page (manifest, text, counts, title, source); bumps `updated_at`.
    /// WrongKind { kind: Text } on a text. Works on a trashed page, as save_document does.
    pub fn save_page(&mut self, id: DocumentId, page: &PageContent<'_>) -> Result<DocumentMeta>;
    /// The live page with this address that was updated last; None when there is none.
    pub fn page_by_source_url(&self, url: &str) -> Result<Option<DocumentMeta>>;
    /// Every live page, most recently updated first.
    pub fn pages(&self) -> Result<Vec<DocumentMeta>>;

    /// Stores the bytes once (FileId::of); clears the collector's stamp. Committed before
    /// it returns. Creates images.db when it is not there.
    pub fn put_file(&mut self, bytes: &[u8]) -> Result<FileInfo>;
    pub fn touch_file(&mut self, id: &FileId) -> Result<Option<FileInfo>>;
    pub fn file_info(&self, id: &FileId) -> Result<Option<FileInfo>>;
    /// From images.db, else from the copy among the backups (written back), as image_bytes.
    pub fn file_bytes(&self, id: &FileId) -> Result<Option<Vec<u8>>>;
    pub fn file_count(&self) -> Result<u64>;
    /// Every "file":"<32 hex>" in the content of documents (live and trashed) and snapshots.
    pub fn referenced_files(&self) -> Result<HashSet<FileId>>;
}

impl ImageFile {   // the same three for a connection of its own (the app's background tasks, the listener)
    pub fn put_file(&mut self, bytes: &[u8]) -> Result<FileInfo>;
    pub fn file_info(&self, id: &FileId) -> Result<Option<FileInfo>>;
    pub fn file_bytes(&self, id: &FileId) -> Result<Option<Vec<u8>>>;
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct FileId(pub [u8; 16]);
impl FileId { pub fn of(bytes: &[u8]) -> FileId; pub fn parse(text: &str) -> Option<FileId>; pub fn to_hex(&self) -> String; }
impl fmt::Display for FileId;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileInfo { pub id: FileId, pub byte_len: u64, pub created_at: i64 }

pub enum StoreError { /* ... */ WrongKind { id: i64, kind: DocumentKind } }  // `kind`: what the row is
pub struct SearchHit { /* ... */ pub kind: DocumentKind }
pub struct ImagesCollected { /* ... */ pub files: u64, pub files_referenced: u64, pub files_newly_unreferenced: u64, pub files_deleted: u64, pub file_bytes_freed: u64 }
pub struct ImagesBackedUp { /* ... */ pub files: u64, pub files_copied: u64, pub file_bytes_copied: u64, pub files_waiting: u64 }
```

Behaviour: schema 4 (section 1); `save_document` refuses a page (`WrongKind { kind: Page }`);
`duplicate_document` copies `kind` and leaves `source_url` out; `images.db` schema 2 with
`files` and its migration; `scan.rs` reads `"file"` too; the collector, the copy,
`salvage`, `read_from_copy`, `named_by_kept_backups` and `holds_collected` cover
`files` (section 2). The table at the top of `collect.rs` gets its row.

Tests: `tests/pages.rs` (create, save, find by address, the kind guards both ways,
duplicate, trash and restore, search finds the text, snapshots of a page); in
`tests/images.rs` the files half of every collector, copy and damage test that matters
(a file named only by a version survives; a file of a deleted page goes after 30 days
and the oldest backup; a damaged `images.db` keeps both tables' rows; bytes that are not
their id are taken from the copy; a version-1 `images.db` is migrated by a writer and
read as empty by a reader); schema tests named in section 1; the covering index test.

Done: `CARGO_TARGET_DIR=target-pure cargo test -p writer-store` passes;
`cargo build -p writer-app` and `CARGO_TARGET_DIR=target-pure cargo build -p writer-cli`
still compile (a new variant of `StoreError` and new fields are additive; an exhaustive
`match` elsewhere gets the smallest fix).

### WP2. Bundler (writer-page, new)

Owns `crates/writer-page`. Pure Rust, no Objective-C. Interface:

```rust
pub mod limits {
    pub const MAX_FILE_BYTES: u64 = 32 * 1024 * 1024;
    pub const MAX_PAGE_BYTES: u64 = 64 * 1024 * 1024;     // distinct bytes, served + original
    pub const MAX_FILES: usize = 2_000;
    pub const MAX_EXTERNALS: usize = 500;
    pub const MAX_TEXT_CHARS: usize = 2_000_000;
    pub const FETCH_TIMEOUT: Duration = Duration::from_secs(30);
    pub const FETCH_BUDGET: Duration = Duration::from_secs(180);
    pub const FETCH_PARALLEL: usize = 6;
    pub const MAX_DEPTH: usize = 8;
    pub const ALSO_LOADS: usize = 40;
    pub const SETTLE_MIN_MS: u64 = 1_000;
    pub const SETTLE_MAX_MS: u64 = 3_000;
    pub const CHECK_LIMIT_MS: u64 = 60_000;
}
pub const EXTERNAL_DIR: &str = "_external";
pub const MANIFEST_VERSION: u32 = 1;

/// First 16 bytes of the SHA-256, 32 lowercase hex digits: writer_store::FileId::of(bytes).to_hex().
pub fn file_id_hex(bytes: &[u8]) -> String;
/// The media type served for a path, by its ending ("text/html", "font/woff2", "audio/mpeg", ...).
pub fn media_type(path: &str) -> &'static str;

// ----- input
pub struct Input { pub name: String, pub root: PathBuf, pub entry: String, pub files: Vec<InputFile>, pub fingerprint: String }
pub struct InputFile { pub path: String, pub bytes: Arc<[u8]> }
pub fn read_input(path: &Path, entry: Option<&str>) -> Result<Input, PageError>;

// ----- fetching
pub trait Fetcher: Send + Sync {
    /// GET `url` (absolute http or https). Ok only for status 200.
    fn fetch(&self, url: &str) -> Result<Fetched, FetchError>;
}
#[derive(Clone, Debug)]
pub struct Fetched { pub final_url: String, pub media_type: Option<String>, pub bytes: Vec<u8> }
#[derive(Clone, Debug)]
pub struct FetchError { pub url: String, pub reason: String }     // "HTTP 404", "no network: Could not resolve host"
pub struct CurlFetcher { /* program, user agent */ }
impl CurlFetcher { pub fn new() -> CurlFetcher; pub fn with_program(program: PathBuf) -> CurlFetcher; }
pub struct CachingFetcher { /* Arc<dyn Fetcher>, Mutex<HashMap<String, Result<Fetched, FetchError>>> */ }
impl CachingFetcher { pub fn new(inner: Arc<dyn Fetcher>) -> CachingFetcher; }
pub struct MapFetcher(pub HashMap<String, Fetched>);                // tests of every crate

// ----- the bundle
pub struct Bundle {
    pub name: String,
    pub entry: String,
    pub fingerprint: String,
    pub served: Vec<BundleFile>,
    pub original: Vec<BundleFile>,
    pub externals: Vec<External>,
    pub warnings: Vec<String>,
}
#[derive(Clone)]
pub struct BundleFile { pub path: String, pub media_type: String, pub bytes: Arc<[u8]> }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct External { pub url: String, pub path: String }
pub fn bundle(input: Input, fetcher: &dyn Fetcher) -> Result<Bundle, PageError>;
impl Bundle { pub fn html_paths(&self) -> Vec<String>; pub fn distinct_bytes(&self) -> u64; }

// ----- the check as data (the JSON of `writer __page-check`, and what the app maps to)
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct CheckRequest {
    pub entry: String,
    pub also: Vec<String>,                          // other HTML files, at most ALSO_LOADS
    pub media_types: BTreeMap<String, String>,      // path -> media type
    pub settle_min_ms: u64, pub settle_max_ms: u64, pub limit_ms: u64,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct CheckReport { pub engine: String, pub elapsed_ms: u64, pub loads: Vec<LoadReport>, pub failure: Option<String> }
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
pub struct LoadReport {
    pub path: String, pub title: String, pub first_heading: String, pub text: String,
    pub finished: bool, pub navigation_error: Option<String>, pub crashed: bool,
    pub missing: Vec<String>,          // paths answered 404
    pub blocked: Vec<Blocked>,
    pub script_errors: Vec<String>,    // at most 20
    pub requests: usize,               // answered by the scheme handler
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocked { pub url: String, pub directive: String }   // "font-src", "connect-src", ..., "navigation"
pub fn check_request(bundle: &Bundle) -> CheckRequest;
/// <dir>/request.json and <dir>/files/<path>.
pub fn write_check_dir(bundle: &Bundle, dir: &Path) -> io::Result<()>;
pub fn read_check_dir(dir: &Path) -> io::Result<(CheckRequest, BTreeMap<String, (String, Vec<u8>)>)>;
/// The fake check: the entry's text with its tags taken out, its <title>, nothing blocked.
pub fn static_check(bundle: &Bundle) -> CheckReport;

// ----- the verdict: a page that can be stored, or why not
pub fn prepare(bundle: Bundle, report: &CheckReport) -> Result<Prepared, PageError>;

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Prepared {
    pub name: String,
    pub fingerprint: String,
    pub title_found: String,               // "" when the page gives none
    pub text: String,
    pub entry: String,
    pub blobs: Vec<Blob>,                  // distinct bytes
    pub served: Vec<PathEntry>,
    pub original: Vec<PathEntry>,
    pub externals: Vec<External>,
    pub checked: CheckSummary,
    pub warnings: Vec<String>,
}
#[derive(Clone, PartialEq, Serialize, Deserialize)]      // Debug prints sizes, not bytes
pub struct Blob {
    pub file: String,                      // file_id_hex of the bytes
    pub size: u64,
    #[serde(with = "base64_bytes")]
    pub data: Vec<u8>,                     // empty once stored
    #[serde(default)]
    pub stored: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PathEntry { pub path: String, pub media_type: String, pub blob: usize }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CheckSummary { pub engine: String, pub requests: usize, pub elapsed_ms: u64 }
impl Prepared {
    pub fn distinct_bytes(&self) -> u64;
    /// The title rule's middle steps: title_found, else name.
    pub fn title(&self) -> String;
    pub fn manifest(&self, source_url: Option<&str>, now_ms: i64) -> Manifest;
}

// ----- the manifest: the content of a page row
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Manifest {
    pub page: u32,
    pub entry: String,
    pub files: Vec<ManifestFile>,
    pub original: Vec<ManifestFile>,
    pub externals: Vec<External>,
    pub source_url: Option<String>,
    pub fingerprint: String,
    pub added_at: i64,
    pub checked: Checked,
    pub text: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestFile { pub path: String, pub file: String, #[serde(rename = "type")] pub media_type: String, pub size: u64 }
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Checked { pub engine: String, pub at: i64, pub requests: usize, pub warnings: Vec<String> }
impl Manifest {
    pub fn from_json(text: &str) -> Result<Manifest, PageError>;
    pub fn to_json(&self) -> String;
    /// The served file at a path as a URL names it: exact, then percent-decoded.
    pub fn served(&self, path: &str) -> Option<&ManifestFile>;
    pub fn file_ids(&self) -> BTreeSet<String>;
    pub fn bytes(&self) -> u64;
}

// ----- export
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Which { Served, Original }
pub struct Written { pub dir: PathBuf, pub files: usize, pub bytes: u64, pub missing: Vec<String> }
pub fn write_folder(
    manifest: &Manifest, which: Which, dir: &Path, overwrite: bool,
    bytes_of: &mut dyn FnMut(&str) -> io::Result<Option<Vec<u8>>>,   // file id hex -> bytes
) -> Result<Written, PageError>;

// ----- errors
#[derive(Debug)]
pub enum PageError {
    /// The input cannot be used as asked (no such file, a folder with several pages and no entry, a reference outside the root, an address given as input).
    Input { message: String },
    /// Cannot be kept whole: each problem names a piece and why.
    Incomplete { name: String, problems: Vec<Problem> },
    /// The checker itself failed.
    CheckFailed { reason: String },
    /// Stored content that is no manifest.
    Unreadable { reason: String },
    Io { path: PathBuf, error: io::Error },
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Problem { pub what: String, pub reason: String }
impl PageError { pub fn sentence(&self) -> String; }   // the words of sections 3 and 9
```

Over the limits is `Incomplete` with a problem that says which limit. The sentences are
the ones of sections 3 and 9; `writer-agent` wraps them, it does not reword them.

Tests (`tests/main.rs`, submodules): the tokenizer on every construct of the table in
section 3, written as text in the test; rewriting with relative paths from nested files;
`srcset`; `@import` chains; an ES module graph with absolute-path imports on a fake CDN
(`MapFetcher`); import maps with scopes; `<base>`; preconnect removed; an SVG with
`xlink:href`; the string-literal rule (an audio path in a JS table is taken); the limits;
the fingerprint stable and sensitive; `prepare` for each reason of refusal and for script
errors as warnings; the manifest round trip; `write_folder` both ways; `CurlFetcher`
against a 127.0.0.1 server (200, 404, a redirect, a timeout, a body over the limit, the
user agent seen by the server); `static_check`. `#[ignore]`d: the 14 originals of the
archive, read only, bundled with `CurlFetcher` (this one reaches the internet, so it is
never run by default) into a temporary directory: every external of each page's
`manifest.json` is found and fetched. (The real check of the same 14 is WP9's: 13 are
whole, Quiz Marking Desk too, whose grades live on claude.ai and which says so
offline; the Docs document is refused, its viewer asks claude.ai.)

Done: `CARGO_TARGET_DIR=target-pure cargo test -p writer-page` passes.

### WP3. WebKit (writer-web, new)

Owns `crates/writer-web`. No GPUI: `objc` 0.2, `block` 0.1, `dispatch` 0.2,
`core-foundation`. Everything on the main thread except `PageSource::file`. Interface:

```rust
pub const SCHEME: &str = "writer-page";
pub const CONTENT_SECURITY_POLICY: &str =
    "default-src 'self' data: blob: 'unsafe-inline' 'unsafe-eval'; form-action 'none'; base-uri 'self'";
pub const BLOCK_RULES: &str = r#"[
 {"trigger":{"url-filter":"^https?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^wss?:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^ftp:"},"action":{"type":"block"}},
 {"trigger":{"url-filter":"^file:"},"action":{"type":"block"}}
]"#;

/// Where the bytes of a page come from. Called on a background queue.
pub trait PageSource: Send + Sync + 'static {
    /// The file at `path` (no leading slash, percent-decoded); None answers 404.
    fn file(&self, path: &str) -> Option<ServedFile>;
}
#[derive(Clone)]
pub struct ServedFile { pub media_type: String, pub bytes: Arc<[u8]> }
pub struct MemorySource { /* HashMap<String, ServedFile> */ }
impl MemorySource { pub fn new(files: impl IntoIterator<Item = (String, ServedFile)>) -> MemorySource; }
impl PageSource for MemorySource;

/// A rectangle in points, origin at the top left of the window's content view.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rect { pub x: f64, pub y: f64, pub width: f64, pub height: f64 }

// ----- the check
pub struct CheckInput {
    pub source: Arc<dyn PageSource>,
    pub entry: String,
    pub also: Vec<String>,
    pub settle_min: Duration, pub settle_max: Duration, pub limit: Duration,
    pub rules_dir: PathBuf,             // where the rule list is compiled and kept
}
pub struct CheckOutcome { pub engine: String, pub elapsed: Duration, pub loads: Vec<LoadOutcome>, pub failure: Option<String> }
pub struct LoadOutcome {
    pub path: String, pub title: String, pub first_heading: String, pub text: String,
    pub finished: bool, pub navigation_error: Option<String>, pub crashed: bool,
    pub missing: Vec<String>, pub blocked: Vec<(String, String)>,   // (address, directive)
    pub script_errors: Vec<String>, pub requests: usize,
}
/// On the main thread of a process whose run loop runs (the app). An offscreen window
/// (borderless, far off screen, excluded from the Windows menu, occlusion detection off
/// where `_setWindowOcclusionDetectionEnabled:` answers) holds the view.
pub fn check(input: CheckInput, done: impl FnOnce(CheckOutcome) + 'static);
/// For a program (the tool's child): NSApplication with activation policy "prohibited",
/// runs `check`, returns when it is done.
pub fn check_blocking(input: CheckInput) -> CheckOutcome;

// ----- the view
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Appearance { Light, Dark, System }
pub struct ViewConfig {
    pub host: String,                    // "page-12"
    pub entry: String,
    pub source: Arc<dyn PageSource>,
    pub rules_dir: PathBuf,
    pub appearance: Appearance,
    pub background: (f64, f64, f64),     // underPageBackgroundColor, sRGB 0..1
}
/// What the view tells its owner, on the main thread.
pub trait PageEvents: 'static {
    fn loaded(&self);
    fn failed(&self, message: String);
    fn crashed(&self);
    /// The view became first responder by a click while the owner's focus was elsewhere.
    fn wants_focus(&self);
    /// A key the view received: true hands it to the owner's view instead of the page.
    fn takes_key(&self, key: &KeyPress) -> bool;
    /// A link the reader clicked to another site (http, https, mailto).
    fn open_externally(&self, url: String);
    fn alert(&self, message: String);
    /// The owner's items for the right-click menu; `link` is the link under the pointer.
    fn menu_items(&self, link: Option<&str>) -> Vec<MenuEntry>;
    fn menu_chosen(&self, id: u32);
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct KeyPress { pub key_code: u16, pub characters: String, pub characters_ignoring_modifiers: String, pub cmd: bool, pub ctrl: bool, pub alt: bool, pub shift: bool, pub function: bool }
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct MenuEntry { pub id: u32, pub title: String, pub enabled: bool, pub separator_before: bool }
pub struct Bitmap { pub width: u32, pub height: u32, pub bgra: Vec<u8> }   // device pixels
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum EditCommand { Copy, Cut, Paste, SelectAll, Undo, Redo }
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ViewStats { pub served: usize, pub missing: usize, pub blocked_navigations: usize }
pub struct PrintedPdf { pub bytes: Vec<u8>, pub pages: usize, pub paper: (f64, f64) }

pub struct WebPage { /* the WKWebView subclass, its delegates; !Send */ }
impl WebPage {
    /// Puts a view above `gpui_view` (an `NSView*`) in its superview and loads the entry
    /// once the rule list is compiled.
    pub unsafe fn new(gpui_view: *mut std::ffi::c_void, config: ViewConfig, events: Rc<dyn PageEvents>) -> WebPage;
    pub fn set_frame(&self, frame: Rect);
    pub fn set_hidden(&self, hidden: bool);
    pub fn set_holes(&self, holes: &[Rect]);       // in the view's own points, top left origin; empty: no mask
    pub fn focus(&self);                           // makeFirstResponder: the web view
    pub fn unfocus(&self);                         // makeFirstResponder: gpui_view, when the web view has it
    pub fn is_focused(&self) -> bool;
    pub fn snapshot(&self, done: impl FnOnce(Option<Bitmap>) + 'static);
    pub fn find(&self, query: &str, backwards: bool, done: impl FnOnce(bool) + 'static);
    pub fn clear_find(&self);
    pub fn set_zoom(&self, factor: f64);           // pageZoom
    pub fn set_appearance(&self, appearance: Appearance, background: (f64, f64, f64));
    pub fn edit(&self, command: EditCommand);
    pub fn reload(&self);                          // the entry again
    pub fn print_to_pdf(&self, done: impl FnOnce(Result<PrintedPdf, String>) + 'static);
    pub fn evaluate(&self, script: &str, done: impl FnOnce(Result<String, String>) + 'static);
    pub fn stats(&self) -> ViewStats;
}
impl Drop for WebPage;   // out of its superview, first responder given back, released

/// Asks the runtime for every class and selector this crate sends (a wrong one is an
/// exception, not a compile error), as writer_print::check_selectors does.
pub fn check_selectors() -> Result<(), String>;
```

Behaviour: section 5 entirely (the scheme handler with ranges and stopped tasks, CSP and
DNS headers, rule list, delegates, preferences, one non-persistent data store per process
for views, a new one per check), the check of section 3 (probe script and message handler
only in the check; settling), the right-click menu of section 6, `keyDown:` and
`becomeFirstResponder` of the subclass, snapshots as BGRA, print to PDF as a save job into
a temporary file, removed afterwards.

Tests (`tests/webkit.rs`, `harness = false`, one process, main thread): a 127.0.0.1
server that counts hits; the probe of Appendix A as assertions (every kind blocked and
reported, zero hits; own modules and workers load); 404 reported as missing; ranges
(`bytes=0-1`, `100-`, `-10`, outside: 416); an audio element does not play (`paused`
stays true); a link click (by `evaluate` dispatching a click event) reaches
`open_externally` and not the network; `window.open` to another site does nothing;
`alert` reaches `alert`; text and title read after a script wrote them; settling ends
within `settle_max`; a page that never finishes loading fails at the limit; a view in an
offscreen window: loads, snapshot non-empty, find true and false, zoom, print gives a
PDF with pages, holes set and cleared, `takes_key` is asked for `ctrl-return`;
`check_selectors` passes. Unit tests: the menu filter over identifiers, range parsing,
the frame conversion.

Done: `CARGO_TARGET_DIR=target-pure cargo test -p writer-web` passes on this Mac.

### WP4. Small additive changes (writer-ui, writer-print)

- `writer-ui`: `IconName::Globe` (`assets/icons/globe.svg`, drawn like the set) in
  `IconName::ALL`; `ToastLayer::toast_bounds(&self) -> Vec<Bounds<Pixels>>`: where each
  visible toast was drawn in the last frame, in window coordinates (empty without
  toasts). A test of each.
- `writer-print`: `impl Pdf { pub fn from_bytes(bytes: Vec<u8>, pages: usize) -> Pdf }`
  (the report has `pages`; the other counts are 0). A test that `bytes()` gives them back.

Done: `cargo test -p writer-ui`, `cargo test -p writer-print` pass; `cargo build -p
writer-app` compiles.

### WP5. Agents' command layer (writer-agent)

Owns `crates/writer-agent`. Adds dependencies `writer-page`. Interface:

```rust
// types.rs
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct AddPageRequest {
    pub path: String,                         // a file or a folder: absolute, ~/..., or relative to base_dir
    pub entry: Option<String>,
    pub title: Option<String>,
    pub folder: Option<Target>,
    pub create_folders: bool,
    pub source_url: Option<String>,
    /// Filled by Library::add_page (Prepare). What travels to the app.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub prepared: Option<PreparedPage>,
    #[serde(skip)]
    pub base_dir: Option<PathBuf>,
    #[serde(flatten)]
    pub write: WriteOptions,
}
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum PreparedPage {
    Unchanged { page: DocumentId, fingerprint: String },
    Page(writer_page::Prepared),
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum PageOutcome { Added, Updated, Unchanged }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageAdded {
    pub page: DocumentSummary,
    pub outcome: PageOutcome,
    pub snapshot: Option<VersionInfo>,         // the version kept before an update
    pub files: usize, pub bytes: u64, pub externals: usize,
    pub warnings: Vec<String>,
    #[serde(flatten)]
    pub written: Written,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct PageSummary {
    pub source_url: Option<String>, pub entry: String, pub files: usize, pub bytes: u64,
    pub externals: usize, pub added_at: Time, pub engine: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PagesRequest { pub folder: Option<Target> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PagesList { pub pages: Vec<PageListed> }
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct PageListed { pub page: DocumentSummary, pub summary: PageSummary }

// additions to existing types
pub struct DocumentSummary { /* ... */ #[serde(default)] pub kind: writer_store::DocumentKind, #[serde(default, skip_serializing_if = "Option::is_none")] pub source_url: Option<String> }
pub struct DocumentText { /* ... */ #[serde(default, skip_serializing_if = "Option::is_none")] pub page: Option<PageSummary> }
pub struct DocumentInfo { /* ... */ #[serde(default, skip_serializing_if = "Option::is_none")] pub page: Option<PageSummary> }
pub struct ExportRequest { /* ... */ pub original: bool }      // and the format "html": a folder
// Target::parse reads "page:<id>" as DocumentId(id).

// error.rs
pub enum ErrorKind { /* ... */ PageIncomplete /* 14, "page_incomplete" */, WrongKind /* 15, "wrong_kind" */ }
pub enum Error { /* ... */
    PageIncomplete { name: String, problems: Vec<writer_page::Problem> },
    WrongKind { id: i64, name: String, is_page: bool },
}

// library.rs
pub trait PageChecker: Send + Sync {
    fn check(&self, bundle: &writer_page::Bundle) -> Result<writer_page::CheckReport>;
}
/// Runs `<program> __page-check <dir>`; program: std::env::current_exe(). With
/// THE_WRITER_PAGE_CHECK=fake it answers with writer_page::static_check itself.
pub struct ProcessChecker { /* program */ }
impl ProcessChecker { pub fn current_exe() -> ProcessChecker; pub fn with_program(program: PathBuf) -> ProcessChecker; }
pub struct StaticChecker;               // writer_page::static_check, for tests
impl Library {
    pub fn with_page_checker(self, checker: Arc<dyn PageChecker>) -> Library;
    pub fn with_fetcher(self, fetcher: Arc<dyn writer_page::Fetcher>) -> Library;   // default: CachingFetcher over CurlFetcher
    pub fn pages(&self, request: &PagesRequest) -> Result<PagesList>;                 // a reader
    // add_page comes from the table (operations.rs)
}

// pages.rs, the table's function
pub(crate) fn add_page(session: &mut Session<'_>, request: &AddPageRequest) -> Result<PageAdded>;

// live
pub const PROTOCOL_VERSION: u32 = 3;
pub const MAX_PAGE_BYTES: usize = 64 * 1024 * 1024;
impl Incoming { pub fn request_mut(&mut self) -> &mut Request; }
/// For the app's listener thread, before it queues a request: stores the blobs of an
/// add_page that is not a dry run through `files`, empties their data, marks them stored.
/// Any other request is left as it is.
pub fn store_page_files(request: &mut Request, files: &mut writer_store::ImageFile) -> std::result::Result<(), Error>;
```

Behaviour: sections 3 (re-adding), 4 (what travels), 8 (reads), 9 (everything).
`content::load` refuses a page with `WrongKind`, which covers every text operation; the
item operations (`rename_document`, `move_document`, `duplicate_document`, trash, restore,
delete) branch for pages before they load content (rename computes the chosen-title flag
without the text); `read`, `info`, `find`, `versions`, `read_version`, `restore_version`
and `export` have a page branch; folder exports leave pages out with a warning. The table
in `operations.rs` gets the row of section 9; the test of the table knows `pages` as a
reader. `Change::DocumentCreated` / `DocumentText` / `DocumentRenamed` as in section 9.

Tests (`tests/suite/pages.rs`): add from a file and from a folder (with `MapFetcher` and
`StaticChecker`); unchanged, updated (with its version), added again after the old one
went to the trash; every `wrong_kind` operation; reads; export both ways; dry run writes
nothing; limits; the error reports and exit codes; `store_page_files` then the operation
on the main thread reads no bytes; the same through the stand-in app (the second run of
the suite), libraries compared row by row with `images.db`; a request of protocol 3
against a stand-in that answers as version 2 is refused whole.

Done: `CARGO_TARGET_DIR=target-pure cargo test -p writer-agent` passes (both runs).

### WP6. Library model and sidebar (writer-library)

Owns `crates/writer-library`. Interface:

```rust
pub use writer_store::{DocumentKind, FileId, FileInfo, PageContent};
pub struct DocumentEntry { /* ... */ pub kind: DocumentKind }
impl Library {
    /// As create_document, for a page: the same events.
    pub fn create_page(&mut self, folder: Option<FolderId>, page: &PageContent<'_>, cx: &mut Context<Self>) -> Result<DocumentMeta>;
    /// As save_document, for a page: the same events.
    pub fn save_page(&mut self, id: DocumentId, page: &PageContent<'_>, cx: &mut Context<Self>) -> Result<DocumentMeta>;
}
pub enum LibraryPanelEvent { /* ... */
    /// Files dropped on the panel: on a folder row (`folder`), else the folder selected, else the root.
    DropPaths { paths: Vec<PathBuf>, folder: Option<FolderId> },
}
```

The row of a page draws `IconName::Globe` (rows, the drag ghost); everything else of a row
is the same. The panel registers `on_drop::<ExternalPaths>` on its rows and its empty area
and emits `DropPaths` (the app decides what to take). The menu extension
(`set_menu_extension`) is asked for the library and folder targets as it is now; nothing
else changes in the menus.

Tests (`tests/panel/`): a page row has the globe; a drop emits `DropPaths` with the folder
under it; `create_page` emits the events of a new document.

Done: `cargo test -p writer-library` passes; `cargo build -p writer-app` compiles.

### WP7. The page view in the app (writer-app, wave 2)

Owns `crates/writer-app` in wave 2. Adds dependencies `writer-page`, `writer-web`.

**First milestone (commit it, then stop for the person check):** the scripted step
`page:make:<folder>[|<entry>]` (scripted runs only: reads the folder with
`writer_page::read_input`, refuses a page with externals, `writer_page::static_check`,
`prepare`, `put_file` for each blob and `create_page`, no WebKit check; the archive's
artifact folders with the entry `offline/index.html` are already offline), a page opened
from the sidebar in the real binary, focus sync, key routing, the snapshot overlay with
`drop_image`, and toast holes. A person runs checks 1 and 2 of section 13. Only when they
pass does the rest of WP7 go on; a failure goes back to the design (section 14, risk 1).
`page:make:` stays as a QA helper; WP9's `page:import:` is the real way in.

Then: section 6 entirely (the shape, `pages.rs` with the engine seam and
`LibraryPageSource`, `workspace/page.rs`, the routing in `session.rs`, `render.rs`,
the palette filter, the toasts of the commands that do not apply, find, zoom, theme,
crash, the context menus, the title and status items, the new actions and their places
in the keymap and the list of shortcuts, the scripted steps and `state`); section 7
(print through `PrintTarget`, export as a folder, the version history of a page). The
reload of an open page that changed in the library (an agent's update arrives as
`Change::DocumentText` in `take_over_open_document`: for a page, reload) is here; the
words of the agents' note are WP9's.

`LibraryPageSource` (`pages.rs`): a `writer_web::PageSource` over the manifest's served
files and the `ImageFile` connection `images::LibraryImages` holds for background tasks
(the mutex only background tasks lock), keeping whole files it read in memory up to 64 MB
for the life of the view.

Tests (`tests/suite/pages.rs`, fake engine): opening a page from the sidebar, quick open
and a launch; the session is `None` and the editor read-only meanwhile; text commands not
available and not in the palette; every toast of section 6; `cmd-1` and `cmd-0` move the
focus and the fake's first responder follows; a modal over the page makes one snapshot
and drops it when it closes (counts equal after 20 opens); a toast makes holes; find,
zoom, theme reach the fake; a crash reloads once, twice shows the placeholder; print
records one job of the page's PDF; export writes the folder; the version history of a
page; the title menu and the status items of a page with the pointer (`mouse.rs`) and
`ctrl-enter` (`context_menus.rs`); the keymap tests with the new context; the counts of
the list of shortcuts.

Done: `cargo test -p writer-app` passes; the person checks 1, 2, 7, 8, 9, 10 of section 13
are done on the real binary.

### WP8. Tool, MCP server, skill (writer-cli, writer-mcp, writer-agent-setup)

Owns the three crates. `writer-cli` adds `writer-page`, `writer-web`.

- `writer-cli`: the commands of section 9 in `spec.rs` (group "Documents": `page add`,
  `pages`), their parsing and output (`run.rs`), the hints (`main.rs`, `hint_in`),
  `writer __page-check <dir>` in `page_check.rs` (handled before parsing; maps
  `writer_web::CheckOutcome` to `writer_page::CheckReport`), `get`, `export`, `info`
  outputs for pages, exit codes 14 and 15 in the help.
- `writer-mcp`: `add_page` (section 9), `export` with `html` and `original`, `list_pages`
  hidden, the outputs of reads for pages (`render.rs`), `tool_text` for the new
  messages; the count test (37, 12).
- `writer-agent-setup`: the paragraph and the recipe of section 9 in `skill_body.md`,
  exit codes 14 and 15 in its list; its tests (the text held, the examples run against
  the built tool with `THE_WRITER_PAGE_CHECK=fake` and a temporary library).

Tests: `writer-cli`'s suite with `THE_WRITER_PAGE_CHECK=fake` (every new command, the
stand-in app for `page add`, `--list`, the exit codes); two cases with the real check
(WebKit in the child, a page with a font from a 127.0.0.1 server is added and its text
is right; a page that fetches at run time is refused with 14 and the address); the MCP
tool through the server's stdio.

Done: `CARGO_TARGET_DIR=target-pure cargo test -p writer-cli -p writer-mcp -p
writer-agent-setup` passes.

### WP9. Import, agents and the assistant in the app; docs (writer-app, writer-assistant, wave 3)

Owns `crates/writer-app` in wave 3, `crates/writer-assistant`, `CLAUDE.md`,
`docs/gpui-notes.md`, `scripts/bundle.sh`.

- File > Import Page… (`workspace::ImportPage`; the open panel with files and folders,
  several; `pages::use_open_panel`, `pages::choose_beforehand` for tests), drops (the
  sidebar's `DropPaths`, the page area, the text area through a hook on
  `images::LibraryImages`), and the flow: read and bundle on a background thread
  (`CachingFetcher` over `CurlFetcher`, or the fetcher a test installs:
  `pages::set_fetcher`), the check through the engine (`writer_web::check` for WebKit,
  `writer_page::static_check` or the fake's answer), `writer_page::prepare`, the blobs
  stored in the background through `LibraryImages`' `ImageFile`, then on the main thread
  `Library::create_page`, the sidebar showing it, the first page opened. Toasts: "Added
  “Orbitarium”" (with Open when it was not opened), the refusal's sentence, "3 files were
  not pages and were left out". `page:import:` step.
- The listener thread stores the blobs before queuing (`writer_agent::live::store_page_files`
  in `crates/writer-app/src/live.rs`), and the main thread's cost is measured
  (`live=[… cost=[…]]`).
- The agents' note words for `add_page` (`workspace::sentence`), Open as its button.
- The assistant: `writer_assistant::OpenDocument` gains `kind: OpenKind` (`#[derive(Default)]
  enum OpenKind { #[default] Text, Page { source_url: Option<String> } }`) and the
  sentence of section 8 in the prompt; the app sends a page's text (`assistant.rs`), the
  quick chat's toast.
- The library's menus get "Import Page…" (library and folder targets) and the page items
  on page rows (`extend_the_menus_of_other_crates`).
- `CLAUDE.md`: the crate table (two new crates; `writer-store`'s pages and `files`;
  `writer-agent`, `writer-mcp` 37 tools, `writer-cli`, `writer-app`), the dependency
  direction (`app → {page, web}`, `agent → page`, `cli → {page, web}`, `web` depends on
  nothing of ours, `page` on nothing of ours), the working rules (target-pure for both),
  a section "Pages" in the style of "Images", the scripted steps, the counts of the list
  of shortcuts, the exit codes. `docs/gpui-notes.md`: a section "A native view in the
  window" (the frame, the first responder, key equivalents as found, the snapshot and
  `drop_image`, the holes).
- `scripts/bundle.sh`: the smoke test adds a tiny page (no externals) with the bundled
  tool while the bundled app is open, and sees it in the library.

Tests: `tests/suite/pages.rs` gets the import (file, folder, several, a drop on each place,
a refusal), `tests/suite/live.rs` an agent's `add_page` while a page is open (reload, note)
and while a text is open (nothing in the editor changes), `tests/suite/assistant.rs` the
side chat with a page open (fake engine), `writer-assistant`'s own tests for the prompt.

Done: `cargo test -p writer-app` and `CARGO_TARGET_DIR=target-pure cargo test -p
writer-assistant` pass; the person checks 3 to 6 of section 13 are done; the 14 pages of
the archive go into a throwaway library with `writer page add --list` (all but the two
noted are added, the two are refused with their reasons) and show in the app with Wi-Fi
off.

### Which existing tests and counts change

- `writer-store`: `schema.rs` tests (`SCHEMA_VERSION` is 4), the covering index test,
  `tests/images.rs` literals of `ImagesCollected` and `ImagesBackedUp` (new fields).
- `writer-agent`: the test of the operations table; the error kinds' list and their exit
  codes; `PROTOCOL_VERSION` 3 in the live tests.
- `writer-mcp`: the tool count (37, of them 12 read-only).
- `writer-cli`: the help tests, the stand-in coverage of commands that change the
  library, the exit codes.
- `writer-agent-setup`: `skill_against_binary.rs` (exit codes 14 and 15), the skill text
  test.
- `writer-app`: `tests/suite/keymap.rs` and `keybindings.rs` (the new context, rows 377 /
  347, entries 402), `menus` (new items), `context_menus.rs` (a page's menus), `mouse.rs`
  (a page's title and status items).

---

## 13. Checks only a person can do

Each in under a minute, on the real binary (`cargo build -p writer-app`, `cargo build
-p writer-cli`), with `THE_WRITER_DATA_DIR` set to a temporary library that holds the
archive's pages: added with `writer --library <dir> page add --list …` once WP8 is in, and
before that (WP7's milestone) with a scripted run of `page:make:<artifact folder>|offline/index.html`
for each, followed by `quit`, then a normal launch on the same directory.

1. **Keys in an active window.** Open Reading Orbitarium, click into the page. Space plays
   and pauses, the arrows step. `cmd-shift-p`: the palette comes up over a still picture
   of the page; type, `escape`: the arrows work in the page again. `cmd-0` then `cmd-1`:
   the page has the keys again. Select a sentence, `cmd-c`, paste into TextEdit.
   `ctrl-enter`: our menu. `cmd-f`, type a word: it is marked in the page.
2. **Overlays.** With a page open: quick open (`cmd-p`), the settings (`cmd-,`), the list
   of shortcuts (`cmd-/`), a context menu: each is visible over the page and the page
   comes back after. Run `writer append` on another document from a terminal: the note at
   the bottom shows while the page goes on animating. Open and close the palette 20 times
   with Activity Monitor open: the app's memory comes back.
3. **The print panel.** Print… on a page: the sheet comes up with a preview of the
   pages; Cancel gives the keys back to the page; Save as PDF writes a PDF that looks
   like the page.
4. **A drop from the Finder.** Drag an `.html` file onto the sidebar: it is added in that
   folder. Drag a folder onto an open page: it is added and opens.
5. **No autoplay.** Open Reading Orbitarium: silence until Play is clicked.
6. **A link.** Click a link to another site in a page: Safari (the default browser) opens
   it; the page stays.
7. **Offline.** Turn Wi-Fi off, quit and open the app, open a page with Google Fonts and
   one with three.js (Nimbus Landing Variants): fonts and the 3D scene are there.
8. **The pointer.** Over a page: a hand over links, a text cursor over text, no flicker
   between GPUI's arrow and WebKit's cursors. Right-click: WebKit's menu without Reload,
   Download, Inspect, with our items at the end.
9. **Scrolling.** Two-finger scroll in a long page is smooth; the window resized while a
   page is open keeps the page in its place.
10. **Theme.** Switch to Night: a page that follows `prefers-color-scheme` turns dark;
    the area around the page has the theme's canvas.

---

## 14. Risks, and how the build retires each early

1. **Key routing in an active window** (spike finding d, never checked by a person). WP7's
   first milestone is exactly this, before anything else is built on it, with checks 1
   and 2. Fallbacks, in order: the web view subclass also overrides
   `performKeyEquivalent:` and hands every `cmd` chord to GPUI's view first; the view is
   made a subview of GPUI's view instead of a sibling (the spike's other mode).
2. **Overlay latency and memory.** The snapshot takes 55 to 90 ms and GPUI's `img()`
   leaked. WP3 delivers BGRA, WP7 paints `RenderImage`s and drops them; the test counts
   drops; check 2 measures memory. Fallback: hide the view without a snapshot (the area
   shows the canvas while an overlay is up).
3. **Cursor fights** between GPUI and WebKit over the page (check 8). Retired by WP7's
   first milestone too. Fallback: the page element sets no cursor style and the web view
   subclass re-asserts its cursor in `mouseMoved:`; else accept GPUI's arrow over the page.
4. **What the check cannot see** (workers, late requests). Retired by Appendix A for
   documents; WP3's test program holds it. The view blocks the rest anyway; the skill
   says that pages whose data lives on a service are not kept.
5. **Scheme handler threading** (answering a stopped task raises an exception; media make
   many range requests). WP3's tests switch pages fast during media loads and count no
   exception.
6. **The bundler misses references in real pages.** WP2's ignored test over the 14
   archived originals, run by hand during WP2 and again in WP9, before release.
7. **Main-thread cost of a page from an agent.** The listener stores the blobs; WP9
   measures `live=[… cost]` with the 16 MB page while a book is open and writes the
   numbers into CLAUDE.md.
8. **The schema bump** makes 0.7.0 refuse the library after 0.8.0 opened it. Accepted, as
   for 0.7.0 (question 9).
9. **WebKit in a command line program inside the app bundle** (NSApplication in a child
   of `writer`, the bundle id it picks up). WP8's real-check cases run the built binary;
   WP9's smoke test runs the bundled one.
10. **An MCP client's time limit** for a call that checks several pages. At most 10 pages
    per call; the description says to use the command line for more.
11. **Timing of animated pages** (text written late is missing from the text). Settling
    bounds; the text is for search and reading, not a promise of every frame.
12. **Disk.** Two small crates, no new heavy third-party crate, no new GPUI test binary.

---

## Questions for the user

Each has a default the design uses until the user says otherwise.

1. Where do pages go when no folder is named? Default: the top of the library; the skill
   suggests a folder "Claude Pages".
2. Should a page's own saved state (a theme it remembers, a quiz's progress) survive a
   relaunch? Default: no; it lasts while the app runs.
3. Pages among the documents of the same tree, with a globe? Default: yes, no separate
   section.
4. A click on a link to another site opens the default browser at once. Ask first
   instead? Default: open at once.
5. Should a page be added from an address directly (`writer page add https://…`)?
   Default: not in v1.
6. Should a page imported in the app from a file remember the file, so that importing it
   again updates it? Default: no; each import is a new page.
7. A page that needs something that cannot be stored (an embedded site, data from a
   service) is refused. Keep the rest of it with a warning instead? Default: refuse
   (your words: "not saved half").
8. Downloads from a page (a "Download CSV" button): refused in v1. Let them go to
   Downloads? Default: refused.
9. After 0.8.0 opens a library, 0.7.0 refuses it (as 0.7.0 did to 0.6.0). Fine?
   Default: yes.
10. Script errors of a page are warnings, not reasons to refuse it. Fine? Default: yes.

---

## Appendix A. The CSP probe of 2026-11-13

`scratchpad/pages-design/csp/probe.swift` (this session's scratchpad), macOS 27.0.1,
arm64: an offscreen `WKWebView`, a `WKURLSchemeHandler` on `writer-page://check/`, a
`securitypolicyviolation` listener injected at document start in every frame, a
navigation delegate that cancels `http(s)` frames, and a Python server on 127.0.0.1 that
logs every request. The page asks for a style sheet, a picture, a CSS background, a font
(`@font-face`), an autoplaying `<audio>`, an external `<iframe>`, `fetch`, `WebSocket`,
`EventSource`, `sendBeacon`, `XMLHttpRequest`, a fetch from a worker (`w.js`, served with
its own CSP header) and a fetch from a same-origin iframe; it imports an ES module
(`./mod.js`) and starts the worker from the scheme.

| Run | Requests that reached the server | CSP reports | Own module and worker |
|---|---|---|---|
| no rule list, no CSP (control) | 11 (`a.css`, `b.png`, `bg.png`, `c`, `d`, `e`, `f.woff2`, `frominner`, `fromworker`, `s.mp3`, `xhr`) | 0 | loaded |
| CSP header only | 0 | 13 (every one, with its address and directive, including the inner frame's; the worker's fetch blocked, not reported) | loaded |
| rule list only | 0 | 0 | loaded |
| both | 0 | 13 | loaded |

What it showed: `'self'` matches the custom scheme (modules and workers load under the
CSP); the CSP blocks every kind of request and names it, which is what the check needs;
the rule list blocks the same without naming; an `<audio>` is fetched (preloaded) even
when it may not play; the beacon of the control was not sent before the probe ended.

## Appendix B. The archive in numbers

Read from `<page archive>/archive.json` and the folders (read only).

| Page | Kind | Externals | Served (offline + vendor) | Files | Largest file |
|---|---|---|---|---|---|
| Lantern Redesign Concepts | index + 12 concept pages, 6 style sheets, 42 images | 64 | 16.2 MB | 125 | 1.95 MB PNG |
| Nimbus Landing Variants | index + 9 option pages + 37 images | 133 | 10.6 MB | 180 | 0.6 MB (three.js r128) |
| Reading Orbitarium | page + 17 narration MP3s | 12 | 11.8 MB | 30 | 1.37 MB MP3 |
| Nimbus Brand Sheet | page + 42 PNGs | 19 | 6.3 MB | 62 | 0.32 MB |
| Orbitarium | plain page | 31 | 0.9 MB | 32 | 0.2 MB |
| Tidewater Field Guide | plain page | 34 | 1.1 MB | 35 | 0.23 MB |
| the six other plain pages | plain page | 12 to 28 | 0.3 to 0.5 MB | 13 to 29 | under 0.1 MB |
| Quiz Marking Desk | page with `window.claude` db and user | 16 | 0.4 MB | 17 | 0.1 MB |
| Nimbus Planning Doc | a Claude Docs document (the type's page connects to the service) | 0 | 0.1 MB offline rebuild; 32 MB original viewer | 1 | |

The externals are mostly Google Fonts (CSS and WOFF2); Nimbus Landing Variants also has
three.js r128 from cdnjs. Two are not keepable as they come: the Docs document (its viewer
needs the service; its content is kept when exported) and Quiz Marking Desk's saved
grades (they live in a claude.ai database; the page itself is kept).
