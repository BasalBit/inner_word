// The export page (export.html), loaded offscreen to make a whiteboard's PDF
// (docs/whiteboard-design.md, section 5): the stored scene drawn by Excalidraw's own
// exportToSvg, in light colours, at scale 1 (or less when a side would pass maxSide), with
// its fonts inlined, mended where Excalidraw's SVG differs from its canvas (a frame's clip,
// the room around an arrow's label), and the page then showing that SVG alone at (0, 0).
import { convertToExcalidrawElements, exportToSvg, restore, restoreElements } from "@excalidraw/excalidraw";
import { defineCalls } from "./calls.js";
import { loadFontsFor } from "./fonts.js";
import { withKeptLinks } from "./links.js";
import { loadPictures } from "./pictures.js";
import { NEVER_KEPT, markReady } from "./scene.js";

async function storedScene() {
  const response = await fetch("./scene.json");
  if (!response.ok) throw new Error(`scene.json could not be read (${response.status})`);
  return await response.json();
}

function sizeOf(svg) {
  return { width: parseFloat(svg.getAttribute("width")), height: parseFloat(svg.getAttribute("height")) };
}

const SVG_NS = "http://www.w3.org/2000/svg";

/** Excalidraw 0.18.1's `BOUND_TEXT_PADDING`: the room the canvas leaves around the label of
 * an arrow or a line. */
const BOUND_TEXT_PADDING = 5;

/** What Excalidraw's exportToSvg draws otherwise than its canvas, mended so that the PDF is
 * what the writer sees (docs/whiteboard-design.md, section 5):
 *
 * - A frame's children are wrapped in a group whose clip is set with
 *   `setAttributeNS(SVG_NS, "clip-path", …)`: an attribute in the SVG namespace, which no
 *   renderer reads (presentation attributes have none), so the children were drawn whole
 *   where the canvas cuts them at the frame's edge. The attribute is set again plainly.
 * - The line of an arrow or a line is masked out under its label, by the label's box
 *   exactly, where the canvas clears the box and `BOUND_TEXT_PADDING` around it: the line
 *   ran up to the letters. The hole of each such mask is made as large as the canvas's. */
function asTheCanvasDraws(svg) {
  for (const group of svg.querySelectorAll("g")) {
    const clip = group.getAttributeNS(SVG_NS, "clip-path");
    if (clip !== null) {
      group.removeAttributeNS(SVG_NS, "clip-path");
      group.setAttribute("clip-path", clip);
    }
  }
  for (const hole of svg.querySelectorAll('mask[id^="mask-"] > rect[fill="#000"]')) {
    const number = (name) => parseFloat(hole.getAttribute(name));
    const [x, y, width, height] = ["x", "y", "width", "height"].map(number);
    if (![x, y, width, height].every(Number.isFinite)) continue;
    hole.setAttribute("x", `${x - BOUND_TEXT_PADDING}`);
    hole.setAttribute("y", `${y - BOUND_TEXT_PADDING}`);
    hole.setAttribute("width", `${width + 2 * BOUND_TEXT_PADDING}`);
    hole.setAttribute("height", `${height + 2 * BOUND_TEXT_PADDING}`);
  }
  return svg;
}

function showOnly(svg) {
  for (const element of [document.documentElement, document.body]) {
    element.style.margin = "0";
    element.style.padding = "0";
    element.style.background = "transparent";
  }
  document.body.style.overflow = "hidden";
  svg.style.display = "block";
  document.body.replaceChildren(svg);
}

const exportCalls = {
  /** {"padding": 32, "maxSide": 14400} -> {"width", "height"} of the SVG the page shows (CSS
   * pixels, which a PDF makes points). */
  async exportLayout({ padding = 32, maxSide = 14400 }) {
    const scene = await storedScene();
    const elements = withKeptLinks(
      (Array.isArray(scene.elements) ? scene.elements : []).filter((e) => e && !e.isDeleted && !NEVER_KEPT.has(e.type)),
    );
    if (!elements.length) throw new Error("the whiteboard is empty: there is nothing to export");
    const files = await loadPictures(scene.files);
    await loadFontsFor(elements);
    const restored = restore({ elements, appState: scene.appState ?? {}, files }, null, null, {
      refreshDimensions: true,
      repairBindings: true,
    });
    const live = restored.elements.filter((e) => !e.isDeleted);
    const background = restored.appState.viewBackgroundColor || "#ffffff";
    const draw = (scale) =>
      exportToSvg({
        elements: live,
        appState: {
          exportBackground: true,
          viewBackgroundColor: background,
          exportWithDarkMode: false,
          exportEmbedScene: false,
          // Excalidraw's default is the display's pixel ratio: a Retina PDF would be twice
          // the drawing's size.
          exportScale: scale,
        },
        files,
        exportPadding: padding,
      });
    let svg = await draw(1);
    let size = sizeOf(svg);
    const longest = Math.max(size.width, size.height);
    if (longest > maxSide) {
      svg = await draw(maxSide / longest);
      size = sizeOf(svg);
    }
    showOnly(asTheCanvasDraws(svg));
    return size;
  },

  /** Excalidraw's own conversion of skeletons, for the converter's cross-check (B11):
   * {"skeletons": [...]} -> {"elements": [...]}. */
  async convertSkeletons({ skeletons }) {
    const list = Array.isArray(skeletons) ? skeletons : [];
    // Measured once the fonts of their texts are in: converted once to know the texts.
    await loadFontsFor(convertToExcalidrawElements(list, { regenerateIds: false }));
    const converted = convertToExcalidrawElements(list, { regenerateIds: false });
    const elements = restoreElements(converted, null, { refreshDimensions: true, repairBindings: true });
    return { elements };
  },
};

export function startExport() {
  defineCalls(exportCalls, { needsReady: false });
  markReady("");
}
