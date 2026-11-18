# Display Font

View ▸ **Display Font…** sets the font of the text views of this type (Text Compare, or Text Edit).

- **Family**: one of the fonts of the theme, or *Editor font of the theme*.
- **Size**: 6 to 72 points.
- **Use for the theme's Editor font** also writes the theme's editor font (`theme.toml`), so every view that uses it
  follows.

The family and the size are kept in `[layout.text-compare]` (`font_family`, `font_size`) and restored for the next
view; Ctrl+Num +/−/0 still change the size of this view. See [The Text Compare view](topic:view-text-compare).

In a [Hex Compare view](topic:view-hex-compare) the dialog sets the **Size** only (6 to 48 points, kept in
`[layout.hex-compare] font_size`): the panes use the theme's monospace font.
