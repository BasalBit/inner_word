// The whiteboard's bundle: the real Excalidraw, its glue and its fonts, built into one
// offline folder, ../dist (docs/whiteboard-design.md, section 3).
//
//   npm ci --ignore-scripts --no-audit --no-fund   # the only step that reaches the network
//   node build.mjs                                 # writes ../dist from nothing
//   node build.mjs --check                         # builds into a temporary folder and compares
//                                                  # it with ../dist, byte for byte
//
// The same inputs give the same bytes: nothing written here carries a time, an absolute
// path or the order a directory happened to be read in.
import * as esbuild from "esbuild";
import * as fontkit from "fontkit";
import crypto from "node:crypto";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";

const WEB = path.dirname(fileURLToPath(import.meta.url));
const DIST = path.resolve(WEB, "..", "dist");
const CHECK = process.argv.includes("--check");
const EXCALIDRAW_DIR = path.join(WEB, "node_modules", "@excalidraw", "excalidraw");
const EXCALIDRAW_PROD = path.join(EXCALIDRAW_DIR, "dist", "prod");

// ---------------------------------------------------------------------------------------
// What is accepted, and the media type of every file of dist/.

/** The licenses a file of the bundle may be under (about.toml's list, and OFL for fonts). */
const ACCEPTED = [
  "MIT",
  "ISC",
  "Apache-2.0",
  "BSD-2-Clause",
  "BSD-3-Clause",
  "0BSD",
  "CC0-1.0",
  "Zlib",
  "Unlicense",
  "OFL-1.1",
];

/** By ending; a file of any other ending fails the build. */
const MEDIA_TYPES = {
  ".html": "text/html; charset=utf-8",
  ".js": "text/javascript; charset=utf-8",
  ".css": "text/css; charset=utf-8",
  ".woff2": "font/woff2",
  ".wasm": "application/wasm",
  ".json": "application/json",
  ".svg": "image/svg+xml",
  ".png": "image/png",
  ".txt": "text/plain; charset=utf-8",
};

/** Excalidraw's numbers for the families a scene names (FONT_FAMILY in 0.18.1), and the
 * folder of dist/fonts/ each one's files are in. Helvetica (2) is the system's: no file. */
const FAMILY_TABLE = '{Virgil:1,Helvetica:2,Cascadia:3,Excalifont:5,Nunito:6,"Lilita One":7,"Comic Shanns":8,"Liberation Sans":9}';
const SCENE_FAMILIES = [
  { number: 1, name: "Virgil", folder: "Virgil" },
  { number: 3, name: "Cascadia", folder: "Cascadia" },
  { number: 5, name: "Excalifont", folder: "Excalifont" },
  { number: 6, name: "Nunito", folder: "Nunito" },
  { number: 7, name: "Lilita One", folder: "Lilita" },
  { number: 8, name: "Comic Shanns", folder: "ComicShanns" },
  { number: 9, name: "Liberation Sans", folder: "Liberation" },
];
/** The characters font-metrics.json gives advances for: U+0020 to U+024F. */
const FIRST_CHAR = 0x20;
const LAST_CHAR = 0x24f;

function fail(message) {
  throw new Error(message);
}

function sha256(bytes) {
  return crypto.createHash("sha256").update(bytes).digest("hex");
}

function walk(dir, base = dir) {
  let found = [];
  for (const name of fs.readdirSync(dir).sort()) {
    const full = path.join(dir, name);
    if (fs.statSync(full).isDirectory()) found = found.concat(walk(full, base));
    else found.push(path.relative(base, full).split(path.sep).join("/"));
  }
  return found;
}

function readJson(file) {
  return JSON.parse(fs.readFileSync(file, "utf8"));
}

/** JSON as every file of dist/ writes it: two spaces, keys in the order given, a final newline. */
function jsonText(value) {
  return JSON.stringify(value, null, 2) + "\n";
}

// ---------------------------------------------------------------------------------------
// The patches. Each one must apply exactly as often as it says, or the build fails, so
// that a new Excalidraw cannot drop one quietly.

