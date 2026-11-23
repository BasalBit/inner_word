// The pictures of a scene: the stored bytes of each, which the view's source answers under
// _files/<FileId>, made into what Excalidraw's files hold (a data URL).

const FILE_ID = /^[0-9a-f]{32}$/;

export function toDataURL(blob) {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(reader.result);
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

/** The stored bytes of one picture as a data URL, or null when the library does not have them. */
export async function fetchPicture(file) {
  if (typeof file !== "string" || !FILE_ID.test(file)) return null;
  try {
    const response = await fetch(`./_files/${file}`);
    if (!response.ok) return null;
    return await toDataURL(await response.blob());
  } catch {
    return null;
  }
}

/**
 * The pictures a scene's `files` names ({"<Excalidraw file id>": {mimeType, file, created}}),
 * as Excalidraw's BinaryFiles. A picture the library does not have is left out: Excalidraw
 * draws its element as a missing picture.
 */
export async function loadPictures(files) {
  const loaded = {};
  await Promise.all(
    Object.entries(files ?? {}).map(async ([id, entry]) => {
      const dataURL = await fetchPicture(entry?.file);
      if (!dataURL) return;
      loaded[id] = {
        id,
        mimeType: entry.mimeType || dataURL.slice(5, dataURL.indexOf(";")),
        dataURL,
        created: Number.isFinite(entry.created) ? entry.created : Date.now(),
      };
    }),
  );
  return loaded;
}
