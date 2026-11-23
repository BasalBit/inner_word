// What the page knows of its drawing: Excalidraw's API, the revs of the drawing in this view,
// what was posted, the versions of the elements at each posted rev (the base rule of adopt
// judges "untouched" by them), and the scene as the app is given it (4.4 to 4.6).
import { hashElementsVersion } from "@excalidraw/excalidraw";
import { wellFormed } from "./bridge.js";

/** Kinds of element a whiteboard never keeps: web embeds and AI frames (4.13). */
export const NEVER_KEPT = new Set(["iframe", "embeddable", "magicframe"]);

/** How many posted or adopted revs keep their element versions. */
const VERSIONS_KEPT = 8;

let resolveReady;

export const board = {
  /** Excalidraw's imperative API, once it is mounted. */
  api: null,
  /** Resolves when the scene is on the canvas and `ready` was posted. */
  ready: new Promise((resolve) => {
    resolveReady = resolve;
  }),
  isReady: false,
  /** The ContentStamp of the stored scene this view loaded. */
  stamp: "",
  /** The state of the drawing: a new rev for every state the change detector sees. */
  rev: 0,
  signature: null,
  /** The last rev the app was given (a `scene` post, or a `collect`). */
  posted: 0,
  /** rev -> Map(element id -> version on the canvas when that rev was posted or adopted). */
  versionsAt: new Map(),
  /** Excalidraw file ids the app has the bytes of (stored, posted, or given by the app). */
  filesKnown: new Set(),
  pointerDown: false,
  editing: false,
  composing: false,
  tooLarge: false,
  /** Excalidraw's own `t`, for its own words (I18nBridge). */
  t: null,
};

export function markReady(stamp) {
  board.stamp = stamp ?? "";
  board.isReady = true;
  resolveReady(true);
}

function signatureOf(elements, appState, files) {
  const ids = Object.keys(files ?? {}).sort().join(",");
  return [
    hashElementsVersion(elements),
    elements.length,
    appState.viewBackgroundColor,
    appState.gridSize,
    appState.gridStep,
    appState.gridModeEnabled,
    ids,
  ].join("|");
}

/** The rev of the drawing as it is now: a new one when it changed since the last look. */
export function currentRev(elements, appState, files) {
  const api = board.api;
  if (!api) return board.rev;
  const signature = signatureOf(
    elements ?? api.getSceneElementsIncludingDeleted(),
    appState ?? api.getAppState(),
    files ?? api.getFiles(),
  );
  if (signature !== board.signature) {
    if (board.signature !== null) board.rev++;
    board.signature = signature;
  }
  return board.rev;
}

/** Keeps every element's version on the canvas as of `rev` (deleted ones included). */
export function recordVersions(rev) {
  const versions = new Map();
  for (const element of board.api.getSceneElementsIncludingDeleted()) versions.set(element.id, element.version);
  board.versionsAt.delete(rev);
  board.versionsAt.set(rev, versions);
  while (board.versionsAt.size > VERSIONS_KEPT) board.versionsAt.delete(board.versionsAt.keys().next().value);
}

function isLinear(element) {
  return element.type === "line" || element.type === "arrow";
}

/** The four canvas settings that belong to the drawing (what serializeAsJSON keeps for a
 * database). */
export function drawingAppState(appState) {
  return {
    viewBackgroundColor: appState.viewBackgroundColor,
    gridSize: appState.gridSize,
    gridStep: appState.gridStep,
    gridModeEnabled: appState.gridModeEnabled,
  };
}

/** The image elements of a list that are not deleted, and the file ids they name. */
export function namedFileIds(elements) {
  const ids = [];
  const seen = new Set();
  for (const element of elements) {
    if (element.isDeleted || element.type !== "image" || !element.fileId || seen.has(element.fileId)) continue;
    seen.add(element.fileId);
    ids.push(element.fileId);
  }
  return ids;
}

/**
 * The scene as the app is given it: Excalidraw's envelope, the elements that are not deleted
 * (as serializeAsJSON writes them for a database), the drawing's canvas settings, and a
 * `files` entry for each picture the canvas has, without its bytes (the app has those).
 */
