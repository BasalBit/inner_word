# The converter's corpus

Each `*.json` here is a list of skeletons as an agent writes them (`docs/whiteboard-design.md`,
section 7.2). Two tests read them:

- `innerword-board`'s `tests/suite/corpus.rs`: `innerword_board::add` on an empty scene, with
  `HashedMaking::new(&[b"corpus", <file name>], 1795432196771)`, must give exactly the stored
  scene in `golden/<file name>`. After a deliberate change of the converter or of the font
  metrics, write the golden files again with `INNERWORD_BOARD_BLESS=1` and read the diff.
- `innerword-web`'s WebKit program (the converter's cross-check, section 9, B11): the same
  skeletons through Excalidraw's own `convertToExcalidrawElements` and `restoreElements`.

So that the two can agree, every element has an id, no skeleton names a picture (Excalidraw's
converter takes a `fileId`, an agent a `path`), and every arrow bound to two elements is given
from the edge of the one to the edge of the other (Excalidraw's converter keeps the points it
is given; `innerword-board` puts bound ends on the outlines). An arrow or a line that runs straight
up or down is given by its points: Excalidraw's converter takes a `width` of 0 for 100
(`element.width || 100`), where `innerword-board` takes 0. Labels have ids of their own on
each side: match them by `containerId`. Known differences the cross-check allows for: a
freedraw's `width`, `height`, `pressures` and `simulatePressure` (Excalidraw's converter
leaves the first two 0 and the others out; `innerword-board` writes them), and the order of a
container's `boundElements` when an arrow comes before its element in the list.
