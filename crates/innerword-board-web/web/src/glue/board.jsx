// The whiteboard's page (index.html): Excalidraw mounted with the stored scene, and the glue
// around it (docs/whiteboard-design.md, 4.3 to 4.15).
import React, { useCallback, useEffect, useState } from "react";
import { createRoot } from "react-dom/client";
import {
  CaptureUpdateAction,
  Excalidraw,
  MainMenu,
  WelcomeScreen,
  restore,
  restoreLibraryItems,
  useI18n,
} from "@excalidraw/excalidraw";
import { editingChanged, installGestureTracking, sceneChanged, sendStatus } from "./autosave.js";
import { post, reportError } from "./bridge.js";
import { boardCalls, defineCalls } from "./calls.js";
import { loadFontsFor } from "./fonts.js";
import { installKeys, watchTheInterface } from "./keys.js";
import { itemsOfStored, libraryChanged, postLibrary, servedLibrary, whenLibraryWaitingChanges } from "./library.js";
import { onLinkOpen, withKeptLinks } from "./links.js";
import { loadPictures } from "./pictures.js";
import { NEVER_KEPT, board, currentRev, markReady, recordVersions, withoutNeverKept } from "./scene.js";

const EXCALIDRAW_VERSION = "0.18.1";

/** Dialogs of Excalidraw's that the whiteboard leaves out: closed as soon as they open. */
const DIALOGS_LEFT_OUT = new Set(["commandPalette", "imageExport", "jsonExport"]);

async function fetchJson(path) {
  try {
    const response = await fetch(path);
    if (!response.ok) return null;
    return await response.json();
  } catch {
    return null;
  }
}

function prefersDark() {
  return window.matchMedia?.("(prefers-color-scheme: dark)").matches ?? false;
}

/** The file id of a picture Excalidraw takes in (a paste): the first 16 bytes of the
 * SHA-256 of the file as it came, as hex. */
