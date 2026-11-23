// Excalidraw's shape library, kept by the app in the library's settings (4.15): served as
// library.json at the load, posted as `library` when it changes, and the `library` call.
import { getLibraryItemsHash } from "@excalidraw/excalidraw";
import { MAX_MESSAGE_BYTES, fitsIn, postText, wellFormed } from "./bridge.js";
import { loadFontsFor } from "./fonts.js";
import { board, boundsOf, randomId, randomInteger, selectedIds, viewCenter } from "./scene.js";

const POST_AFTER_MS = 300;
/** Excalidraw's LIBRARY_DISABLED_TYPES in 0.18.1: what "Add to library" refuses. */
const NOT_IN_A_LIBRARY = ["iframe", "embeddable", "image"];

export const library = {
  items: [],
  hash: null,
  lrev: 0,
  posted: 0,
  timer: null,
};

/** Told whenever `libraryWaiting()` may have changed (the glue sends its status then). */
let waitingChanged = () => {};

/** Who is told when a change of the library starts or stops waiting: the status. */
export function whenLibraryWaitingChanges(tell) {
  waitingChanged = tell;
}

/**
 * The status's `libraryWaiting` (4.6, 4.15): the library changed since the app was last
 * given it. True from Excalidraw's onLibraryChange (or the `library` call) until the
 * `library` message is posted POST_AFTER_MS later, or until `collect` brings it; and for as
 * long as the library is too long for one message (posted as `tooLarge`, never counted as
 * given): `collect` brings it whole. A deliberate end asks `collect` while it is true.
 */
export function libraryWaiting() {
  return library.lrev > library.posted;
}

/** The items a stored library holds, whatever form it was written in. */
export function itemsOfStored(stored) {
  if (!stored || typeof stored !== "object") return [];
  if (Array.isArray(stored.libraryItems)) return stored.libraryItems;
  if (Array.isArray(stored.library)) {
    // Version 1 of the file: a list of lists of elements.
    return stored.library.map((elements, n) => ({ id: `library-item-${n}`, status: "unpublished", elements, created: 0 }));
  }
  return [];
}

/** The library as served: what a first onLibraryChange with the same items is measured against. */
export function servedLibrary(items) {
  library.items = items;
  library.hash = getLibraryItemsHash(items);
}

/** Excalidraw's library changed (onLibraryChange), or a call changed it. */
export function libraryChanged(items) {
  const hash = getLibraryItemsHash(items);
  library.items = items;
  if (hash === library.hash) return;
  library.hash = hash;
  library.lrev++;
  clearTimeout(library.timer);
  library.timer = setTimeout(postLibrary, POST_AFTER_MS);
  waitingChanged();
}

/**
 * Posts the library when it changed since its last post. A library longer than one message
 * is posted as `{"items": null, "tooLarge": true}` (the view would drop the whole message,
 * and the app would never hear of it), so that the app keeps the library it stored and
 * tells the writer it is too large to keep; it does not count as posted, and `collect`
 * still brings it whole.
 */
export function postLibrary() {
  clearTimeout(library.timer);
  library.timer = null;
  if (library.lrev <= library.posted) return;
  const text = wellFormed(JSON.stringify({ v: 1, kind: "library", lrev: library.lrev, items: library.items }));
  if (!fitsIn(text, MAX_MESSAGE_BYTES)) {
    postText(`{"v":1,"kind":"library","lrev":${library.lrev},"items":null,"tooLarge":true}`);
    return;
  }
  if (postText(text)) {
    library.posted = library.lrev;
    waitingChanged();
  }
}

/** For `collect`: the library when it changed since its last post (and it counts as
 * posted; `collect` answers the status after it, `libraryWaiting` false). */
export function collectLibrary() {
  if (library.lrev <= library.posted) return null;
  clearTimeout(library.timer);
  library.timer = null;
  library.posted = library.lrev;
  return wellFormed(JSON.stringify({ lrev: library.lrev, items: library.items }));
}

function text(key, fallback) {
  try {
    const said = board.t?.(key);
    return said && said !== key ? said : fallback;
  } catch {
    return fallback;
  }
}

/** The selection as one library item, as Excalidraw's "Add to library" makes it: the
 * selected elements, the labels of the selected shapes and arrows, the contents of the
 * selected frames. */
