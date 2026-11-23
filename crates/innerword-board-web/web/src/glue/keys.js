// Keys on a whiteboard (docs/whiteboard-design.md, 4.7). The window keeps its own keys and
// Excalidraw has the rest; here the page makes sure of two things the app cannot: that
// Excalidraw never acts on a key the whiteboard leaves out of it, and that no tooltip or
// menu item of Excalidraw's names a key the window takes, or one of the whiteboard's own
// keys the user gave a command (the app says which, at the load and after every install of
// its keymap; product rule 3: a key shown is a key that does it, and the user can change
// the window's keys).
import { post } from "./bridge.js";

/**
 * Keys Excalidraw would act on and must not: the window has them (save, export, its
 * palette, quick open) or they open what the whiteboard leaves out (open a file, save to
 * disk, the image export, the theme). Stopped before Excalidraw sees them, and not
 * prevented, so that a key the window's menu bar has still reaches it.
 */
const NOT_EXCALIDRAWS = [
  { meta: true, shift: false, alt: false, key: "s" }, // save
  { meta: true, shift: true, alt: false, key: "s" }, // save to disk: has no switch in 0.18.1
  { meta: true, shift: false, alt: false, key: "o" }, // open
  { meta: true, shift: true, alt: false, key: "e" }, // the image export dialog
  { meta: true, shift: false, alt: false, key: "/" }, // its command palette
  { meta: true, shift: true, alt: false, key: "p" }, // its command palette
  { meta: true, shift: false, alt: false, key: "p" }, // its hint "use the palette"
  { meta: false, shift: true, alt: true, code: "KeyD" }, // its theme (off anyway)
];

/**
 * The keys no tooltip or menu item of Excalidraw's names, as Excalidraw writes them on a Mac
 * (getShortcutKey): the window takes these keys (or the user may give them to another
 * command), so a tooltip that named one would name a key that does something else here.
 */
export const KEYS_THE_WINDOW_TAKES = ["Cmd++", "Cmd+-", "Cmd+0", "?", "Cmd+F", "Cmd+/", "Cmd+Shift+P", "Cmd+S", "Cmd+Shift+S", "Cmd+Shift+E", "Cmd+O"];

/** Shifted punctuation (a US keyboard) as the key without shift: Excalidraw writes
 * "Cmd+Shift+<" where the keymap writes `cmd-shift-,`, and "Cmd++" for `cmd-shift-=`. */
const SHIFTED = { "<": ",", ">": ".", "?": "/", ":": ";", '"': "'", "{": "[", "}": "]", "|": "\\", "~": "`", _: "-", "+": "=" };

/** Excalidraw's names of keys, as the keymap names them ("Delete" is the Mac's delete key,
 * which the keymap calls backspace). */
const EXCALIDRAWS_NAMES = { up: "up", down: "down", left: "left", right: "right", enter: "enter", return: "enter", esc: "escape", escape: "escape", delete: "backspace", backspace: "backspace", space: "space", tab: "tab" };

const EXCALIDRAWS_MODIFIERS = { cmd: "cmd", ctrl: "ctrl", option: "alt", alt: "alt", shift: "shift" };
const APPS_MODIFIERS = new Set(["cmd", "ctrl", "alt", "shift"]);

/** One key in one form, to compare a key Excalidraw shows with one the app names:
 * `ctrl-alt-shift-cmd-<key>` with the modifiers it has, in that order. */
function oneForm(modifiers, key) {
  if (!key) return null;
  const has = new Set(modifiers);
  if (SHIFTED[key]) {
    has.add("shift");
    key = SHIFTED[key];
  }
  return [...["ctrl", "alt", "shift", "cmd"].filter((m) => has.has(m)), key.toLowerCase()].join("-");
}

/** A key as Excalidraw writes it ("Cmd+Option+[", "Shift+Option+C", "Cmd++"), or null for
 * what is not one key ("Option+Drag", a word). */
export function keyShown(text) {
  let rest = String(text ?? "").trim();
  let key;
  if (rest === "+" || rest.endsWith("++")) {
    key = "+";
    rest = rest.slice(0, -2);
  } else {
    const at = rest.lastIndexOf("+");
    key = rest.slice(at + 1);
    rest = at >= 0 ? rest.slice(0, at) : "";
  }
  const modifiers = [];
  for (const word of rest ? rest.split("+") : []) {
    const modifier = EXCALIDRAWS_MODIFIERS[word.toLowerCase()];
    if (!modifier) return null;
    modifiers.push(modifier);
  }
  const named = EXCALIDRAWS_NAMES[key.toLowerCase()];
  if (named) key = named;
  else if ([...key].length !== 1) return null;
  return oneForm(modifiers, key);
}

/** A key as the app names it (innerword_keymap's normal form: `cmd-k`, `cmd-alt-[`,
 * `alt-shift-c`, `cmd-shift-,`), or null. */
export function keyNamed(text) {
  let rest = String(text ?? "").trim().toLowerCase();
  let key;
  if (rest === "-" || rest.endsWith("--")) {
    key = "-";
    rest = rest.slice(0, -2);
  } else {
    const at = rest.lastIndexOf("-");
    key = rest.slice(at + 1);
    rest = at >= 0 ? rest.slice(0, at) : "";
  }
  const modifiers = rest ? rest.split("-") : [];
  if (!key || modifiers.some((m) => !APPS_MODIFIERS.has(m))) return null;
  return oneForm(modifiers, key);
}

const TAKEN_BY_THE_WINDOW = new Set(KEYS_THE_WINDOW_TAKES.map(keyShown).filter(Boolean));