async function generateIdForFile(file) {
  const digest = await crypto.subtle.digest("SHA-256", await file.arrayBuffer());
  return [...new Uint8Array(digest).slice(0, 16)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

function onPaste(data) {
  // Web embeds never reach the canvas from the clipboard (Excalidraw JSON any site can put there).
  // Nor a link the whiteboard never keeps (a `file:` or an app's own address).
  if (Array.isArray(data?.elements)) data.elements = withKeptLinks(data.elements.filter((e) => !NEVER_KEPT.has(e?.type)));
  return true;
}

/** Excalidraw's `t`, for its own words (the refusal of "Add to library", the embed's label). */
function I18nBridge() {
  const { t } = useI18n();
  board.t = t;
  return null;
}

function openSearch() {
  board.api?.toggleSidebar({ name: "default", tab: "search", force: true });
}

function BoardMainMenu() {
  return (
    <MainMenu>
      <MainMenu.Item onSelect={openSearch}>Find on Whiteboard</MainMenu.Item>
      <MainMenu.Item onSelect={() => post("help")}>Keyboard Shortcuts</MainMenu.Item>
      <MainMenu.DefaultItems.ClearCanvas />
      <MainMenu.Separator />
      <MainMenu.DefaultItems.ChangeCanvasBackground />
    </MainMenu>
  );
}

function BoardWelcomeScreen() {
  return (
    <WelcomeScreen>
      <WelcomeScreen.Hints.ToolbarHint />
      <WelcomeScreen.Hints.HelpHint />
      <WelcomeScreen.Center>
        <WelcomeScreen.Center.Heading>Pick a tool and draw. The whiteboard keeps every stroke as you go.</WelcomeScreen.Center.Heading>
      </WelcomeScreen.Center>
    </WelcomeScreen>
  );
}

let lastOpenDialog = null;

function onChange(elements, appState, files) {
  if (!board.api) return;
  const dialog = appState.openDialog?.name ?? null;
  if (dialog !== lastOpenDialog) {
    lastOpenDialog = dialog;
    if (dialog === "help") {
      post("help");
      queueMicrotask(() => board.api.updateScene({ appState: { openDialog: null }, captureUpdate: CaptureUpdateAction.NEVER }));
    } else if (DIALOGS_LEFT_OUT.has(dialog)) {
      queueMicrotask(() => board.api.updateScene({ appState: { openDialog: null }, captureUpdate: CaptureUpdateAction.NEVER }));
    }
  }
  if (!board.isReady) return;
  // The kinds never kept go the moment they are on the canvas (a paste, an undo).
  const cleaned = withoutNeverKept(elements);
  if (cleaned) queueMicrotask(() => board.api.updateScene({ elements: cleaned, captureUpdate: CaptureUpdateAction.NEVER }));
  editingChanged(appState.editingTextElement != null);
  currentRev(elements, appState, files);
  sceneChanged();
  sendStatus();
}

function App({ initial }) {
  const [theme, setTheme] = useState(prefersDark() ? "dark" : "light");
  useEffect(() => {
    const query = window.matchMedia?.("(prefers-color-scheme: dark)");
    if (!query) return undefined;
    const follow = () => setTheme(query.matches ? "dark" : "light");
    query.addEventListener("change", follow);
    return () => query.removeEventListener("change", follow);
  }, []);
  const onApi = useCallback((api) => {
    if (!board.api) board.api = api;
  }, []);
  return (
    <div className="innerword-board">
      <Excalidraw
        excalidrawAPI={onApi}
        initialData={initial}
        onChange={onChange}
        onLibraryChange={libraryChanged}
        onPaste={onPaste}
        onLinkOpen={onLinkOpen}
        generateIdForFile={generateIdForFile}
        theme={theme}
        langCode="en"
        aiEnabled={false}
        validateEmbeddable={false}
        handleKeyboardGlobally={true}
        autoFocus={true}
        UIOptions={{
          canvasActions: {
            loadScene: false,
            saveToActiveFile: false,
            export: false,
            saveAsImage: false,
            toggleTheme: null,
            clearCanvas: true,
            changeViewBackgroundColor: true,
          },
          tools: { image: false },
        }}
      >
        <BoardMainMenu />
        <BoardWelcomeScreen />
        <I18nBridge />
      </Excalidraw>
    </div>
  );
}

function frame() {
  return new Promise((resolve) => requestAnimationFrame(() => resolve()));
}

/** Waits until Excalidraw has the initial scene on its canvas. */
async function mounted() {
  while (!board.api || board.api.getAppState().isLoading) await frame();
  await frame();
  await frame();
}

/** The stored scene of board.json, or null when it is not there as the app writes it: a
 * string `stamp` and a `scene` that is an object with a list of `elements`. */
function storedScene(stored) {
  if (!stored || typeof stored !== "object" || typeof stored.stamp !== "string") return null;
  const scene = stored.scene;
  if (!scene || typeof scene !== "object" || Array.isArray(scene) || !Array.isArray(scene.elements)) return null;
  return scene;
}

/** The scroll and zoom the writer last looked at, when they belong to this scene. */
function viewOf(view, stamp) {
  if (!view || typeof view !== "object" || !stamp || view.at !== stamp) return null;
  const { scrollX, scrollY, zoom } = view;
  if (![scrollX, scrollY, zoom].every((n) => typeof n === "number" && Number.isFinite(n)) || zoom <= 0) return null;
  return { scrollX, scrollY, zoom: { value: Math.min(30, Math.max(0.1, zoom)) } };
}

export async function startBoard() {
  installKeys();
  defineCalls(boardCalls, { needsReady: true });
  installGestureTracking({ onHidden: postLibrary });
  whenLibraryWaitingChanges(sendStatus);

  const [stored, storedLibrary] = await Promise.all([fetchJson("./board.json"), fetchJson("./library.json")]);
  // The app always serves board.json (4.3). Without a stored scene to start from, the page
  // does not start: an empty canvas that posted `ready` would be taken for the drawing, and
  // the writer's first stroke saved over it. The app shows its "stopped working" instead.
  const scene = storedScene(stored);
  if (!scene) throw new Error("board.json could not be read: there is no stored scene to start from");
  const stamp = stored.stamp;
  const elements = scene.elements.filter((e) => e && !e.isDeleted && !NEVER_KEPT.has(e.type));

  const files = await loadPictures(scene.files);
  for (const id of Object.keys(scene.files ?? {})) board.filesKnown.add(id);
  await loadFontsFor(elements);
  const restored = restore({ elements, appState: scene.appState ?? {}, files }, null, null, {
    refreshDimensions: true,
    repairBindings: true,
  });

  const libraryItems = restoreLibraryItems(itemsOfStored(storedLibrary), "unpublished");
  servedLibrary(libraryItems);

  const view = viewOf(stored?.view, stamp);
  const appState = {
    viewBackgroundColor: restored.appState.viewBackgroundColor,
    gridSize: restored.appState.gridSize,
    gridStep: restored.appState.gridStep,
    gridModeEnabled: restored.appState.gridModeEnabled,
    ...(view ?? {}),
  };
  const initial = { elements: restored.elements, appState, files, libraryItems };

  createRoot(document.getElementById("root")).render(<App initial={initial} />);
  await mounted();

  const api = board.api;
  // Opening is not an edit: nothing of the load is undone, and nothing of it is saved.
  api.history.clear();
  if (!view && api.getSceneElements().length) api.scrollToContent(undefined, { fitToContent: true, animate: false });
  watchTheInterface(() => {
    try {
      const said = board.t?.("toolBar.embeddable");
      return said && said !== "toolBar.embeddable" ? said : "Web Embed";
    } catch {
      return "Web Embed";
    }
  });
  currentRev();
  board.rev = 0;
  board.posted = 0;
  recordVersions(0);
  markReady(stamp);
  post("ready", { excalidraw: EXCALIDRAW_VERSION, elements: api.getSceneElements().length, stamp });
  sendStatus();
}

export function startBoardSafely() {
  startBoard().catch((error) => {
    reportError(`the whiteboard could not start: ${error?.message ?? error}`);
  });
}
