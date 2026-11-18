# The Picture Compare view

The Picture Compare view shows two pictures and their difference image: the left picture, the right picture and the
difference pane, side by side (View ▸ **Side-by-Side Layout**: Left | Right with the difference pane under them) or
stacked (**Over-Under Layout**: Left above Right with the difference pane beside them). View ▸ **Difference Pane**
hides or shows the third pane. Every picture format the [hikasa picture compare](topic:picture-compare) command reads
opens here; two files of a picture format open in this view from the command line, the Home view, a folder row and
Open With ▸ *Picture Compare*.

## Zoom and panning

- One zoom and one position for every pane: the panes always show the same place (synchronized panning).
- View ▸ **Fit to Window** (the default) shows the whole picture in every pane, enlarging small pictures; it follows
  the window's size until you zoom or pan. **Actual Size** shows one picture pixel per screen pixel. **Zoom In** and
  **Zoom Out** (Ctrl+Num + and Ctrl+Num −) go through the fixed steps from 1/256 to 6,400 %; View ▸ **Zoom** ▸ lists
  the usual ones. Ctrl+wheel zooms one step about the pointer, a touchpad pinch continuously.
- Drag with the left button in the left or right pane, or with the middle button in any pane, to pan; the wheel pans
  up and down, Shift+wheel sideways; Page Up and Page Down move by a pane, Home and End to the top and the bottom.
- From 1,600 % on a grid shows the pixel boundaries.
- **Full Screen** (F11) shows the focused pane alone; F11 or Esc returns.

## Aligning the pictures

- Drag in the difference pane to move the right picture over the left one (the offset); the outline shows where it
  goes, the statistics follow when you stop. Esc during the drag returns to where you started. The arrow keys move it
  by one pixel, with Ctrl by ten; **Reset Difference Offset** puts it back.
- **Rotate Clockwise**, **Rotate Counterclockwise**, **Flip Horizontally** and **Flip Vertically** turn the picture
  of the focused pane (the right one from the difference pane). **Auto Scale** resamples the smaller picture to the
  larger one's size; pictures of different shapes get a warning.
- **Swap Sides** exchanges the pictures (the offset is reversed).

## The difference image

- **Tolerance Mode** colours each position same (shaded), similar (within the tolerance) or different; the
  **Tolerance** field above the panes sets the largest distance counted as similar (0 to 255).
- **Mismatch Range Mode** shows how far each pixel is from the other: black is equal, brighter yellow is further.
- **Blend Mode** mixes the pictures by the **Blend** percentage; **Blend Toggle** (F8) flips between the two pictures
  for a quick look without changing the session.
- **Transparency Checkerboard** draws a checkerboard under transparent pixels. The colours are the theme's
  (Options ▸ Appearance ▸ Picture).

## Status, Info and the clipboard

- The status bar shows the pixel under the pointer in every pane: its position, each side's coordinates and colour,
  the distance and the class; then the offset, the zoom, the mode and the verdict.
- Session ▸ **Picture Compare Info** (Ctrl+I) shows the statistics ([Picture Compare Info](topic:dialog-picture-info)).
- Edit ▸ **Copy** (Ctrl+C) places the focused pane at full size on the clipboard as it is shown (mode, checkerboard,
  colours). **Open Clipboard** (Shift+Ctrl+V) compares the clipboard's picture as the current side.
- The header above each pane shows its file: size, the decoder's colour type and bit depth, alpha, frames, the
  orientation applied, the tone map of HDR pictures, the file size; *Converted* when a format's conversion produced it.

Session ▸ **Rules…** opens the [Session Settings](topic:dialog-session-settings) on its Picture tab (mode, tolerance,
blend, Ignore Unimportant Differences, Auto Scale, tone map, maximum megapixels) and its Format tab. Saving the session
keeps the orientations, the offset, the zoom, the layout and the checkerboard.
