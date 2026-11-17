# Find and Replace

Search ▸ **Find…** (Ctrl+F) opens the Find panel above the panes; Search ▸ **Replace…** (Ctrl+H) the Replace panel.
The panes stay usable while it is open; Esc in the field closes it.

- The text to find (it starts as the selection, or the word at the caret), **Match case**, **Whole words**, **Regular
  expression**, **Wrap around**, and in Text Compare the sides searched (**Left**, **Right**, **Both**: by row, the
  left side first).
- Enter or ▼ finds the next hit, Shift+Enter or ▲ the previous one (F3 and Shift+F3 also work with the panel closed).
  The hit is selected in its pane. *Passed the end of the file.* says the search wrapped; *Text not found.* that there
  is no hit. **Close automatically** hides the panel after a hit.
- Every hit on screen is highlighted.
- **Select All** selects the lines holding a hit (*N lines selected*): Copy copies those lines, Delete removes them.
- **Replace** replaces the selected hit and finds the next one. **Replace All** replaces every hit of the selection when
  it spans lines, else of the whole file (in Text Compare: each side searched that may be edited), after the question
  *Replace N occurrences?*; Undo in each pane takes it back in one step.

A pattern that is not valid shows why under the field. See [The Text Compare view](topic:view-text-compare).
