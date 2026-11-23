// The one script of the bundle: index.html mounts the whiteboard, export.html answers the
// export's calls. The first import sets where Excalidraw finds its fonts.
import "./glue/asset-path.js";
import "@excalidraw/excalidraw/index.css";
import "./board.css";
import { installErrorReports } from "./glue/bridge.js";
import { startBoardSafely } from "./glue/board.jsx";
import { startExport } from "./glue/export.js";

installErrorReports();
if (document.documentElement.dataset.page === "export") startExport();
else startBoardSafely();
