// When the page posts its scene, and its status (docs/whiteboard-design.md, 4.4 and 4.6).
//
// At once at the end of a gesture (the pointer released, a text's editing ended, a paste, an
// undo or redo, a deletion, a call that inserts); 150 ms after the last change made with
// another key (holding an arrow key to nudge posts once); at most once a second while a text
// is edited; never while the pointer is down; at once when the page is hidden or goes.
import { MAX_VIEW_SCENE_BYTES, fitsIn, post, postFile, postSceneText } from "./bridge.js";
import { board, currentRev, namedFileIds, recordVersions, sceneJson, selectedIds } from "./scene.js";
import { libraryWaiting } from "./library.js";

const KEY_QUIET_MS = 150;
const EDITING_EVERY_MS = 1000;
const VIEW_STATUS_EVERY_MS = 250;
/** How long a post waits for a picture Excalidraw is still reading (a paste), at most. */
const PICTURE_PATIENCE_MS = 5000;

let keyTimer = null;
let editTimer = null;
let pictureTimer = null;
let pictureWaitSince = 0;
let lastEditPost = 0;
/** Until this time a change is the end of a gesture: posted at once. */
let immediateUntil = 0;

export function markImmediate(ms = 400) {
  immediateUntil = Math.max(immediateUntil, Date.now() + ms);
}

function clearTimers() {
  clearTimeout(keyTimer);
  clearTimeout(editTimer);
  clearTimeout(pictureTimer);
  keyTimer = editTimer = pictureTimer = null;
}

/** Posts a `file` message for each picture the scene names that the app does not have and
 * the canvas has the bytes of; the ids it could not post. A picture whose message would be
 * longer than the view takes is not posted (`filesTooLarge`), and not tried again. */
export function postNewFiles(elements) {
  const files = board.api.getFiles();
  const missing = [];
  for (const id of namedFileIds(elements)) {
    if (board.filesKnown.has(id)) continue;
    const file = files[id];
    if (!file?.dataURL || board.filesTooLarge.has(id)) {
      missing.push(id);
      continue;
    }
    const posted = postFile(id, file.mimeType, file.dataURL);
    if (posted === "posted") board.filesKnown.add(id);
    else {
      if (posted === "tooLarge") board.filesTooLarge.add(id);
      missing.push(id);
    }
  }
  return missing;
}

/** Whether a picture the drawing names is still being read by Excalidraw (a paste). */
function pictureOnItsWay(elements) {
  const files = board.api.getFiles();
  return elements.some((e) => !e.isDeleted && e.type === "image" && e.status === "pending" && e.fileId && !files[e.fileId]?.dataURL);
}

/**
 * Posts the scene now when the app has not been given this rev. A picture Excalidraw is
 * still reading (a paste) is waited for, up to five seconds, unless `force`: the page is
 * being hidden or going, and then the scene goes as it is (the app's `files` call asks for
 * a picture it names that the app has not had).
 */
export function postNow({ force = false } = {}) {
  clearTimers();
  if (!board.isReady) return;
  const rev = currentRev();
  if (rev <= board.posted) {
    sendStatus();
    return;
  }
  const elements = board.api.getSceneElementsIncludingDeleted();
  if (!force && pictureOnItsWay(elements)) {
    if (!pictureWaitSince) pictureWaitSince = Date.now();
    if (Date.now() - pictureWaitSince < PICTURE_PATIENCE_MS) {
      pictureTimer = setTimeout(() => postNow(), 100);
      return;
    }
  }
  pictureWaitSince = 0;
  postNewFiles(elements);
  const json = sceneJson();
  if (!fitsIn(json, MAX_VIEW_SCENE_BYTES)) {
    board.tooLarge = true;
    sendStatus();
    return;
  }
  board.tooLarge = false;
  postSceneText(rev, json);
  board.posted = rev;
  recordVersions(rev);
  if (board.editing) lastEditPost = Date.now();
  sendStatus();
}

/** Called for every change Excalidraw reports (after the rev was worked out). */
export function sceneChanged() {
  if (!board.isReady || board.rev <= board.posted) return;
  if (board.pointerDown) return;
  if (board.editing) {
    if (editTimer) return;
    const wait = Math.max(0, lastEditPost + EDITING_EVERY_MS - Date.now());
    editTimer = setTimeout(() => {
      editTimer = null;
      postNow();
    }, wait);
    return;
  }
  if (Date.now() < immediateUntil) {
    postNow();
    return;
  }
  clearTimeout(keyTimer);
  keyTimer = setTimeout(() => {
    keyTimer = null;
    postNow();
  }, KEY_QUIET_MS);
}

