// Links of elements (docs/whiteboard-design.md, 4.11). A link opens in the reader's browser
// only when the reader opened it: the app's gate allows a navigation to the address of a
// real <a> under a real click (the link popover's), never a script's. Excalidraw's link
// mark on the canvas calls window.open with no <a> under the pointer, which the gate
// refuses; so here nothing is opened, and nothing a message says ever opens a link.
import { CaptureUpdateAction } from "@excalidraw/excalidraw";
import { board } from "./scene.js";

const ELEMENT_LINK_KEY = "element";

/** The schemes a link of an element may have (innerword_board's LINK_SCHEMES): the web,
 * mail, and the view's own (Excalidraw's link to an element of the drawing). */
const LINK_SCHEMES = new Set(["http", "https", "mailto", "innerword-page"]);

/**
 * Whether an element's link is kept (innerword_board's `link_is_kept`, which every scene
 * the app reads goes through): one without a scheme, or one of LINK_SCHEMES. Any other
 * (`file:`, `smb:`, an app's own scheme) would be an `<a>` in an exported PDF, which a click
 * in Preview hands to the Finder or to that app.
 */
export function linkIsKept(link) {
  if (typeof link !== "string") return true;
  const cleaned = link.replace(/^[\u0000-\u0020]+|[\u0000-\u0020]+$/g, "").replace(/[\t\n\r]/g, "");
  const colon = cleaned.indexOf(":");
  if (colon < 0) return true;
  const scheme = cleaned.slice(0, colon);
  if (!/^[A-Za-z][A-Za-z0-9+.-]*$/.test(scheme)) return true;
  return LINK_SCHEMES.has(scheme.toLowerCase());
}

/** The elements with every link that is not kept taken out (a paste, the export page). */
export function withKeptLinks(elements) {
  return elements.map((e) => (e && e.link && !linkIsKept(e.link) ? { ...e, link: null } : e));
}

/** Excalidraw's onLinkOpen. */
export function onLinkOpen(element, event) {
  const link = String(element?.link ?? "");
  if (/^(https?|mailto):/i.test(link)) return; // the gate decides; the canvas mark opens nothing
  event.preventDefault();
  let url;
  try {
    url = new URL(link, location.href);
  } catch {
    return;
  }
  const id = url.searchParams.get(ELEMENT_LINK_KEY);
  if (!id || url.host !== location.host) return;
  const api = board.api;
  const target = api.getSceneElements().find((e) => e.id === id);
  if (!target) return;
  api.updateScene({ appState: { selectedElementIds: { [id]: true } }, captureUpdate: CaptureUpdateAction.NEVER });
  api.scrollToContent(target, { fitToContent: false, animate: true });
}
