// Links of elements (docs/whiteboard-design.md, 4.11). A link opens in the reader's browser
// only when the reader opened it: the app's gate allows a navigation to the address of a
// real <a> under a real click (the link popover's), never a script's. Excalidraw's link
// mark on the canvas calls window.open with no <a> under the pointer, which the gate
// refuses; so here nothing is opened, and nothing a message says ever opens a link.
import { CaptureUpdateAction } from "@excalidraw/excalidraw";
import { board } from "./scene.js";

const ELEMENT_LINK_KEY = "element";

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