const patches = {
  fontCdn: 0,
  xiaolai: 0,
  locales: 0,
};

function excalidrawPlugin() {
  const localeFiles = fs
    .readdirSync(path.join(EXCALIDRAW_PROD, "locales"))
    .filter((name) => name.endsWith(".js"));
  const localesKept = localeFiles.filter((name) => /^(en|percentages)-/.test(name));
  const localesLeftOut = localeFiles.length - localesKept.length;
  if (localesKept.length !== 2) fail(`expected the en locale and the percentages beside the others, found ${localesKept.join(", ")}`);
  let familyTableSeen = 0;

  return {
    name: "innerword-excalidraw",
    setup(build) {
      // Patch 3: the locales other than English are never loaded (langCode "en"); each
      // dynamic import of one resolves to one empty module instead.
      build.onResolve({ filter: /^\.\/locales\/[^/]+\.js$/ }, (args) => {
        if (!args.importer.startsWith(EXCALIDRAW_PROD + path.sep)) return undefined;
        const name = args.path.slice("./locales/".length);
        if (/^(en|percentages)-/.test(name)) return undefined;
        patches.locales++;
        return { path: "locale-left-out", namespace: "innerword-left-out" };
      });
      build.onLoad({ filter: /.*/, namespace: "innerword-left-out" }, () => ({
        // Excalidraw reads the module's keys and falls back to English for each one missing.
        contents: "export default {};\n",
        loader: "js",
      }));

      build.onLoad({ filter: /[\\/]@excalidraw[\\/]excalidraw[\\/]dist[\\/]prod[\\/].*\.js$/ }, async (args) => {
        let text = await fs.promises.readFile(args.path, "utf8");

        // Patch 1: no font CDN. Excalidraw appends
        // https://esm.sh/@excalidraw/excalidraw@<v>/dist/prod/ to the src of every
        // FontFace whatever EXCALIDRAW_ASSET_PATH says; WebKit checks each against the
        // policy at once. Fall back to it only when no local path was given (never here).
        const cdn = /return ([\w$]+)\.push\(new URL\(([\w$]+),([\w$]+)\.ASSETS_FALLBACK_URL\)\),\1\}/g;
        text = text.replace(cdn, (_, list, file, cls) => {
          patches.fontCdn++;
          return `return ${list}.length||${list}.push(new URL(${file},${cls}.ASSETS_FALLBACK_URL)),${list}}`;
        });

        // Patch 2: no Xiaolai (answer 1). The list of its faces is emptied, so no FontFace
        // names a file the bundle does not have; CJK text falls back to the Mac's own font.
        const xiaolaiPaths = new Set();
        for (const m of text.matchAll(/var ([\w$]+)="\.\/fonts\/Xiaolai\/[^"]+\.woff2";/g)) xiaolaiPaths.add(m[1]);
        if (xiaolaiPaths.size) {
          const faces = /var ([\w$]+)=\[\{uri:([\w$]+),descriptors:\{unicodeRange:"[^"]*"\}\}(?:,\{uri:[\w$]+,descriptors:\{unicodeRange:"[^"]*"\}\})*\];/g;
          text = text.replace(faces, (whole, list, first) => {
            if (!xiaolaiPaths.has(first)) return whole;
            const uris = [...whole.matchAll(/\{uri:([\w$]+),/g)].map((m) => m[1]);
            if (!uris.every((uri) => xiaolaiPaths.has(uri))) fail("a list of Xiaolai's faces names another font");
            patches.xiaolai++;
            return `var ${list}=[];`;
          });
          // The paths themselves go too: nothing in the bundle names a file it does not have.
          text = text.replace(/var ([\w$]+)="\.\/fonts\/Xiaolai\/[^"]+\.woff2";/g, (_, name) => `var ${name}="";`);
        }

        if (text.includes(FAMILY_TABLE)) familyTableSeen++;
        return { contents: text, loader: "js" };
      });

      build.onEnd((result) => {
        if (result.errors.length) return;
        if (patches.fontCdn !== 1) fail(`the font CDN patch applied ${patches.fontCdn} times, expected 1`);
        if (patches.xiaolai !== 1) fail(`the Xiaolai patch applied ${patches.xiaolai} times, expected 1`);
        if (patches.locales !== localesLeftOut) fail(`the locales patch applied ${patches.locales} times, expected ${localesLeftOut}`);
        if (familyTableSeen !== 1) fail(`Excalidraw's table of font families changed (expected ${FAMILY_TABLE} once): read the font numbers again`);
      });
    },
  };
}

