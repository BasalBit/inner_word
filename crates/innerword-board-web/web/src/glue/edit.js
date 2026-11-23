// Changes the app makes to the canvas: elements it inserts (the Add commands, pictures, a
// Mermaid diagram, an item of the shape library) and an outside write it adopts (an agent's
// change, a restored version), by the base rule (docs/whiteboard-design.md, 4.5).
//
// Two rules hold for both, because a change of the app's can come while the writer is in the
// middle of a gesture (6.7 lets an agent's change in after a second of patience, with the
// pointer down or a text being edited):
//
// - Every wait comes first (the pictures fetched, the fonts of the incoming texts loaded).
//   Reading the canvas, merging and replacing it are then one stretch without an await, so
//   no gesture, key or deletion of the writer's can come between the read and the write and
//   be replaced by the copy read before it.
// - Only the elements the change names are made anew. Every other element of the canvas
//   stays the very object Excalidraw has: a freehand stroke, a shape being drawn or a text
//   being typed is extended by Excalidraw through its own reference to that object
//   (`mutateElement` on `appState.newElement`), so a copy in its place would freeze the
//   stroke where it was, and restoring it as a load does would remove a stroke that has
//   only just begun (too small to see) or a new text that is still empty. This is what
//   Excalidraw's own collaboration does with a remote change.
import { CaptureUpdateAction, restoreElements } from "@excalidraw/excalidraw";
import { markImmediate, sceneChanged, sendStatus } from "./autosave.js";
import { loadFontsFor } from "./fonts.js";
import { fetchPicture } from "./pictures.js";
import { NEVER_KEPT, board, bumped, currentRev, orderByIndex, randomInteger, recordVersions } from "./scene.js";

/** The ids of the elements an element names: its container, its frame, the two ends of a
 * binding, and the elements bound to it. */
function namedIds(element) {
  const ids = [];
  if (typeof element.containerId === "string") ids.push(element.containerId);
  if (typeof element.frameId === "string") ids.push(element.frameId);
  for (const end of ["startBinding", "endBinding"]) {
    const id = element[end]?.elementId;
    if (typeof id === "string") ids.push(id);
  }
  if (Array.isArray(element.boundElements)) {
    for (const bound of element.boundElements) if (typeof bound?.id === "string") ids.push(bound.id);
  }
  return ids;
}

/**
 * Restores incoming elements as a load does (texts measured again in their fonts, labels,
 * bindings and frames mended, the fields a Rust-made element lacks filled), with no local
 * elements (so no version is raised). Restoring a list mends a binding only to an element
 * in the list, so copies of the canvas's elements that the incoming ones name go through
 * the restore beside them, and are then thrown away: a label keeps a container, an arrow
 * the shape it is bound to, an element its frame, although the change did not name them.
 * Answers id -> restored element; an incoming element the restore drops (too small to see)
 * is not in it.
 */
function restoredIncoming(incoming, canvasById) {
  const incomingIds = new Set(incoming.map((element) => element.id));
  const context = new Map();
  for (const element of incoming) {
    for (const id of namedIds(element)) {
      if (incomingIds.has(id) || context.has(id)) continue;
      const mine = canvasById.get(id);
      if (mine) context.set(id, { ...mine });
    }
  }
  const restored = restoreElements(orderByIndex([...incoming, ...context.values()]), null, {
    refreshDimensions: true,
    repairBindings: true,
  });
  const out = new Map();
  for (const element of restored) if (incomingIds.has(element.id)) out.set(element.id, element);
  return out;
}

function withNeverKeptDeleted(element) {
  return !element.isDeleted && NEVER_KEPT.has(element.type) ? bumped(element, { isDeleted: true }) : element;
}

function selection(ids) {
  return Object.fromEntries(ids.map((id) => [id, true]));
}

/**
 * Puts elements on the canvas as one step of the undo: an element whose id the canvas has
 * replaces its copy (at a higher version), the others are added on top of the drawing. The
 * new ones are selected (their labels with their shapes). Answers the ids of the new
 * elements.
 */
