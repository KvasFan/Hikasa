# Picture Compare

Compares two images visually and by metadata.

- **PX-001** (P0): Supported decoders via the `image` crate: PNG, JPEG, GIF (first
  frame), BMP, ICO (largest frame), TIFF, WebP, TGA, PNM, QOI, EXR/HDR (tone-mapped to
  8-bit), DDS; SVG via `resvg` (P2); camera RAW via external converter (format
  conversion command line, P2). All images are normalized to 8-bit RGBA for comparison;
  colour spaces beyond sRGB are not supported in 1.0.
  *M2 notes (§1.12.14, D-337):* the container is recognised by its content (the magic bytes), the name's extension
  only when the content says nothing (TGA has no signature): a `.png` holding a JPEG decodes as a JPEG. PNG (APNG:
  the default image), JPEG, GIF (first frame), BMP, ICO (the largest entry), TIFF (first page), WebP (lossy and
  lossless; first frame), TGA, PNM, QOI, OpenEXR (first part) and Radiance HDR decode; DDS, AVIF, SVG and anything
  unrecognised are `unsupported` (a folder comparison then compares their bytes). Normalization to RGBA8 is
  normative: grey `l` → (l, l, l, 255), grey-alpha → (l, l, l, a), RGB → alpha 255; 16-bit channels
  `(v · 255 + 32767) / 65535`; float channels of TIFF and the like `clamp(v, 0, 1) · 255` rounded half up (NaN → 0).
  HDR and OpenEXR are linear light: each colour channel goes through the **tone map** (`clamp`, the default, keeps SDR
  content as authored; `reinhard`, `v / (1 + v)`, keeps highlights; NaN → 0, +∞ → 1, negative → 0) and the sRGB
  transfer function by a 65,536-entry table whose CRC-32 a test pins on every platform; alpha is clamped linearly.
  A JPEG, TIFF, PNG or WebP EXIF orientation is applied, so pictures compare as viewers show them
  (`orientation_applied`, informational); an ICC profile other than sRGB is ignored with `color_profile_ignored`;
  an animated or multi-page input compares its first frame (`frames_ignored`, informational). A picture of more than
  `--max-megapixels` (256 MP by default) is `too_large` from its header, before any pixel buffer is allocated
  (the decoder's own allocations are bounded to 16 bytes a pixel plus 64 MiB); a file above 2 GiB is not read; a
  truncated or corrupt file is `invalid_data`; no input makes the decoder panic (a property test feeds random and
  mutated files). The two sides decode in parallel. File Info reports the format, size, the decoder's colour type,
  bits a channel, alpha, frames, the orientation applied and the tone map.
- **PX-002** (P0): Layout: left picture, right picture, **difference** pane (three
  panes; side-by-side or stacked); File Info headers (dimensions, bit depth, format,
  file size); zoom (In/Out Ctrl+Num+/−, Actual Size, Fit to Window, mouse wheel with
  Ctrl), synchronized panning, full screen (F11, Esc exits) of the picked pane.
  *M2 W13 notes (§1.13.10, §1.13.11, D-362, D-363):* one viewport for the view: every shown pane maps the area with the
  same scale and centre, so the panes always show the same place. **Fit to Window** (the default) is the largest scale
  at which the area fits every shown pane with an 8-point margin, enlarging small pictures up to 64×; it follows the
  window until the user zooms or pans. **Actual Size** is one picture pixel per physical screen pixel. Zoom In/Out step
  through 1/256 … 64 (25 fixed steps); Ctrl+wheel steps about the pointer (the area point under it stays there),
  a pinch zooms continuously. Panning: a primary drag in the left or right pane, a middle drag in any pane, the wheel
  (Shift: sideways), the scroll bars, Page Up/Down, Home/End; at least 32 points of the area stay in every pane. A
  pixel grid shows from 1,600 %. Layouts: side by side (the difference pane under the two pictures) or stacked (beside
  them); *Difference Pane* hides it. Full Screen shows the focused pane alone. The window never holds a full-resolution
  image: each pane shows an overview (the finest level of at most 1,024 × 1,024 texels) and the 256 × 256 tiles of the
  level `⌊log₂(1/zoom)⌋` it needs, rendered by the engine on the pool and uploaded at most 8 a frame, within 64 MiB of
  textures per window (the least recently drawn tiles go first; a hidden tab keeps only its overviews).
- **PX-003** (P0): Difference modes:
  - **Tolerance Mode** (default): per pixel, the maximum channel distance is compared
    with a configurable *Tolerance* (0–255, toolbar spinner). Same pixels render in
    shades of grey, unimportant (≤ tolerance) in blue, important in red (colours
    configurable). *Ignore Unimportant Differences* treats ≤ tolerance as same.
  - **Mismatch Range Mode**: black = same, yellow brightness = degree of difference.
  - **Blend Mode**: blend the two images by a *Blend percentage* (toolbar slider);
    **Blend Toggle** (F8) flips between left and right.
  *M2 W13 notes (§1.13.12, D-364):* the mode buttons are on the toolbar and in View; the **Tolerance** field and, in
  Blend mode, the **Blend** slider are above the panes. F8 is a quick look: it flips the shown percentage between 0 and
  100 (outside Blend mode it switches to Blend at 0) and never changes the session; the slider does. A mode, the blend
  or the theme's colours change only the difference pane's tiles; a tolerance recomputes the statistics in a job.
  *M2 notes (§1.12.16, §1.12.18, D-339, D-341):* the **distance** of a position is the largest channel difference
  over R, G, B and A, except that two fully transparent pixels are equal whatever their colour; a position covered
  by one picture only counts that pixel's alpha (it is compared with a transparent pixel). With the tolerance `t`
  (0–255, default 0) a position is *same* at distance 0, *similar* (unimportant) up to `t`, *different*
  (important) above it. The verdict: 1 when the files' bytes are identical (checked only when every position is
  same), 2 when every position is same, 12 with only similar positions (2 with *Ignore Unimportant Differences*),
  13 with any different position; never 11. The display mode never changes the classes or the verdict. Rendering
  is integer arithmetic (round half up): **Tolerance** shades the theme's *same* colour by the luma
  `(77r + 150g + 29b + 128) >> 8` of the left pixel (the right one where only it covers) between a quarter and full
  brightness, and paints *similar* and *different* in the theme's colours; **Mismatch Range** is black for same and
  `(v, v, 0)` with `v = 32 + 223·d/255` otherwise (a 1-step difference stays visible); **Blend** mixes every channel,
  alpha included, `(L·(100 − p) + R·p) / 100`, a missing side transparent. Positions no picture covers are
  transparent.