// ---------------------------------------------------------------------------------------
// The licenses: every package that has bytes in the bundle, each text once.

/** An SPDX expression to the license it is taken under, or null when none is accepted. */
function acceptedLicense(expression) {
  if (!expression) return null;
  const text = String(expression).trim().replace(/^\((.*)\)$/, "$1");
  if (/ AND /i.test(text)) {
    const parts = text.split(/ AND /i).map((p) => p.trim().replace(/^\((.*)\)$/, "$1"));
    return parts.every((p) => ACCEPTED.includes(p)) ? parts.join(" AND ") : null;
  }
  const alternatives = text.split(/ OR /i).map((p) => p.trim());
  for (const license of ACCEPTED) if (alternatives.includes(license)) return license;
  return null;
}

/** What a license file says, for a package whose package.json names none. */
function licenseOfText(raw) {
  const text = raw.replace(/\s+/g, " ");
  if (/Permission is hereby granted, free of charge, to any person obtaining a copy/i.test(text)) return "MIT";
  if (/Permission to use, copy, modify, and\/or distribute this software for any/i.test(text)) return "ISC";
  if (/Apache License[\s\S]*Version 2\.0/i.test(text)) return "Apache-2.0";
  if (/Redistribution and use in source and binary forms/i.test(text)) {
    return /Neither the name/i.test(text) ? "BSD-3-Clause" : "BSD-2-Clause";
  }
  return null;
}

function licenseFile(dir) {
  const names = fs.existsSync(dir) ? fs.readdirSync(dir).sort() : [];
  const name = names.find((n) => /^(licen[cs]e|copying)([.-].*)?$/i.test(n) && fs.statSync(path.join(dir, n)).isFile());
  return name ? path.join(dir, name) : null;
}

function packageOfInput(input) {
  const at = input.lastIndexOf("node_modules/");
  if (at < 0) return null;
  const rest = input.slice(at + "node_modules/".length).split("/");
  const name = rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0];
  return { name, dir: path.join(WEB, input.slice(0, at + "node_modules/".length) + name) };
}

function packagesOfSourceMap(file) {
  const map = `${file}.map`;
  if (!fs.existsSync(map)) return [];
  const own = packageOfInput(path.relative(WEB, file).split(path.sep).join("/"));
  const found = new Set();
  for (const source of readJson(map).sources ?? []) {
    const at = source.lastIndexOf("node_modules/");
    if (at < 0) continue;
    const rest = source.slice(at + "node_modules/".length).split("/");
    const name = rest[0].startsWith("@") ? `${rest[0]}/${rest[1]}` : rest[0];
    if (name !== own?.name) found.add(name);
  }
  return [...found];
}

function lockfilePackages() {
  const lock = readJson(path.join(WEB, "package-lock.json"));
  const names = new Set();
  for (const key of Object.keys(lock.packages ?? {})) {
    const at = key.lastIndexOf("node_modules/");
    if (at >= 0) names.add(key.slice(at + "node_modules/".length));
  }
  return names;
}

