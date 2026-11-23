// What the app asks the page (docs/whiteboard-design.md, 4.5): `window.innerwordBoard.call`,
// run by WebKit's callAsyncJavaScript in the page world with one argument, the JSON text
// `arg`. Calls run one after another, in the order the app made them; each answers JSON text.
// `window.innerwordBoard` cannot be replaced, and the whiteboard's policy lets no script but
// the bundle's run in this world.
import { CaptureUpdateAction, convertToExcalidrawElements } from "@excalidraw/excalidraw";
import { markImmediate, postNow, sendStatus, statusNow } from "./autosave.js";
import { post, postSceneText, wellFormed, fitsIn, PART_BYTES } from "./bridge.js";
import { adopt, insertElements } from "./edit.js";
import { KEYS_THE_WINDOW_TAKES } from "./keys.js";
import { collectLibrary, libraryCall } from "./library.js";
import { mermaidCall } from "./mermaid.js";
import { board, currentRev, namedFileIds, recordVersions, sceneAt, sceneJson, selectedIds, viewCenter } from "./scene.js";

const VERSION = "0.18.1";

let queue = Promise.resolve();

/** Defines window.innerwordBoard with the calls of this page. `needsReady`: the calls wait
 * until the page posted `ready` (all but `state`). */
export function defineCalls(calls, { needsReady }) {
  const call = (name, argText) => {
    const run = async () => {
      const fn = Object.prototype.hasOwnProperty.call(calls, name) ? calls[name] : null;
      if (!fn) throw new Error(`the whiteboard has no call named ${name}`);
      const arg = argText === undefined || argText === null || argText === "" ? {} : typeof argText === "string" ? JSON.parse(argText) : argText;
      if (needsReady && name !== "state") await board.ready;
      const answer = await fn(arg ?? {});
      return typeof answer === "string" ? answer : wellFormed(JSON.stringify(answer ?? {}));
    };
    const result = queue.then(run, run);
    queue = result.then(
      () => undefined,
      () => undefined,
    );
    return result;
  };
  const surface = Object.freeze({
    call,
    excalidraw: VERSION,
    keysTheWindowTakes: Object.freeze([...KEYS_THE_WINDOW_TAKES]),
    /** Excalidraw's API, for the WebKit tests and the scripted `board:js:` step only. */
    get api() {
      return board.api;
    },
  });
  Object.defineProperty(window, "innerwordBoard", { value: surface, writable: false, configurable: false, enumerable: false });
}

function editingTextarea() {
  const active = document.activeElement;
  return active instanceof HTMLTextAreaElement && active.classList.contains("excalidraw-wysiwyg") ? active : null;
}

/** A key of Excalidraw's own, dispatched to its container (0.18.1 has no public undo). */
function dispatchCommandKey(key, code, shift = false) {
  const target = document.querySelector(".excalidraw") ?? document;
  target.dispatchEvent(new KeyboardEvent("keydown", { key, code, metaKey: true, shiftKey: shift, bubbles: true, cancelable: true }));
}

function clampZoom(value) {
  return Math.min(30, Math.max(0.1, Math.round(value * 1e6) / 1e6));
}

/** Excalidraw's getStateForZoom: the middle of the view stays where it is. */
function zoomedAt(appState, nextZoom) {
  const x = appState.width / 2;
  const y = appState.height / 2;
  const current = appState.zoom.value;
  return {
    scrollX: appState.scrollX + (x - x / current) - (x - x / nextZoom),
    scrollY: appState.scrollY + (y - y / current) - (y - y / nextZoom),
    zoom: { value: nextZoom },
  };
}

async function waitFor(find, ms) {
  const until = Date.now() + ms;
  for (;;) {
    const found = find();
    if (found || Date.now() > until) return found ?? null;
    await new Promise((resolve) => setTimeout(resolve, 16));
  }
}

function setInputValue(input, value) {
  const setter = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value").set;
  setter.call(input, value);
  input.dispatchEvent(new Event("input", { bubbles: true }));
}

const PICTURE_GAP = 20;

