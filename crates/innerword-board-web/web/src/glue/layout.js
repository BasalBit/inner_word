// The layout Excalidraw draws for the width of the view (docs/whiteboard-design.md, 4.3):
// the full one, or the compact one below 730 pixels.

/**
 * Makes Excalidraw draw the layout of the width its container has whenever that changes
 * (the side chat or the sidebar shown or hidden, the window resized).
 *
 * Excalidraw 0.18.1 chooses the layout in its ResizeObserver (`refreshEditorBreakpoints`,
 * which keeps the choice in `App.device`) and renders there only when the container's
 * rectangle differs from the one in its state. Its listener of the window's `resize`
 * (`onResize`) has stored the new rectangle already, and React 19 renders an update made in
 * a `resize` listener at once (a discrete event: in the microtask after the listener), so
 * with the layout of the old width; the observer, which runs after it, then changes the
 * choice and renders nothing. The layout of the old width stayed until something else made
 * Excalidraw render. `refresh` from an observer of the glue's renders again: a default
 * update, in a task after every observer has run. One render more for each change of the
 * container's size, none while it keeps its size.
 */
export function followTheWidth(api) {
  const container = document.querySelector(".excalidraw-container");
  if (!container || typeof ResizeObserver !== "function") return;
  new ResizeObserver(() => api.refresh()).observe(container);
}