function notices(metafile, fontEntries) {
  const inBundle = new Map(); // name@version -> { name, version, dir }
  for (const output of Object.values(metafile.outputs)) {
    for (const [input, { bytesInOutput }] of Object.entries(output.inputs)) {
      if (!bytesInOutput) continue;
      const pkg = packageOfInput(input);
      if (!pkg) continue;
      const manifest = readJson(path.join(pkg.dir, "package.json"));
      const key = `${pkg.name}@${manifest.version}`;
      if (!inBundle.has(key)) inBundle.set(key, { name: pkg.name, version: manifest.version, dir: pkg.dir, files: new Set() });
      inBundle.get(key).files.add(path.join(WEB, input));
    }
  }

  const declared = readJson(path.join(WEB, "bundled-inside.json")).carriers;
  const textsOfOurs = readJson(path.join(WEB, "bundled-inside.json")).textsOfOurs ?? {};
  const withoutText = [];
  const usedTexts = new Set();
  const lockfile = lockfilePackages();
  const entries = []; // { label, license, note, text }
  const used = new Set();

  for (const pkg of [...inBundle.values()].sort((a, b) => (a.name < b.name ? -1 : a.name > b.name ? 1 : 0))) {
    const manifest = readJson(path.join(pkg.dir, "package.json"));
    const file = licenseFile(pkg.dir);
    const text = file ? fs.readFileSync(file, "utf8") : null;
    let named = manifest.license ?? (Array.isArray(manifest.licenses) ? manifest.licenses.map((l) => l.type ?? l).join(" OR ") : null);
    if (named && typeof named === "object") named = named.type;
    let license = acceptedLicense(named);
    let note = null;
    if (!named) {
      license = text ? licenseOfText(text) : null;
      if (!license) fail(`${pkg.name} ${pkg.version} names no license and its license file says nothing known`);
      note = "its package.json names no license; read from its license file";
    } else if (!license) {
      fail(`${pkg.name} ${pkg.version} is under ${named}, which is not accepted`);
    } else if (license !== named.replace(/^\((.*)\)$/, "$1")) {
      note = `${named}, taken under ${license}`;
    }
    let ownText = text;
    if (!ownText) {
      // A package that ships no license file: the text is ours (licenses/), named in
      // bundled-inside.json's "textsOfOurs" by the package's name or its scope ("@radix-ui/*").
      const key = Object.keys(textsOfOurs).find((k) => k === pkg.name || (k.endsWith("/*") && pkg.name.startsWith(k.slice(0, -1))));
      const ours = key ? textsOfOurs[key] : null;
      if (!ours) {
        withoutText.push(`${pkg.name} ${pkg.version}`);
        continue;
      }
      if (ours.license !== license) fail(`licenses/ gives ${pkg.name} the ${ours.license} license, its package.json ${license}`);
      ownText = fs.readFileSync(path.join(WEB, ours.text), "utf8");
      note = note ? `${note}; the package ships no license text, this one is from its repository` : "the package ships no license text; this one is from its repository";
      usedTexts.add(key);
    }
    entries.push({ label: `${pkg.name} ${pkg.version}`, license, note, text: ownText });

    // Packages built into this package's own files, which the metafile never shows.
    const key = `${pkg.name}@${pkg.version}`;
    const carried = new Set();
    for (const f of pkg.files) for (const name of packagesOfSourceMap(f)) carried.add(name);
    const declaration = declared[key];
    if (carried.size && !declaration) fail(`${key} carries ${[...carried].join(", ")} inside its files: list them in bundled-inside.json`);
    if (!declaration) continue;
    used.add(key);
    const named_ = new Set([...(declaration.fromNodeModules ?? []), ...Object.keys(declaration.withATextOfOurs ?? {})]);
    for (const name of carried) {
      if (!named_.has(name)) fail(`${key} carries ${name} inside its files, which bundled-inside.json does not list`);
    }
    for (const name of declaration.fromNodeModules ?? []) {
      if (!lockfile.has(name)) fail(`bundled-inside.json lists ${name} inside ${key}, and the lockfile has no such package`);
      const dir = path.join(WEB, "node_modules", name);
      const inner = readJson(path.join(dir, "package.json"));
      const innerFile = licenseFile(dir);
      if (!innerFile) fail(`${name} (inside ${key}) has no license file`);
      const innerLicense = acceptedLicense(inner.license);
      if (!innerLicense) fail(`${name} (inside ${key}) is under ${inner.license}, which is not accepted`);
      entries.push({
        label: `${name} (inside ${pkg.name} ${pkg.version}; the text of ${name} ${inner.version})`,
        license: innerLicense,
        note: null,
        text: fs.readFileSync(innerFile, "utf8"),
      });
    }
    for (const [name, own] of Object.entries(declaration.withATextOfOurs ?? {})) {
      if (!ACCEPTED.includes(own.license)) fail(`${name} (inside ${key}) is under ${own.license}, which is not accepted`);
      entries.push({
        label: `${name} ${own.version} (inside ${pkg.name} ${pkg.version})`,
        license: own.license,
        note: own.note ?? null,
        text: fs.readFileSync(path.join(WEB, own.text), "utf8"),
      });
    }
  }
  if (withoutText.length) fail(`packages without a license text (add one to licenses/ and to textsOfOurs in bundled-inside.json): ${withoutText.join(", ")}`);
  for (const key of Object.keys(declared)) {
    if (!used.has(key)) fail(`bundled-inside.json names ${key}, which has no file in the bundle (read the carriers' dist again)`);
  }
  for (const key of Object.keys(textsOfOurs)) {
    if (!usedTexts.has(key)) fail(`bundled-inside.json gives ${key} a text, and the bundle has no such package`);
  }

  // Each text once, with the packages under it.
  const texts = new Map();
  for (const entry of entries) {
    const text = entry.text.replace(/\r\n/g, "\n").trim();
    if (!texts.has(text)) texts.set(text, []);
    texts.get(text).push(entry);
  }

  const rule = "=".repeat(78);
  let out = "";
  out += "The whiteboard of Inner Word: the licenses of what is in this folder\n";
  out += `${rule}\n\n`;
  out += "This folder is the whiteboard's editor, Excalidraw (https://github.com/excalidraw/excalidraw),\n";
  out += "built with the JavaScript packages it uses and its fonts. Every package that has code in\n";
  out += "the folder is listed below with its version and license, then each license text once with\n";
  out += "the packages under it, then the fonts.\n\n";
  out += "Packages\n--------\n\n";
  for (const entry of entries) out += `${entry.label}: ${entry.license}${entry.note ? ` (${entry.note})` : ""}\n`;
  out += "\nLicense texts\n-------------\n";
  for (const [text, under] of texts) {
    out += `\n${rule}\n${under.map((e) => e.label).join("\n")}\n${rule}\n\n${text}\n`;
  }
  out += "\nFonts\n-----\n\n";
  out += "The fonts in fonts/ (Xiaolai, Excalidraw's Chinese, Japanese and Korean hand-drawn font,\n";
  out += "is not in this folder).\n\n";
  for (const font of fontEntries) {
    out += `${font.family} (fonts/${font.folder}/, ${font.use}): ${font.license}\n`;
    for (const line of font.copyright) out += `  ${line}\n`;
  }
  out += `\n${rule}\nThe SIL Open Font License 1.1, for every font above under OFL-1.1\n${rule}\n\n`;
  out += fs.readFileSync(path.join(WEB, "fonts-licenses", "OFL-1.1.txt"), "utf8").trim() + "\n";
  out += `\n${rule}\nThe MIT License of Comic Shanns\n${rule}\n\n`;
  out += fs.readFileSync(path.join(WEB, "fonts-licenses", "ComicShanns-MIT.txt"), "utf8").trim() + "\n";
  return { text: out, packages: entries.map((e) => e.label) };
}

