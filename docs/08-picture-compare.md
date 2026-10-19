# Picture Compare

Compares two images visually and by metadata.

- **PX-001** (P0): Supported decoders via the `image` crate: PNG, JPEG, GIF (first
  frame), BMP, ICO (largest frame), TIFF, WebP, TGA, PNM, QOI, EXR/HDR (tone-mapped to
  8-bit), DDS; SVG via `resvg` (P2); camera RAW via external converter (format
  conversion command line, P2). All images are normalized to 8-bit RGBA for comparison;
  colour spaces beyond sRGB are not supported in 1.0.
- **PX-002** (P0): Layout: left picture, right picture, **difference** pane (three
  panes; side-by-side or stacked); File Info headers (dimensions, bit depth, format,
  file size); zoom (In/Out Ctrl+Num+/−, Actual Size, Fit to Window, mouse wheel with
  Ctrl), synchronized panning, full screen (F11, Esc exits) of the picked pane.
- **PX-003** (P0): Difference modes:
  - **Tolerance Mode** (default): per pixel, the maximum channel distance is compared
    with a configurable *Tolerance* (0–255, toolbar spinner). Same pixels render in
    shades of grey, unimportant (≤ tolerance) in blue, important in red (colours
    configurable). *Ignore Unimportant Differences* treats ≤ tolerance as same.
  - **Mismatch Range Mode**: black = same, yellow brightness = degree of difference.
  - **Blend Mode**: blend the two images by a *Blend percentage* (toolbar slider);
    **Blend Toggle** (F8) flips between left and right.
- **PX-004** (P0): Geometry tools: **Auto Scale** (enlarge the smaller image to match),
  Rotate Clockwise / Counterclockwise (90°), Flip Horizontally / Vertically (current
  picture), **difference offset**: drag in the difference pane to shift the right
  image's origin, arrow keys nudge 1 px, Ctrl+arrows nudge 10 px, **Reset Difference
  Offset** (or click the offset label). Transformations are view state, not saved to
  files.
- **PX-005** (P1): **Replacements** (session tab): colour pairs (colour on left ↔
  colour on right) considered unimportant.
- **PX-006** (P1): **Compare Metadata**: a tree of metadata groups (Image, Camera/EXIF,
  GPS, Interoperability, IPTC, Thumbnail, Windows XMP fields…) compared by key; shown in
  a details panel with per-item important/unimportant colouring.
- **PX-007** (P0): Show transparency as checkerboard (option); background colour per
  theme; Picture Compare colours page in options (same/different/similar pixels).
- **PX-008** (P0): Status bar: cursor pixel coordinates and both pixels' RGBA values,
  current offset, zoom; **Picture Compare Info**: counts of same/similar/different
  pixels and percentage.
- **PX-009** (P0): Edit ▸ Copy copies the picked pane (or difference image) to the
  clipboard; Open Clipboard loads an image from the clipboard.
- **PX-010** (P1): Report: side-by-side (images + difference image embedded in HTML)
  or summary (statistics); `ignore-unimportant` option.
