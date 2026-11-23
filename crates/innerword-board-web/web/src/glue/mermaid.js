// Mermaid to drawing (docs/whiteboard-design.md, 4.13): the `mermaid` call opens Excalidraw's
// own dialog, or converts a definition as the dialog's Insert does and puts the drawing at
// the middle of the view, selected. Mermaid's code is loaded only when it is used.
import { CaptureUpdateAction, convertToExcalidrawElements } from "@excalidraw/excalidraw";
import { centred } from "./library.js";
import { board } from "./scene.js";

async function parse(definition) {
  const { parseMermaidToExcalidraw } = await import("@excalidraw/mermaid-to-excalidraw");
  try {
    return await parseMermaidToExcalidraw(definition);
  } catch (error) {
    // As the dialog does: a second try with double quotes made single.
    return await parseMermaidToExcalidraw(definition.replace(/"/g, "'"));
  }
}

/** {} opens the dialog; {"insert": "<definition>"} converts and inserts. Answers {} or {ids}. */
export async function mermaidCall(arg, { insert }) {
  const api = board.api;
  if (typeof arg.insert !== "string") {
    api.updateScene({ appState: { openDialog: { name: "ttd", tab: "mermaid" } }, captureUpdate: CaptureUpdateAction.NEVER });
    return {};
  }
  const { elements: skeletons, files } = await parse(arg.insert);
  const elements = convertToExcalidrawElements(skeletons, { regenerateIds: true });
  const pictures = Object.values(files ?? {});
  // A diagram Excalidraw does not turn into shapes comes as one SVG picture: posted to the
  // app as a `file` (the autosave does) like a pasted one.
  if (pictures.length) api.addFiles(pictures);
  const ids = await insert(centred(elements, api.getAppState()));
  return { ids };
}