// ---------------------------------------------------------------------------------------
// The fonts: copied without Xiaolai, their copyright lines checked, their metrics written.

function copyFonts(out) {
  const source = path.join(EXCALIDRAW_PROD, "fonts");
  const table = readJson(path.join(WEB, "fonts-licenses", "fonts.json")).folders;
  const folders = fs.readdirSync(source).sort().filter((f) => f !== "Xiaolai");
  for (const folder of folders) {
    const info = table[folder] ?? fail(`fonts-licenses/fonts.json has no entry for the font folder ${folder}`);
    if (!ACCEPTED.includes(info.license)) fail(`the font ${folder} is under ${info.license}, which is not accepted`);
    for (const name of fs.readdirSync(path.join(source, folder)).sort()) {
      const file = path.join(source, folder, name);
      const font = fontkit.openSync(file);
      const said = String(font.copyright ?? "");
      for (const line of info.copyright) {
        if (!said.includes(line)) fail(`${folder}/${name}: its name table does not say "${line}" (it says "${said.slice(0, 200)}")`);
      }
      fs.mkdirSync(path.join(out, "fonts", folder), { recursive: true });
      fs.copyFileSync(file, path.join(out, "fonts", folder, name));
    }
  }
  for (const folder of Object.keys(table)) {
    if (!folders.includes(folder)) fail(`fonts-licenses/fonts.json names ${folder}, which Excalidraw does not have`);
  }
  return folders.map((folder) => ({ folder, ...table[folder] }));
}

