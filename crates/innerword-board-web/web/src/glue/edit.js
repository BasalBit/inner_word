// Changes the app makes to the canvas: elements it inserts (the Add commands, pictures, a
// Mermaid diagram, an item of the shape library) and an outside write it adopts (an agent's
// change, a restored version), by the base rule (docs/whiteboard-design.md, 4.5).
import { CaptureUpdateAction, restoreElements } from "@excalidraw/excalidraw";
import { markImmediate, postNow, sendStatus } from "./autosave.js";
import { loadFontsFor } from "./fonts.js";
import { fetchPicture } from "./pictures.js";
import { NEVER_KEPT, board, bumped, currentRev, orderByIndex, randomInteger, recordVersions } from "./scene.js";

/** Restores a whole list of elements as a load does (texts measured again in their fonts,
 * labels, bindings and frames mended), in the order of their fractional indices first, so
 * that Excalidraw repairs only the indices that are out of order. Restoring the whole list,
 * not the incoming elements alone, keeps a binding to an element the change did not name. */
function restoredWhole(elements) {
  return restoreElements(orderByIndex(elements), null, { refreshDimensions: true, repairBindings: true });
}

function withNeverKeptDeleted(elements) {
  return elements.map((e) => (!e.isDeleted && NEVER_KEPT.has(e.type) ? bumped(e, { isDeleted: true }) : e));
}

function selection(ids) {
  return Object.fromEntries(ids.map((id) => [id, true]));
}

/**
 * Puts elements on the canvas as one step of the undo: an element whose id the canvas has
 * replaces its copy (at a higher version), the others are added. The new ones are selected
 * (their labels with their shapes). Answers the ids of the new elements.
 */
export async function insertElements(incoming, { select = true, editText = null } = {}) {
  const api = board.api;
  await loadFontsFor(incoming);
  const canvas = api.getSceneElementsIncludingDeleted();
  const byId = new Map(canvas.map((e) => [e.id, e]));
  const replaced = new Map();
  const added = [];
  for (const element of incoming) {
    if (!element || typeof element.id !== "string") continue;
    const mine = byId.get(element.id);
    if (mine) {
      replaced.set(element.id, { ...element, version: Math.max(element.version ?? 1, mine.version + 1), versionNonce: randomInteger() });
    } else {
      // On top of the drawing: Excalidraw gives it an index after the last one.
      added.push({ ...element, index: null });
    }
  }
  const merged = canvas.map((e) => replaced.get(e.id) ?? e).concat(added);
  const restored = withNeverKeptDeleted(restoredWhole(merged));
  const addedIds = new Set(added.map((e) => e.id));
  const newIds = restored.filter((e) => addedIds.has(e.id) && !e.isDeleted).map((e) => e.id);
  const chosen = restored.filter((e) => addedIds.has(e.id) && !e.isDeleted && !(e.type === "text" && e.containerId)).map((e) => e.id);
  markImmediate();
  api.updateScene({
    elements: restored,
    appState: select ? { selectedElementIds: selection(chosen), selectedGroupIds: {} } : undefined,
    captureUpdate: CaptureUpdateAction.IMMEDIATELY,
  });
  if (editText && newIds.includes(editText)) startEditing(editText);
  return newIds;
}

/** Opens a text element (or a shape's label) for typing, as `enter` on it does. */
function startEditing(id) {
  const api = board.api;
  api.updateScene({ appState: { selectedElementIds: { [id]: true } }, captureUpdate: CaptureUpdateAction.NEVER });
  const target = document.querySelector(".excalidraw") ?? document;
  target.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", code: "Enter", bubbles: true, cancelable: true }));
}

/** Adds pictures to the canvas's files, fetched from the source when only their stored id is
 * given; they count as known to the app. */
async function addFiles(files) {
  const api = board.api;
  const adding = [];
  for (const [id, entry] of Object.entries(files ?? {})) {
    if (api.getFiles()[id]?.dataURL) {
      board.filesKnown.add(id);
      continue;
    }
    const dataURL = entry.dataURL ?? (await fetchPicture(entry.file));
    board.filesKnown.add(id);
    if (!dataURL) continue;
    adding.push({ id, mimeType: entry.mimeType, dataURL, created: Number.isFinite(entry.created) ? entry.created : Date.now() });
  }
  if (adding.length) api.addFiles(adding);
}