export async function insertElements(incoming, { select = true, editText = null } = {}) {
  const api = board.api;
  const list = (incoming ?? []).filter((element) => element && typeof element.id === "string");
  await loadFontsFor(list);

  // One stretch from here to updateScene: no await.
  const canvas = api.getSceneElementsIncludingDeleted();
  const byId = new Map(canvas.map((element) => [element.id, element]));
  const replacing = new Map();
  const adding = [];
  for (const element of list) {
    const mine = byId.get(element.id);
    if (mine) replacing.set(element.id, Math.max(element.version ?? 1, mine.version + 1));
    else adding.push(element.id);
  }
  const restored = restoredIncoming(list, byId);

  const replaced = new Map();
  for (const [id, version] of replacing) {
    const element = restored.get(id);
    if (element) replaced.set(id, withNeverKeptDeleted({ ...element, version, versionNonce: randomInteger() }));
  }
  // On top of the drawing: without an index, Excalidraw gives each one after the last.
  const added = adding
    .map((id) => restored.get(id))
    .filter(Boolean)
    .map((element) => withNeverKeptDeleted({ ...element, index: null }));

  const incomingIds = new Set(list.map((element) => element.id));
  const merged = orderByIndex(canvas.map((element) => replaced.get(element.id) ?? element).concat(added), incomingIds);
  const newIds = added.filter((e) => !e.isDeleted).map((e) => e.id);
  const chosen = added.filter((e) => !e.isDeleted && !(e.type === "text" && e.containerId)).map((e) => e.id);
  markImmediate();
  api.updateScene({
    elements: merged,
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
 *
 * `same` is true only when the canvas now holds exactly the scene the app stored: no
 * conflict, every element of the delta at the version the app gave it, and the canvas was
 * at `baseRev` itself when it was merged (nothing the writer did since the scene the change
 * was made on). That is judged against `baseRev`, not against the last rev the page posted:
 * a post that leaves the page after the app wrote the outside change is held by the app and
 * dropped (4.6, "After an outside write"), so a change the page counts as posted may be one
 * the library never got. Otherwise the merge is posted as a change of the writer's.
 */
export async function adopt(delta) {
  const api = board.api;
  const upserted = (delta.upserted ?? []).filter(
    (entry) => entry && entry.element && typeof entry.element.id === "string" && !NEVER_KEPT.has(entry.element.type),
  );
  // Every wait first: the pictures, then the fonts of the incoming texts.
  await addFiles(delta.files);
  await loadFontsFor(upserted.map((entry) => entry.element));

  // One stretch from here to the answer: no await.
  const revBefore = currentRev();
  const atTheBase = Number.isFinite(delta.baseRev) && revBefore === delta.baseRev;
  const record = board.versionsAt.get(delta.baseRev) ?? null;
  const canvas = api.getSceneElementsIncludingDeleted();
  const byId = new Map(canvas.map((element) => [element.id, element]));
  const untouched = (element) => record !== null && record.has(element.id) && record.get(element.id) === element.version;

  const conflicts = [];
  /** id -> the incoming element, for the ones taken */
  const taken = new Map();
  /** id -> the version the incoming copy must end with */
  const versions = new Map();
  let allTheirs = true;

  for (const { element, baseVersion } of upserted) {
    const mine = byId.get(element.id);
    const theirs = element.version ?? 1;
    if (!mine) {
      if (baseVersion === null || baseVersion === undefined) {
        taken.set(element.id, element);
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
    taken.set(element.id, element);
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

  const restored = restoredIncoming([...taken.values()], byId);
  const replaced = new Map();
  const added = [];
  for (const [id, element] of taken) {
    const copy = restored.get(id);
    const mine = byId.get(id);
    if (!copy) {
      // Too small to see: Excalidraw drops it as a load would, and so does the canvas.
      allTheirs = false;
      if (mine && !mine.isDeleted) removed.set(id, bumped(mine, { isDeleted: true }));
      continue;
    }
    const final = { ...copy, version: versions.get(id), versionNonce: element.versionNonce ?? copy.versionNonce };
    if (mine) replaced.set(id, final);
    else added.push(final);
  }

  const incomingIds = new Set(taken.keys());
  const merged = orderByIndex(
    canvas.map((element) => replaced.get(element.id) ?? removed.get(element.id) ?? element).concat(added),
    incomingIds,
  );
  api.updateScene({
    elements: merged,
    captureUpdate: delta.undoable === false ? CaptureUpdateAction.NEVER : CaptureUpdateAction.IMMEDIATELY,
  });

  // What Excalidraw holds now: the repair of an order it found out of place (or the restore)
  // may have raised the version of an incoming element, and then the canvas is not S2.
  const now = new Map(api.getSceneElementsIncludingDeleted().map((element) => [element.id, element]));
  for (const [id, element] of taken) {
    if (now.get(id)?.version !== (element.version ?? 1)) allTheirs = false;
  }

  let rev = currentRev();
  const same = conflicts.length === 0 && allTheirs && atTheBase;
  if (!same && rev <= board.posted) {
    // The canvas is not what the library holds, and its rev was posted before the merge
    // (a post the app drops while it adopts): a new rev, so that it is posted again.
    board.rev += 1;
    rev = board.rev;
  }
  recordVersions(rev);
  if (same) {
    // The canvas holds exactly what the library holds: nothing to post.
    board.posted = rev;
    sendStatus();
  } else {
    // The merge is a change of the writer's from here: posted once the answer is out, at the
    // end of a gesture that is under way, or within a second while a text is edited.
    markImmediate();
    setTimeout(sceneChanged, 0);
  }
  return { rev, same, conflicts };
}