function fontMetrics(out) {
  const families = {};
  for (const family of SCENE_FAMILIES) {
    const dir = path.join(out, "fonts", family.folder);
    const files = fs.readdirSync(dir).sort();
    const fonts = files.map((name) => fontkit.openSync(path.join(dir, name)));
    const unitsPerEm = fonts[0].unitsPerEm;
    if (!fonts.every((f) => f.unitsPerEm === unitsPerEm)) fail(`the files of ${family.name} differ in units per em`);
    const advances = [];
    for (let code = FIRST_CHAR; code <= LAST_CHAR; code++) {
      let advance = null;
      for (const font of fonts) {
        if (!font.hasGlyphForCodePoint(code)) continue;
        const width = font.glyphForCodePoint(code).advanceWidth;
        if (advance === null) advance = width;
        else if (advance !== width) fail(`${family.name}: two of its files give U+${code.toString(16)} different advances`);
      }
      advances.push(advance);
    }
    families[String(family.number)] = { name: family.name, unitsPerEm, advances };
  }
  return {
    about: "The advance width of every character from U+0020 to U+024F in the fonts a scene can name, in font units, by Excalidraw's family number; null where the family has no glyph (the text falls back to another font there). Written by build.mjs from the files of fonts/; innerword-board keeps a copy.",
    excalidraw: readJson(path.join(EXCALIDRAW_DIR, "package.json")).version,
    first: FIRST_CHAR,
    last: LAST_CHAR,
    families,
  };
}

// ---------------------------------------------------------------------------------------
// The build.