export function sceneJson() {
  const api = board.api;
  const elements = api
    .getSceneElements()
    .filter((element) => !NEVER_KEPT.has(element.type))
    .map((element) => (isLinear(element) && element.lastCommittedPoint ? { ...element, lastCommittedPoint: null } : element));
  const all = api.getFiles();
  const files = {};
  for (const id of namedFileIds(elements)) {
    const file = all[id];
    if (file) files[id] = { mimeType: file.mimeType, created: file.created };
  }
  return wellFormed(
    JSON.stringify({
      type: "excalidraw",
      version: 2,
      source: "inner-word",
      elements,
      appState: drawingAppState(api.getAppState()),
      files,
    }),
  );
}

/** The elements of a list with the kinds never kept marked deleted, or null when it has none. */
export function withoutNeverKept(elements) {
  if (!elements.some((element) => !element.isDeleted && NEVER_KEPT.has(element.type))) return null;
  return elements.map((element) =>
    !element.isDeleted && NEVER_KEPT.has(element.type) ? bumped(element, { isDeleted: true }) : element,
  );
}

/** A copy of an element with `changes`, its version raised by one, as Excalidraw's
 * newElementWith does. */
export function bumped(element, changes = {}) {
  return {
    ...element,
    ...changes,
    version: (element.version ?? 0) + 1,
    versionNonce: randomInteger(),
    updated: Date.now(),
  };
}

export function randomInteger() {
  return Math.floor(Math.random() * 2 ** 31);
}

const ID_ALPHABET = "0123456789ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz_-";

/** A new id in Excalidraw's form (21 characters of nanoid's alphabet). */
export function randomId() {
  const bytes = crypto.getRandomValues(new Uint8Array(21));
  let id = "";
  for (const byte of bytes) id += ID_ALPHABET[byte & 63];
  return id;
}

/** Elements ordered by fractional index as Excalidraw orders them (ties by id); an element
 * without an index keeps its place after those that have one. */
export function orderByIndex(elements) {
  const indexed = [];
  const rest = [];
  for (const element of elements) (typeof element.index === "string" ? indexed : rest).push(element);
  indexed.sort((a, b) => (a.index < b.index ? -1 : a.index > b.index ? 1 : a.id < b.id ? -1 : a.id > b.id ? 1 : 0));
  return indexed.concat(rest);
}

/** The middle of the view in scene coordinates. */
export function viewCenter(appState) {
  return {
    x: appState.width / 2 / appState.zoom.value - appState.scrollX,
    y: appState.height / 2 / appState.zoom.value - appState.scrollY,
  };
}

/** A point of the view (CSS pixels from its top left) in scene coordinates. */
export function sceneAt(appState, x, y) {
  return {
    x: (x - appState.offsetLeft) / appState.zoom.value - appState.scrollX,
    y: (y - appState.offsetTop) / appState.zoom.value - appState.scrollY,
  };
}

/** The ids selected now, in the order they were selected. */
export function selectedIds(appState, elements) {
  const live = new Set(elements.filter((e) => !e.isDeleted).map((e) => e.id));
  return Object.keys(appState.selectedElementIds ?? {}).filter((id) => appState.selectedElementIds[id] && live.has(id));
}

/** The bounds of elements: [minX, minY, maxX, maxY], or null for none. */
export function boundsOf(elements) {
  let minX = Infinity;
  let minY = Infinity;
  let maxX = -Infinity;
  let maxY = -Infinity;
  for (const e of elements) {
    if (e.isDeleted) continue;
    const xs = [e.x, e.x + (e.width ?? 0)];
    const ys = [e.y, e.y + (e.height ?? 0)];
    if (Array.isArray(e.points) && e.points.length) {
      for (const [px, py] of e.points) {
        xs.push(e.x + px);
        ys.push(e.y + py);
      }
    }
    minX = Math.min(minX, ...xs);
    minY = Math.min(minY, ...ys);
    maxX = Math.max(maxX, ...xs);
    maxY = Math.max(maxY, ...ys);
  }
  return Number.isFinite(minX) ? [minX, minY, maxX, maxY] : null;
}

/** Elements moved by (dx, dy). */
export function moved(elements, dx, dy) {
  return elements.map((e) => ({ ...e, x: e.x + dx, y: e.y + dy }));
}
