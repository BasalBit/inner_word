// Text is measured with whatever font is loaded at that moment: the same text measured 26 %
// wider once Excalifont was there (the scout's run). So every page loads the fonts of the
// texts it is about to measure before it restores, converts or inserts them (4.3).
import { exportToCanvas } from "@excalidraw/excalidraw";

/**
 * Loads the font faces the text elements of `elements` use, for their characters.
 *
 * Excalidraw registers its faces with document.fonts only when it loads fonts for elements
 * (Fonts.loadElementsFonts), which its exports do and nothing else that is exported does: so
 * the texts are exported once onto a canvas of one pixel. With no texts this still registers
 * every face (the canvas is then empty, and the export may throw after the fonts are in;
 * that is ignored).
 */
export async function loadFontsFor(elements) {
  const texts = (elements ?? []).filter((e) => e && e.type === "text" && !e.isDeleted && typeof e.text === "string");
  try {
    await exportToCanvas({
      elements: texts.map((e) => ({ ...e, containerId: null, frameId: null, groupIds: [] })),
      appState: { exportBackground: false },
      files: {},
      maxWidthOrHeight: 1,
    });
  } catch {
    // An empty list: the faces are registered all the same.
  }
  try {
    await document.fonts.ready;
  } catch {
    // Nothing to wait for.
  }
}
