# Inner Word: the landing page

A static page at <https://innerword.io/>: `index.html`, `styles.css`, `script.js`,
`assets/` and `.htaccess` (the server's settings). No build step, no framework, no web
fonts, no analytics. The page makes no request to any other site; the only addresses it
names are links to GitHub. Every path is relative, so it works at a domain root and under
a subpath alike.

## Preview locally

```bash
cd site
python3 -m http.server 8000
# open http://localhost:8000/
```

Opening `index.html` straight from the disk works too. Only the "Copy" button of the
Terminal command needs `http://localhost`, because browsers allow clipboard access only
on a secure origin. `python3 -m http.server` does not read `.htaccess`, so the
redirects, headers and Content-Security-Policy apply only on the server.

## Where it is served from

The page is at **<https://innerword.io/>**, on the owner's Hostinger shared hosting
(LiteSpeed), in `/home/<REMOTE_USER>/domains/innerword.io/public_html`. DNS for
`innerword.io` and `www.innerword.io` points at Hostinger's CDN, and Hostinger issues the
TLS certificate itself. GitHub Pages is no longer used (the old address,
`basalbit.github.io/inner_word/`, is not kept up to date).

### The workflow

`.github/workflows/site.yml` ("Site") publishes this folder on every push to `main` that
changes `site/` or the workflow, and on demand from the Actions tab (*Run workflow*). It
copies `site/` with `rsync -rlptz --delete --chmod=D755,F644` over SSH (port 65002) into
`public_html`: what is no longer in `site/` is removed there, `README.md` (these notes)
is never published, and `.well-known/` on the server is never deleted (Hostinger may use
it for certificate checks). It publishes a staged copy in which `index.html` names
`styles.css?v=<commit>` and `script.js?v=<commit>` (the first 12 characters of the commit),
because Hostinger's CDN and browsers keep the style sheet and the script up to an hour and
a fix would otherwise reach some readers late; `site/index.html` itself keeps the plain
names, so the local preview works, and pictures get a new file name when they change. If
anything else ever looks stale on the live site, hPanel > *Performance* > *CDN* > *Flush
cache* empties the CDN by hand. Then it fetches `https://innerword.io/` a few times and checks
that the page serves the `data-version` of `index.html` and names the versioned `styles.css`; while the certificate of a new
domain is not issued yet, that is a warning and not a failure.

`.htaccess` sends `http://` and `www.innerword.io` to `https://innerword.io/` (301, the path
kept), hides dotfiles (except `.well-known`), sets the caching (pictures 7 days,
`styles.css` and `script.js` an hour, the page itself `no-cache`) and the security
headers, among them a Content-Security-Policy that allows only the page's own script,
style sheet and pictures. A new inline `<script>`, `style="..."` attribute, `data:` image,
font or request to another site has to be added to that policy, or the browser blocks it.

### The secrets

Three repository secrets (*Settings > Secrets and variables > Actions*):

- `SSH_PRIVATE_KEY`: the private half of the deploy key.
- `REMOTE_USER`: the SSH user of the hosting account (hPanel > *SSH Access*).
- `REMOTE_HOST`: the server's IP address (hPanel > *SSH Access*). The workflow refuses
  any other address than the one whose host key it pins.

**The deploy key** is an ed25519 key made for this repository only; its public half is in
hPanel > *SSH Access* > *SSH keys* under the name "innerword.io GitHub Actions". To rotate
it:

```bash
ssh-keygen -t ed25519 -N "" -C "innerword.io GitHub Actions" -f innerword-deploy
# hPanel > SSH Access: add innerword-deploy.pub, then delete the old key there
gh secret set SSH_PRIVATE_KEY -R BasalBit/inner_word < innerword-deploy
rm innerword-deploy innerword-deploy.pub   # run the workflow once to check
```

**The server's host key is pinned** in the workflow (`PINNED_HOST` and `KNOWN_HOST`), and
SSH checks it strictly, so a server that is not Hostinger's is refused. If Hostinger moves
the account to another server (the IP in hPanel > *SSH Access* changes, or the job fails
with "Host key verification failed"), read the new key and compare it with the
fingerprint hPanel shows before trusting it:

```bash
ssh-keyscan -p 65002 -t ed25519 <new ip> > hostkey
ssh-keygen -lf hostkey          # must equal the SHA256 fingerprint in hPanel
```

Then put the line from `hostkey` into `KNOWN_HOST`, the IP into `PINNED_HOST`, and the
IP into the secret `REMOTE_HOST`.

### The address of the page

`<link rel="canonical">`, `og:url`, `og:image` and `twitter:image` in `index.html` name
`https://innerword.io/` (social sites need an absolute image address). If the page ever
moves, change those four addresses, `homepage` in the root `Cargo.toml` and the links in
the repository's `README.md`.

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