/** The whiteboard's own keys the user gave a command that runs on a whiteboard (the app
 * says which: `keysGivenUp` of board.json, then the call `keys`), in one form. */
let givenUp = new Set();

/** Whether a key Excalidraw shows is not to be shown: the window has it. */
function notShown(text) {
  const key = keyShown(text);
  return key !== null && (TAKEN_BY_THE_WINDOW.has(key) || givenUp.has(key));
}

/** The keys of a list ("R or 2", "Cmd+K") that may be shown, or null when none is left. */
function keysLeft(text) {
  const parts = text.split(" or ");
  const kept = parts.filter((part) => !notShown(part));
  if (kept.length === parts.length) return text;
  return kept.length ? kept.join(" or ") : null;
}

/** A tooltip ("Group — Cmd+G", "Link - Cmd+K") without the keys that are not shown. */
function titleLeft(title) {
  for (const separator of [" \u2014 ", " - "]) {
    const at = title.lastIndexOf(separator);
    if (at < 0) continue;
    const keys = keysLeft(title.slice(at + separator.length));
    return keys === null ? title.slice(0, at) : title.slice(0, at + separator.length) + keys;
  }
  return title;
}

/** Where Excalidraw's menus write a key as an item's whole text. */
const SHORTCUT_TEXTS = ".context-menu-item__shortcut, .dropdown-menu-item__shortcut, .welcome-screen-menu-item__shortcut";

/** What was changed of each element, to give it back when a key is no longer given up:
 * the attribute as Excalidraw wrote it and as it is shown. */
const changedAttributes = new WeakMap();
/** The menus' key texts hidden. */
const hiddenTexts = new WeakSet();

function tidyAttribute(element, name, rewrite) {
  const now = element.getAttribute(name);
  const memo = changedAttributes.get(element)?.get(name);
  const original = memo && now === memo.shown ? memo.original : now;
  if (original === null) return;
  const shown = rewrite(original);
  if (shown !== now) element.setAttribute(name, shown);
  let memos = changedAttributes.get(element);
  if (shown === original) {
    memos?.delete(name);
    return;
  }
  if (!memos) changedAttributes.set(element, (memos = new Map()));
  memos.set(name, { original, shown });
}

function tidyText(element) {
  const hide = notShown(element.textContent);
  if (hide && !hiddenTexts.has(element)) {
    element.style.display = "none";
    hiddenTexts.add(element);
  } else if (!hide && hiddenTexts.has(element)) {
    element.style.display = "";
    hiddenTexts.delete(element);
  }
}

function tidyKeys(root) {
  const each = (selector, act) => {
    if (root.matches?.(selector)) act(root);
    for (const element of root.querySelectorAll?.(selector) ?? []) act(element);
  };
  each("[title]", (element) => tidyAttribute(element, "title", titleLeft));
  // Emptied rather than taken away, so that it is found again when a key comes back.
  each("[aria-keyshortcuts]", (element) => tidyAttribute(element, "aria-keyshortcuts", (keys) => keysLeft(keys) ?? ""));
  each(SHORTCUT_TEXTS, tidyText);
}

/** Looks at every tooltip and menu key again (the keys given up changed). */
let tidyAll = () => {};

/** The keys the app gave up to commands of the user's (innerword_keymap's normal form). */
export function setKeysGivenUp(keys) {
  givenUp = new Set((Array.isArray(keys) ? keys : []).map(keyNamed).filter(Boolean));
  tidyAll();
}

function matches(rule, event) {
  if (!!rule.meta !== event.metaKey || !!rule.shift !== event.shiftKey || !!rule.alt !== event.altKey || event.ctrlKey) return false;
  if (rule.code) return event.code === rule.code;
  return event.key.toLowerCase() === rule.key;
}

function typingInto(target) {
  if (!(target instanceof HTMLElement)) return false;
  return target.isContentEditable || target.tagName === "INPUT" || target.tagName === "TEXTAREA" || target.tagName === "SELECT";
}

export function installKeys() {
  // On window, in the capture phase, and before Excalidraw mounts: first of all listeners.
  window.addEventListener(
    "keydown",
    (event) => {
      if (NOT_EXCALIDRAWS.some((rule) => matches(rule, event))) {
        event.stopImmediatePropagation();
        return;
      }
      // Excalidraw's help: the app's own list of shortcuts, at "On a whiteboard".
      if (event.key === "?" && !event.metaKey && !event.ctrlKey && !event.altKey && !typingInto(event.target)) {
        event.preventDefault();
        event.stopImmediatePropagation();
        post("help");
      }
    },
    true,
  );
}

/** Keeps the keys that are not shown out of the tooltips and menu items, and the web embed
 * out of the extra tools' menu (it shares its test id with "Mermaid to Excalidraw", so it is
 * found by its label). */
export function watchTheInterface(labelOfTheEmbed) {
  const tidy = (root) => {
    tidyKeys(root);
    const label = labelOfTheEmbed();
    for (const item of root.querySelectorAll?.('.App-toolbar__extra-tools-dropdown [data-testid="toolbar-embeddable"]') ?? []) {
      if (item.textContent.trim() === label && item.style.display !== "none") item.style.display = "none";
    }
  };
  const observer = new MutationObserver((mutations) => {
    for (const mutation of mutations) {
      if (mutation.type === "attributes") tidy(mutation.target);
      else for (const node of mutation.addedNodes) if (node.nodeType === 1) tidy(node.parentElement ?? node);
    }
  });
  observer.observe(document.body, { subtree: true, childList: true, attributes: true, attributeFilter: ["title", "aria-keyshortcuts"] });
  tidyAll = () => tidy(document.body);
  tidy(document.body);
}