- **PX-004** (P0): Geometry tools: **Auto Scale** (enlarge the smaller image to match),
  Rotate Clockwise / Counterclockwise (90°), Flip Horizontally / Vertically (current
  picture), **difference offset**: drag in the difference pane to shift the right
  image's origin, arrow keys nudge 1 px, Ctrl+arrows nudge 10 px, **Reset Difference
  Offset** (or click the offset label). Transformations are view state, not saved to
  files.
  *M2 notes (§1.12.17, D-340):* rotations and flips of a side compose into one of the eight elements of the
  dihedral group (`none`, `cw`, `ccw`, `180`, `flip-h`, `flip-v`, `transpose`, `transverse`), applied as exact pixel
  index maps (no resampling, no copy). **Auto Scale** (off by default) resamples the picture with fewer pixels (the
  right one on a tie) to the other's exact size by a normative fixed-point bilinear interpolation, so results are
  the same bytes on every platform; when the aspect ratios differ by more than 1 % it says `aspect_ratio_differs`.
  The **offset** is the right picture's origin in left coordinates (each component within ±2²⁰); the compared area is
  the union of both rectangles, and positions only one picture covers count as differences by that pixel's alpha.
  Orientation, Auto Scale and the offset are view state, saved with the session, never written to the files; a new
  geometry recomputes the statistics without decoding again (8 ms for two 12 MP pictures).
  *M2 W13 notes (§1.13.12, D-364):* in the window the offset drag moves the right picture by the drag's distance in
  area pixels (the rest under a pixel is carried to the next move); the right pane and the difference pane's outline
  follow at once while the statistics are recomputed in the background, one job at a time, always ending on the last
  offset; Esc during the drag returns to its start. Transforms act on the focused pane's picture (the difference
  pane's: the right one). Swap Sides exchanges the decoded pictures and negates the offset without decoding again.
- **PX-005** (P1): **Replacements** (session tab): colour pairs (colour on left ↔
  colour on right) considered unimportant.
