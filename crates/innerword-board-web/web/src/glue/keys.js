// Keys on a whiteboard (docs/whiteboard-design.md, 4.7). The window keeps its own keys and
// Excalidraw has the rest; here the page makes sure of two things the app cannot: that
// Excalidraw never acts on a key the whiteboard leaves out of it, and that no tooltip of
// Excalidraw's names a key the window takes (product rule 3: a key shown is a key that
// does it, and the user can change the window's keys).
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
 * The keys of the controls whose tooltip loses its key, as Excalidraw writes them on a Mac
 * (getShortcutKey): the window takes these keys (or the user may give them to another
 * command), so a tooltip that named one would name a key that does something else here.
 */
export const KEYS_THE_WINDOW_TAKES = ["Cmd++", "Cmd+-", "Cmd+0", "?", "Cmd+F", "Cmd+/", "Cmd+Shift+P", "Cmd+S", "Cmd+Shift+S", "Cmd+Shift+E", "Cmd+O"];

/** The controls whose tooltip names one of those keys in 0.18.1. */
const CONTROLS_WITHOUT_KEYS = [".zoom-in-button", ".zoom-out-button", ".reset-zoom-button", ".help-icon"];

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

function cutKey(control) {
  const title = control.getAttribute("title");
  if (title) {
    const at = title.indexOf(" — ");
    if (at >= 0) control.setAttribute("title", title.slice(0, at));
  }
  if (control.hasAttribute("aria-keyshortcuts")) control.removeAttribute("aria-keyshortcuts");
}

/** Keeps the keys out of those tooltips, and the web embed out of the extra tools' menu
 * (it shares its test id with "Mermaid to Excalidraw", so it is found by its label). */
export function watchTheInterface(labelOfTheEmbed) {
  const tidy = (root) => {
    for (const selector of CONTROLS_WITHOUT_KEYS) {
      if (root.matches?.(selector)) cutKey(root);
      for (const control of root.querySelectorAll?.(selector) ?? []) cutKey(control);
    }
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
  tidy(document.body);
}