export const boardCalls = {
  /** Everything the app has not been posted yet; never waits on a frame or a timer. */
  collect() {
    const api = board.api;
    const rev = currentRev();
    const elements = api.getSceneElementsIncludingDeleted();
    const all = api.getFiles();
    const files = [];
    for (const id of namedFileIds(elements)) {
      if (board.filesKnown.has(id) || !all[id]?.dataURL) continue;
      files.push({ id, mimeType: all[id].mimeType, dataURL: all[id].dataURL });
      board.filesKnown.add(id);
    }
    let scene = "null";
    if (rev > board.posted) {
      const json = sceneJson();
      if (fitsIn(json, PART_BYTES)) {
        scene = json;
      } else {
        postSceneText(rev, json);
        scene = '"parts"';
      }
      board.posted = rev;
      board.tooLarge = false;
      recordVersions(rev);
    }
    const library = collectLibrary() ?? "null";
    const status = statusNow();
    queueMicrotask(sendStatus);
    return `{"rev":${rev},"scene":${scene},"files":${wellFormed(JSON.stringify(files))},"library":${library},"status":${wellFormed(JSON.stringify(status))}}`;
  },

  /** Posts a `file` message for each id the canvas has the bytes of. */
  files({ ids }) {
    const all = board.api.getFiles();
    const posted = [];
    const missing = [];
    for (const id of ids ?? []) {
      const file = all[id];
      if (file?.dataURL && post("file", { id, mimeType: file.mimeType, dataURL: file.dataURL })) {
        board.filesKnown.add(id);
        posted.push(id);
      } else {
        missing.push(id);
      }
    }
    return { posted, missing };
  },

  adopt(delta) {
    return adopt(delta);
  },

  undo({ steps = 1 }) {
    for (let i = 0; i < steps; i++) {
      if (editingTextarea()) document.execCommand("undo");
      else dispatchCommandKey("z", "KeyZ");
    }
    markImmediate();
    postNow();
    return { rev: currentRev() };
  },

  redo({ steps = 1 }) {
    for (let i = 0; i < steps; i++) {
      if (editingTextarea()) document.execCommand("redo");
      else dispatchCommandKey("z", "KeyZ", true);
    }
    markImmediate();
    postNow();
    return { rev: currentRev() };
  },

  selectAll() {
    const textarea = editingTextarea();
    if (textarea) textarea.select();
    else dispatchCommandKey("a", "KeyA");
    return {};
  },

  zoom(arg) {
    const api = board.api;
    if (arg.fit) {
      if (api.getSceneElements().length) api.scrollToContent(undefined, { fitToContent: true, animate: false });
      return { zoom: api.getAppState().zoom.value };
    }
    const appState = api.getAppState();
    let next = appState.zoom.value;
    if (arg.in) next += 0.1;
    else if (arg.out) next -= 0.1;
    else if (typeof arg.to === "number" && Number.isFinite(arg.to)) next = arg.to;
    next = clampZoom(next);
    api.updateScene({ appState: zoomedAt(appState, next), captureUpdate: CaptureUpdateAction.NEVER });
    queueMicrotask(sendStatus);
    return { zoom: next };
  },

  async search(arg) {
    const api = board.api;
    api.toggleSidebar({ name: "default", tab: "search", force: true });
    const input = await waitFor(() => document.querySelector(".layer-ui__search-inputWrapper input"), 1000);
    if (input) {
      if (typeof arg.query === "string") setInputValue(input, arg.query);
      input.focus();
      input.select?.();
    }
    return {};
  },

  center() {
    const api = board.api;
    const appState = api.getAppState();
    const middle = viewCenter(appState);
    return { x: middle.x, y: middle.y, selected: selectedIds(appState, api.getSceneElementsIncludingDeleted()) };
  },

  /** Pictures the app stored (Insert > Image…, a drop): each at its size, side by side,
   * centred on the point (the view's CSS pixels) or the middle of the view. */
  async insertPictures({ pictures, at }) {
    const api = board.api;
    const appState = api.getAppState();
    const list = (pictures ?? []).filter((p) => p && typeof p.file === "string" && typeof p.dataURL === "string");
    if (!list.length) return { ids: [] };
    api.addFiles(list.map((p) => ({ id: p.file, mimeType: p.mimeType, dataURL: p.dataURL, created: Date.now() })));
    for (const p of list) board.filesKnown.add(p.file);
    const middle = at && Number.isFinite(at.x) && Number.isFinite(at.y) ? sceneAt(appState, at.x, at.y) : viewCenter(appState);
    const total = list.reduce((sum, p) => sum + p.width, 0) + PICTURE_GAP * (list.length - 1);
    let x = middle.x - total / 2;
    const skeletons = list.map((p) => {
      const skeleton = { type: "image", fileId: p.file, status: "saved", scale: [1, 1], x, y: middle.y - p.height / 2, width: p.width, height: p.height };
      x += p.width + PICTURE_GAP;
      return skeleton;
    });
    const elements = convertToExcalidrawElements(skeletons, { regenerateIds: true }).map((e) => ({ ...e, status: "saved" }));
    const ids = await insertElements(elements);
    return { ids };
  },

  /** Elements innerword_board made (the Add commands, the scripted board:draw:). */
  async insertElements({ elements, select = true, editText = null }) {
    const ids = await insertElements(elements ?? [], { select, editText });
    return { ids };
  },

  mermaid(arg) {
    return mermaidCall(arg, { insert: (elements) => insertElements(elements) });
  },

  library(arg) {
    return libraryCall(arg, { insert: (elements) => insertElements(elements) });
  },

  state() {
    return statusNow();
  },
};