/**
 * Takes an outside write into the canvas by the base rule (4.5): for each element of the
 * delta, the incoming copy where the writer has not touched the canvas's copy since the
 * scene the change was made on (judged by the versions this page recorded at `baseRev`,
 * not by `baseVersion`: Excalidraw raises versions while it loads), the writer's copy and
 * a conflict otherwise. One updateScene: one step of the undo, or none when `undoable` is
 * false. Answers {rev, same, conflicts}.
 */
export async function adopt(delta) {
  const api = board.api;
  const revBefore = currentRev();
  const nothingUnposted = revBefore === board.posted;
  await addFiles(delta.files);

  const record = board.versionsAt.get(delta.baseRev) ?? null;
  const canvas = api.getSceneElementsIncludingDeleted();
  const byId = new Map(canvas.map((e) => [e.id, e]));
  const untouched = (element) => record !== null && record.has(element.id) && record.get(element.id) === element.version;

  const conflicts = [];
  const replaced = new Map();
  const added = [];
  /** id -> the version the incoming copy must end with */
  const versions = new Map();
  let allTheirs = true;

  for (const { element, baseVersion } of delta.upserted ?? []) {
    if (!element || typeof element.id !== "string") continue;
    if (NEVER_KEPT.has(element.type)) continue;
    const mine = byId.get(element.id);
    const theirs = element.version ?? 1;
    if (!mine) {
      if (baseVersion === null || baseVersion === undefined) {
        added.push(element);
        versions.set(element.id, theirs);
      } else {
        // The writer deleted it and a reload forgot the tombstone.
        conflicts.push(element.id);
      }
      continue;
    }
    const tombstoneOfTheBase = mine.isDeleted && untouched(mine);
    if ((baseVersion === null || baseVersion === undefined) && !tombstoneOfTheBase) {
      // An id the writer gave an element of their own meanwhile.
      conflicts.push(element.id);
      continue;
    }
    if (!untouched(mine)) {
      conflicts.push(element.id);
      continue;
    }
    const version = Math.max(theirs, mine.version + 1);
    if (version !== theirs) allTheirs = false;
    replaced.set(element.id, element);
    versions.set(element.id, version);
  }

  const removed = new Map();
  for (const { id, baseVersion } of delta.removed ?? []) {
    const mine = byId.get(id);
    if (!mine) continue;
    if (!untouched(mine) || baseVersion === null || baseVersion === undefined) {
      if (!mine.isDeleted) conflicts.push(id);
      continue;
    }
    if (mine.isDeleted) continue;
    removed.set(id, bumped(mine, { isDeleted: true }));
    allTheirs = false;
  }

  const merged = canvas.map((e) => replaced.get(e.id) ?? removed.get(e.id) ?? e).concat(added);
  await loadFontsFor([...replaced.values(), ...added]);
  const restored = withNeverKeptDeleted(restoredWhole(merged)).map((e) => {
    if (!versions.has(e.id)) return e;
    const original = replaced.get(e.id) ?? added.find((a) => a.id === e.id);
    return { ...e, version: versions.get(e.id), versionNonce: original?.versionNonce ?? e.versionNonce };
  });

  api.updateScene({
    elements: restored,
    captureUpdate: delta.undoable === false ? CaptureUpdateAction.NEVER : CaptureUpdateAction.IMMEDIATELY,
  });

  const rev = currentRev();
  recordVersions(rev);
  const same = conflicts.length === 0 && allTheirs && nothingUnposted;
  if (same) {
    // The canvas holds exactly what the library holds: nothing to post.
    board.posted = rev;
    sendStatus();
  } else {
    // The merge is a change of the writer's from here: posted once the answer is out.
    markImmediate();
    setTimeout(postNow, 0);
  }
  return { rev, same, conflicts };
}
