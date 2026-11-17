# Inner Word: the landing page

A static page: `index.html`, `styles.css`, `script.js` and `assets/`. No build step, no
framework, no web fonts, no analytics. The page makes no request to any other site; the
only addresses it names are links to GitHub. Every path is relative, so it works at a
domain root (`https://innerword.app/`) and under a subpath
(`https://basalbit.github.io/inner_word/`) alike.

## Preview locally

```bash
cd site
python3 -m http.server 8000
# open http://localhost:8000/
```

Opening `index.html` straight from the disk works too. Only the "Copy" button of the
Terminal command needs `http://localhost`, because browsers allow clipboard access only
on a secure origin.

## Host it

Any static host serves the folder as it is.

- **GitHub Pages, with Actions** (the way this repository is set up): the workflow
  `.github/workflows/pages.yml` publishes this folder on every push to `main` that changes
  `site/`, and on demand from the Actions tab (*Run workflow*). One setting is needed once:
  *Settings > Pages > Build and deployment > Source: GitHub Actions*. The page is then at
  `https://basalbit.github.io/inner_word/`. GitHub Pages can publish a folder from a branch
  only when it is the repository root or `/docs`, which is why the workflow is used.
- **Netlify**: a new site from the repository, base directory `site`, no build command,
  publish directory `site`. Or drag the `site` folder onto Netlify Drop.
- **Cloudflare Pages**: connect the repository, framework preset *None*, no build
  command, build output directory `site`. Or upload the folder directly
  (`npx wrangler pages deploy site`).

### The address of the page

The social preview and the canonical address assume GitHub Pages at
`https://basalbit.github.io/inner_word/`: `<link rel="canonical">`, `og:url`, `og:image`
and `twitter:image` in `index.html` name it (social sites need an absolute image
address). If the page is hosted anywhere else, change those four addresses.

## A new release

The version is named in one place: the attributes on the `<html>` element at the top of
`index.html`.

```html
<html lang="en" data-version="0.10.1" data-repo="https://github.com/BasalBit/inner_word"
      data-size-universal="25 MB" data-size-arm64="12 MB">
```

`script.js` builds every download link and every version label from them. For a new
release:

1. Change `data-version` (and the two sizes, if they changed).
2. Publish a GitHub release with the tag **`v<version>`** (for example `v0.11.0`) and
   the two disk images under exactly these names:
   - `Inner-Word-<version>-universal.dmg`
   - `Inner-Word-<version>-arm64.dmg`

   Pushing the tag runs `.github/workflows/release.yml`, which builds both images with
   `scripts/bundle.sh` under those names and makes a draft release with them; check the
   draft and publish it. The links are
   `https://github.com/BasalBit/inner_word/releases/download/v<version>/Inner-Word-<version>-universal.dmg`
   and the same with `-arm64`. A link to a tag or a file name that does not exist is a
   404, so check both after publishing.
3. If the release changes what the page says (requirements, features, the first-launch
   steps in `packaging/Read Me First.txt`), change the text too.

With JavaScript, a click on a download button also scrolls to the first-launch steps
(`#install`) and says that the download has started, and on a phone or a tablet
(`(max-width: 600px), (pointer: coarse)`) the hero's button reads "Get it for your Mac"
and leads to the download section instead, which asks the reader to open the page on a
Mac and offers to copy its address.

Without JavaScript the download buttons lead to the latest release
(`https://github.com/BasalBit/inner_word/releases/latest`) and the version labels are left out,
so the page is never wrong, only less direct.

Until the repository is public, every GitHub link on the page is a 404 for visitors:
publish the page after the repository and the first release are public.

## Screenshots and other assets

`assets/` holds the screenshots of the real app (0.10.0) as WebP, each in Paper and in
Night (`-dark`) and at two widths for `srcset`, the app icon (`logo.webp`,
`favicon.png`, `apple-touch-icon.png`) and the social preview (`og-image.jpg`,
1200 x 630). The hero and the saved web page are the whole window (2400 and 1200 pixels
wide; the hero has a crop of the text column for phones, `hero-mobile`); in both the
window's grey traffic lights were painted out, since the window was not the active one
when it was taken. The others are crops of the part they show: the sheet of page view
alone (no desk around it, so it sits on the page's own desk), the palette panel alone
(its rounded corners transparent), the format bar, the list of shortcuts, Vim's command
line, the find bar, the library, the comments, the side chat, the note of an agent's
change and the Agents panel. Each crop ends between two paragraphs or two rows, never
through a line of text; the shot of the agent's change has a band of the app's own
canvas colour added under the note, where the window went on with a line cut by the
status bar. All of them were taken in one scripted run of the release app on one
temporary library, so the library, its times and the text agree from shot to shot (the
agent's edit is applied only in the shot of that edit).

Four screenshots have a second crop for phones (`-phone`, one size, at 2x), chosen by a
`<source media="(max-width: 600px)">` as the hero's is, because the wide crop would
shrink to unreadable type there: the palette (the query and two rows, fading out to the
right and below), the format bar (the bar and the paragraph under it, cut after "leave,"
with the rest of the line painted out in the canvas colour), the comments (the panel
with both threads) and the Agents panel (the switch and the two installed agents: rows
of the panel stacked as they are in it, with the descriptions between them left out).

To make a new WebP from a PNG screenshot:

```bash
cwebp -q 82 -m 6 -resize 2400 0 shot.png -o assets/name.webp
cwebp -q 80 -m 6 -resize 1200 0 shot.png -o assets/name-1200.webp
```

Every screenshot is a `<picture>` whose first `<source>` is the Night version for a
reader in dark mode (after the phone sources, where a screenshot has them). The hero also has a small Paper / Night switch: its Night sources
carry their media without the colour scheme in `data-media`, which `script.js` uses.

The side chat is a crop below its header: it was taken with the app's test engine, whose
header names a fake agent. Its context chip ("Chapter 2: Salt and Lamp Oi") is cut by
the app itself, which gives a chip a fixed largest width. Retake it with Claude Code to
show the whole panel.