/** Text editing began or ended (Excalidraw's editingTextElement). */
export function editingChanged(editing) {
  if (board.editing === editing) return;
  board.editing = editing;
  if (!editing) {
    markImmediate();
    postNow();
  } else {
    lastEditPost = Date.now();
    sendStatus();
  }
}

// ---------------------------------------------------------------------------------------
// The status.

export function statusNow() {
  const api = board.api;
  const appState = api ? api.getAppState() : null;
  const elements = api ? api.getSceneElementsIncludingDeleted() : [];
  return {
    rev: api ? currentRev() : board.rev,
    posted: board.posted,
    editing: board.editing,
    composing: board.composing,
    pointer: board.pointerDown ? "down" : "up",
    zoom: appState ? appState.zoom.value : 1,
    scrollX: appState ? appState.scrollX : 0,
    scrollY: appState ? appState.scrollY : 0,
    elements: elements.filter((e) => !e.isDeleted).length,
    selected: appState ? selectedIds(appState, elements).length : 0,
    tooLarge: board.tooLarge,
    libraryWaiting: libraryWaiting(),
  };
}

let lastStatusKey = "";
let lastViewKey = "";
let lastViewPost = 0;
let statusTimer = null;

/** Posts the status when it changed: at once, except a change of only the zoom, the scroll,
 * or the rev while the pointer is down, which goes at most every 250 ms. */
export function sendStatus() {
  if (!board.isReady) return;
  const status = statusNow();
  const key = [status.posted, status.editing, status.composing, status.pointer, status.elements, status.selected, status.tooLarge, status.libraryWaiting].join("|")
    + (status.pointer === "up" ? `|${status.rev}` : "");
  const viewKey = `${status.zoom}|${status.scrollX}|${status.scrollY}|${status.rev}`;
  if (key !== lastStatusKey) {
    clearTimeout(statusTimer);
    statusTimer = null;
    lastStatusKey = key;
    lastViewKey = viewKey;
    lastViewPost = Date.now();
    post("status", status);
    return;
  }
  if (viewKey === lastViewKey || statusTimer) return;
  const wait = Math.max(0, lastViewPost + VIEW_STATUS_EVERY_MS - Date.now());
  statusTimer = setTimeout(() => {
    statusTimer = null;
    const now = statusNow();
    lastViewKey = `${now.zoom}|${now.scrollX}|${now.scrollY}|${now.rev}`;
    lastViewPost = Date.now();
    post("status", now);
  }, wait);
}

// ---------------------------------------------------------------------------------------
// The pointer, the keys and the page's own end.

const IMMEDIATE_KEYS = new Set(["Backspace", "Delete"]);

export function installGestureTracking({ onHidden }) {
  window.addEventListener(
    "pointerdown",
    () => {
      board.pointerDown = true;
      sendStatus();
    },
    true,
  );
  const up = () => {
    if (!board.pointerDown) return;
    board.pointerDown = false;
    markImmediate();
    // After Excalidraw has finished the gesture (its own handlers of this event).
    setTimeout(() => {
      if (currentRev() > board.posted && !board.editing) postNow();
      else sendStatus();
    }, 0);
  };
  window.addEventListener("pointerup", up, true);
  window.addEventListener("pointercancel", up, true);
  window.addEventListener("blur", up);
  window.addEventListener(
    "keydown",
    (event) => {
      const undo = event.metaKey && (event.key === "z" || event.key === "Z");
      if (undo || IMMEDIATE_KEYS.has(event.key) || (event.metaKey && (event.key === "d" || event.key === "x"))) markImmediate();
    },
    true,
  );
  window.addEventListener("paste", () => markImmediate(1000), true);
  window.addEventListener("drop", () => markImmediate(1000), true);
  document.addEventListener(
    "compositionstart",
    () => {
      board.composing = true;
      sendStatus();
    },
    true,
  );
  document.addEventListener(
    "compositionend",
    () => {
      board.composing = false;
      sendStatus();
    },
    true,
  );
  // At once, whatever is on its way: a timer set now may never run.
  document.addEventListener("visibilitychange", () => {
    if (document.visibilityState === "hidden") {
      postNow({ force: true });
      onHidden();
    }
  });
  window.addEventListener("pagehide", () => {
    postNow({ force: true });
    onHidden();
  });
}