- **PX-006** (P1): **Compare Metadata**: a tree of metadata groups (Image, Camera/EXIF,
  GPS, Interoperability, IPTC, Thumbnail, Windows XMP fields…) compared by key; shown in
  a details panel with per-item important/unimportant colouring.
- **PX-007** (P0): Show transparency as checkerboard (option); background colour per
  theme; Picture Compare colours page in options (same/different/similar pixels).
  *M2 notes (§1.12.18, D-341):* the checkerboard is computed by the engine, so `--diff-output --checkerboard`, reports
  and the window draw the same pixels: transparent pixels are composited source-over onto 8-pixel cells of the
  theme's *checker light* and *checker dark* colours, anchored at the area's top-left corner. Images are rendered in
  64-row bands in parallel or as tiles at scale 1/2ᵏ (k = 0…8, each pixel the rounded mean of its 2ᵏ × 2ᵏ block),
  so a view uploads only the tiles it shows at its zoom.
  *M2 W13 notes (§1.13.11):* the window paints the checkerboard itself (cells of 8 physical pixels anchored at the
  area's top-left corner on screen, equal to the engine's at 100 %) under the tiles, which are rendered without it, so
  the option is free to toggle; the colours are the window's theme (they follow a theme change at once).
- **PX-008** (P0): Status bar: cursor pixel coordinates and both pixels' RGBA values,
  current offset, zoom; **Picture Compare Info**: counts of same/similar/different
  pixels and percentage.
  *M2 notes (§1.12.19, D-342):* the statistics count the covered positions, *same*, *similar* and *different*,
  the positions only one side covers, the largest and the mean distance and the bounding rectangle of the non-same
  positions, with integer sums (any number of threads gives the same numbers); percentages have 4 decimals. The
  probe of a position gives both sides' picture coordinates and RGBA values, the distance and the class.
  *M2 W13 notes (§1.13.13, D-365):* the status bar probes the area pixel under the pointer in any pane and prints it as
  `picture compare --probe` does; Picture Compare Info lists the statistics, the geometry and the options, *Copy*
  places the lines `picture compare` prints and *Show* zooms onto the bounds of the differences.
- **PX-009** (P0): Edit ▸ Copy copies the picked pane (or difference image) to the
  clipboard; Open Clipboard loads an image from the clipboard.
  *M2 notes (§1.12.20, D-343):* images are written as PNG by the `image` crate's encoder (RGBA8, adaptive filters;
  fast compression for `--diff-output`, best for reports); clipboard images use it too, so they are compressed (a
  flat 1,000 × 1,000 image is about 6 KiB) where M2 W3 wrote an uncompressed PNG. A picture is placed on the system
  clipboard as an image (on Linux a helper keeps it after `hikasa` ends, as for reports).
  *M2 W13 notes (§1.13.13):* in the window Copy renders the focused pane at full resolution in the mode shown, with the
  view's checkerboard and the window's colours (what is seen); Open Clipboard compares the clipboard's picture as the
  current side (the difference pane: the right one), a clipboard without one says so.
  *M2 W13b review fixes (§1.13.26 W13b review-fix notes):* the window's 64 MiB of textures hold for any number of
  picture tabs: the shown view always gets room for what it draws, the hidden tabs' overviews giving way, the least
  recently shown first. The scroll bars lie on top of the panes (a bar drag scrolls every pane, never moves the offset);
  an offset drag counts from the press itself; nothing reaches the panes under a dialog; Ctrl+wheel zooms one step a
  notch (small touchpad deltas add up). Both splitters drag (`pane_split` and `difference_split` of
  `[layout.picture-compare]`). The status bar's offset resets on a click and its zoom opens the Zoom list; each side's
  header has the path field (Enter compares the typed file on that side), *Browse…* and the format picker (*Auto* or an
  enabled Picture format; a change decodes again); a file dropped on a pane or its header replaces that side (the
  difference pane's: the right), two replace both. A picture pair opened from a folder view with a session is its child
  (D-311): Next/Previous Difference Files go to the parent's next picture pair that differs, passing over the others.
  Closing a view waits for a running geometry job only when `--wait` waits for it.
- **PX-010** (P1): Report: side-by-side (images + difference image embedded in HTML)
  or summary (statistics); `ignore-unimportant` option.
