// The bridge: the page posts to the app, one JSON string a message, through the one script
// message handler a whiteboard's view has (docs/whiteboard-design.md, 4.4). Nothing the app
// is told here is ever run there; it only reads.

export const HANDLER = "innerwordBoard";
/** innerword_board_web::bridge::MAX_MESSAGE_BYTES: the view drops a longer message. */
export const MAX_MESSAGE_BYTES = 24 * 1024 * 1024;
/** A scene whose JSON is longer than this (UTF-8) is posted as scenePart messages. */
export const PART_BYTES = 16 * 1024 * 1024;
/** innerword_board::limits::MAX_VIEW_SCENE_BYTES: over it the scene is not posted by the
 * autosave (status tooLarge); the app asks for it at the end, for the recovery file. */
export const MAX_VIEW_SCENE_BYTES = 256 * 1024 * 1024;

const encoder = new TextEncoder();

/** The UTF-8 length of a text. */
export function utf8Bytes(text) {
  let ascii = true;
  for (let i = 0; i < text.length; i++) {
    if (text.charCodeAt(i) > 0x7f) {
      ascii = false;
      break;
    }
  }
  return ascii ? text.length : encoder.encode(text).length;
}

/** Whether a text is at most `limit` bytes of UTF-8 (without encoding a short one). */
export function fitsIn(text, limit) {
  return text.length * 3 <= limit || utf8Bytes(text) <= limit;
}

/**
 * JSON text made well formed: JSON.stringify writes a lone surrogate (half of a character
 * pair) as an escape such as "\ud800", which serde_json refuses in a string. A paired one
 * is written as the character itself, so every such escape in its output is a lone one and
 * becomes U+FFFD. An escaped backslash is skipped as a whole, so "\\ud800" (a backslash and
 * the letters) is left alone. (`String.prototype.toWellFormed` is newer than the WebKit of
 * macOS 12.3.)
 */
export function wellFormed(text) {
  return text.replace(/\\(\\|u[dD][89a-fA-F][0-9a-fA-F]{2})/g, (match) => (match === "\\\\" ? match : "\\ufffd"));
}

function handler() {
  try {
    return window.webkit?.messageHandlers?.[HANDLER] ?? null;
  } catch {
    return null;
  }
}

/** Posts a message already written as JSON text. False when there is no app (a browser). */
export function postText(text) {
  const h = handler();
  if (!h) return false;
  try {
    h.postMessage(text);
    return true;
  } catch {
    return false;
  }
}

/** Posts `{"v":1,"kind":kind,...fields}`. */
export function post(kind, fields = {}) {
  return postText(wellFormed(JSON.stringify({ v: 1, kind, ...fields })));
}

/** Posts a scene (its JSON, well formed already) with its rev: one `scene` message, or
 * `scenePart` messages when it is longer than one may be. */
export function postSceneText(rev, json) {
  if (fitsIn(json, PART_BYTES)) {
    return postText(`{"v":1,"kind":"scene","rev":${rev},"scene":${json}}`);
  }
  const parts = cutIntoParts(json);
  let ok = true;
  parts.forEach((text, index) => {
    ok = post("scenePart", { rev, index, count: parts.length, text }) && ok;
  });
  return ok;
}

/** The scene's JSON cut between characters into slices of at most PART_BYTES of UTF-8,
 * each of which also fits a message once escaped as a JSON string. */
export function cutIntoParts(json) {
  const parts = [];
  let at = 0;
  while (at < json.length) {
    let end = Math.min(json.length, at + PART_BYTES);
    for (;;) {
      // Never between the two halves of a character pair.
      if (end < json.length && isHighSurrogate(json.charCodeAt(end - 1))) end--;
      const slice = json.slice(at, end);
      const bytes = utf8Bytes(slice);
      const message = bytes <= PART_BYTES ? utf8Bytes(JSON.stringify(slice)) + 128 : Infinity;
      if (bytes <= PART_BYTES && message <= MAX_MESSAGE_BYTES) {
        parts.push(slice);
        break;
      }
      const over = Math.max(bytes / PART_BYTES, message / MAX_MESSAGE_BYTES);
      end = at + Math.max(1, Math.floor((end - at) / over) - 16);
    }
    at += parts[parts.length - 1].length;
  }
  return parts;
}

function isHighSurrogate(unit) {
  return unit >= 0xd800 && unit <= 0xdbff;
}

let lastError = 0;

/** An error of the page, for the app's standard error: at most one a second. */
export function reportError(message) {
  const now = Date.now();
  if (now - lastError < 1000) return;
  lastError = now;
  post("error", { message: String(message).slice(0, 2000) });
}

export function installErrorReports() {
  window.addEventListener("error", (event) => {
    reportError(event.message || String(event.error ?? "an error"));
  });
  window.addEventListener("unhandledrejection", (event) => {
    const reason = event.reason;
    reportError(`a promise was rejected: ${reason?.message ?? reason}`);
  });
  document.addEventListener("securitypolicyviolation", (event) => {
    reportError(`the policy refused ${event.blockedURI || "an inline resource"} (${event.effectiveDirective})`);
  });
}