function itemOfSelection() {
  const api = board.api;
  const all = api.getSceneElements();
  const chosen = new Set(selectedIds(api.getAppState(), all));
  const frames = new Set(all.filter((e) => chosen.has(e.id) && (e.type === "frame" || e.type === "magicframe")).map((e) => e.id));
  for (const e of all) if (e.frameId && frames.has(e.frameId)) chosen.add(e.id);
  for (const e of all) if (e.type === "text" && e.containerId && chosen.has(e.containerId)) chosen.add(e.id);
  const elements = all.filter((e) => chosen.has(e.id));
  return elements;
}

/** A copy of a library item's elements with new ids (and the references among them),
 * new seeds and no index, as Excalidraw puts an item on the canvas. */
export function duplicateElements(elements) {
  const ids = new Map(elements.map((e) => [e.id, randomId()]));
  const groups = new Map();
  const group = (id) => {
    if (!groups.has(id)) groups.set(id, randomId());
    return groups.get(id);
  };
  const now = Date.now();
  return elements.map((e) => {
    const copy = JSON.parse(JSON.stringify(e));
    copy.id = ids.get(e.id);
    copy.seed = randomInteger();
    copy.version = 1;
    copy.versionNonce = randomInteger();
    copy.updated = now;
    copy.isDeleted = false;
    copy.index = null;
    copy.groupIds = (e.groupIds ?? []).map(group);
    if (e.containerId) copy.containerId = ids.get(e.containerId) ?? null;
    if (e.frameId) copy.frameId = ids.get(e.frameId) ?? null;
    if (Array.isArray(e.boundElements)) {
      copy.boundElements = e.boundElements.filter((b) => ids.has(b.id)).map((b) => ({ ...b, id: ids.get(b.id) }));
    }
    for (const end of ["startBinding", "endBinding"]) {
      if (e[end]) copy[end] = ids.has(e[end].elementId) ? { ...e[end], elementId: ids.get(e[end].elementId) } : null;
    }
    return copy;
  });
}

/** Elements centred on the middle of the view. */
export function centred(elements, appState) {
  const bounds = boundsOf(elements);
  if (!bounds) return elements;
  const middle = viewCenter(appState);
  const dx = middle.x - (bounds[0] + bounds[2]) / 2;
  const dy = middle.y - (bounds[1] + bounds[3]) / 2;
  return elements.map((e) => ({ ...e, x: e.x + dx, y: e.y + dy }));
}

/**
 * The `library` call: {"show"|"hide"|"toggle"|"addSelection"|"insert": …}. Answers
 * {lrev, added, refused, ids}: `added` is the item an addSelection made, `ids` the
 * elements an insert put on the canvas.
 */
export async function libraryCall(arg, { insert }) {
  const api = board.api;
  let added = null;
  let refused = null;
  let ids = [];
  if (arg.show || arg.hide || arg.toggle) {
    const force = arg.show ? true : arg.hide ? false : undefined;
    api.toggleSidebar({ name: "default", tab: "library", force });
  } else if (arg.addSelection) {
    const elements = itemOfSelection();
    const banned = NOT_IN_A_LIBRARY.find((type) => elements.some((e) => e.type === type));
    if (!elements.length) {
      refused = "Select the shapes to add to the shape library first.";
    } else if (banned) {
      refused = text(`errors.libraryElementTypeError.${banned}`, `${banned} elements cannot be added to the library.`);
    } else {
      const item = {
        id: randomId(),
        status: "unpublished",
        elements: elements.map((e) => JSON.parse(JSON.stringify(e))),
        created: Date.now(),
      };
      const items = [item, ...library.items];
      await api.updateLibrary({ libraryItems: items, merge: false, openLibraryMenu: false });
      libraryChanged(items);
      added = item.id;
      api.setToast({ message: text("toast.addedToLibrary", "Added to library") });
    }
  } else if (typeof arg.insert === "string") {
    const item = library.items.find((i) => i.id === arg.insert);
    if (!item) {
      refused = "That item is not in the shape library any more.";
    } else {
      const copies = centred(duplicateElements(item.elements ?? []), api.getAppState());
      await loadFontsFor(copies);
      ids = await insert(copies);
    }
  }
  return { lrev: library.lrev, added, refused, ids };
}