async function build(out) {
  fs.rmSync(out, { recursive: true, force: true });
  fs.mkdirSync(out, { recursive: true });
  for (const key of Object.keys(patches)) patches[key] = 0;

  const result = await esbuild.build({
    absWorkingDir: WEB,
    entryPoints: { main: "src/main.jsx" },
    bundle: true,
    format: "esm",
    splitting: true,
    outdir: path.join(out, "assets"),
    entryNames: "[name]",
    chunkNames: "chunk-[hash]",
    assetNames: "[name]-[hash]",
    minify: true,
    target: ["safari15"],
    jsx: "automatic",
    conditions: ["production"],
    define: { "process.env.NODE_ENV": '"production"' },
    // The CSS names its fonts as ./fonts/…; they are copied beside index.html.
    external: ["./fonts/*"],
    plugins: [excalidrawPlugin()],
    metafile: true,
    legalComments: "eof",
    charset: "utf8",
    logLevel: "warning",
  });

  // The interface font, Assistant, is named by main.css relative to assets/.
  const cssPath = path.join(out, "assets", "main.css");
  const css = fs.readFileSync(cssPath, "utf8");
  const rewritten = css.replaceAll("url(./fonts/", "url(../fonts/").replaceAll('url("./fonts/', 'url("../fonts/');
  if (rewritten === css) fail("main.css names no ./fonts/: the interface font moved");
  fs.writeFileSync(cssPath, rewritten);

  for (const page of ["index.html", "export.html"]) fs.copyFileSync(path.join(WEB, "src", page), path.join(out, page));

  const fontEntries = copyFonts(out);
  fs.writeFileSync(path.join(out, "font-metrics.json"), jsonText(fontMetrics(out)));

  const notice = notices(result.metafile, fontEntries);
  fs.writeFileSync(path.join(out, "NOTICES.txt"), notice.text);

  // The Mermaid chunks, so that a test can hold that they are there and whole.
  const mermaidChunks = Object.entries(result.metafile.outputs)
    .filter(([, o]) => Object.keys(o.inputs).some((i) => /node_modules\/mermaid\//.test(i)))
    .map(([file]) => path.relative(out, path.join(WEB, file)).split(path.sep).join("/"))
    .sort();

  const inputs = {};
  for (const file of ["package.json", "package-lock.json", ".npmrc", "build.mjs", "bundled-inside.json"]) {
    inputs[file] = sha256(fs.readFileSync(path.join(WEB, file)));
  }
  for (const dir of ["src", "fonts-licenses", "licenses"]) {
    for (const file of walk(path.join(WEB, dir))) inputs[`${dir}/${file}`] = sha256(fs.readFileSync(path.join(WEB, dir, file)));
  }
  const version = (name) => readJson(path.join(WEB, "node_modules", name, "package.json")).version;
  const buildInfo = {
    about: "What dist/ was built from: the versions, the SHA-256 of every input under web/, the patches applied. Written by build.mjs; a test of the crate holds the hashes against web/.",
    excalidraw: version("@excalidraw/excalidraw"),
    react: version("react"),
    reactDom: version("react-dom"),
    esbuild: version("esbuild"),
    fontkit: version("fontkit"),
    mermaid: version("mermaid"),
    mermaidToExcalidraw: version("@excalidraw/mermaid-to-excalidraw"),
    inputs,
    patches: [
      { name: "no font CDN", applied: patches.fontCdn },
      { name: "no Xiaolai", applied: patches.xiaolai },
      { name: "locales other than English left out", applied: patches.locales },
    ],
    mermaidChunks,
    packages: notice.packages,
  };
  fs.writeFileSync(path.join(out, "BUILD-INFO.json"), jsonText(buildInfo));

  const files = [];
  for (const file of walk(out)) {
    if (file === "MANIFEST.json") continue;
    const type = MEDIA_TYPES[path.extname(file)] ?? fail(`dist/${file}: no media type for its ending`);
    const bytes = fs.readFileSync(path.join(out, file));
    files.push({ path: file, bytes: bytes.length, sha256: sha256(bytes), type });
  }
  fs.writeFileSync(
    path.join(out, "MANIFEST.json"),
    jsonText({ about: "Every file of this folder but this one: its path, size, SHA-256 and the media type it is served with. Written by build.mjs.", files }),
  );
  return { files: files.length, bytes: files.reduce((n, f) => n + f.bytes, 0) };
}

function compare(built, committed) {
  const a = walk(built);
  const b = fs.existsSync(committed) ? walk(committed) : [];
  const problems = [];
  for (const file of a) if (!b.includes(file)) problems.push(`missing from dist/: ${file}`);
  for (const file of b) if (!a.includes(file)) problems.push(`in dist/ but not built: ${file}`);
  for (const file of a) {
    if (!b.includes(file)) continue;
    if (!fs.readFileSync(path.join(built, file)).equals(fs.readFileSync(path.join(committed, file)))) {
      problems.push(`differs: ${file}`);
    }
  }
  return problems;
}

const started = Date.now();
if (CHECK) {
  const temporary = fs.mkdtempSync(path.join(os.tmpdir(), "innerword-board-web-check-"));
  try {
    await build(temporary);
    const problems = compare(temporary, DIST);
    if (problems.length) {
      console.error(`dist/ is not what web/ builds (${problems.length} differences):`);
      for (const p of problems.slice(0, 50)) console.error(`  ${p}`);
      process.exitCode = 1;
    } else {
      console.log(`dist/ is what web/ builds, byte for byte (${walk(DIST).length} files, ${Date.now() - started} ms)`);
    }
  } finally {
    fs.rmSync(temporary, { recursive: true, force: true });
  }
} else {
  const made = await build(DIST);
  console.log(`built dist/: ${made.files} files, ${(made.bytes / 1e6).toFixed(2)} MB, in ${Date.now() - started} ms`);
}
