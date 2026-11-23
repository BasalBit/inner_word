// Excalidraw finds its fonts under EXCALIDRAW_ASSET_PATH: the folder of the page (dist/).
// This module is the first main.jsx imports, so it runs before Excalidraw's. The bundle has
// no inline script (the whiteboard's policy allows none), which is why it is not in the HTML.
window.EXCALIDRAW_ASSET_PATH = new URL("./", location.href).href;
