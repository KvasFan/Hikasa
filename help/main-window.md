# The main window

`hikasa-gui` shows one or more windows, each with a menu bar, a tab bar, the active view with its toolbar, a
status bar and, when needed, a message line, notices and dialogs. Every action is a command with an id, a
caption, a hint and a default shortcut; menus, toolbars and keys come from the same catalogue, and
`commands.toml` changes them (see [hikasa settings](topic:settings)).

## Windows and tabs

- Ctrl+T opens a new tab with the [Home view](topic:view-home); Ctrl+W closes the active tab. Session ▸ New
  Window opens another window with a Home tab.
- Ctrl+Tab and Shift+Ctrl+Tab select the next and the previous tab.
- Right-click a tab for New Tab, Duplicate Tab (the same comparison again, in its own tab), Move Tab to New
  Window, Move Tab to Window ▸ (another open window), Close Other Tabs and Close Tab.
- Drag a tab along the tab bar to reorder it. Where the system reports window positions, releasing it over
  another window's tab bar moves it there, and releasing it outside every window opens it in a new window; on
  other systems (Wayland) use the two Move commands.
- Comparisons opened from the command line or by `hikasa gui open` go to the window used last; with
  `[tabs] open_sessions_in = "window"` they open a new window. `[tabs]` also decides where new tabs go, whether
  closing the last tab closes its window, whether the tab bar hides with one tab, and the warnings before
  closing several tabs or windows.
- Ctrl+Q quits; every view that a waiting `hikasa gui open --wait` started reports its result first.
- The size of the last window closed and the layout of each view type (toolbar shown, log panel, font size,
  thumbnail) are remembered in `state.toml`.

## Menus, toolbars and keys

- The menus show the commands of the active view grouped by kind; View ▸ Display Filter lists the display
  filters and toggles.
- The toolbar shows the view's toolbar commands as icons. Right-click it to show the display filters as a
  drop-down, as favourites or as toggles, to use large buttons with labels, or to hide the toolbar (View ▸
  Toolbar shows it again).
- Hints appear after a short delay with the caption, the shortcut and a description (`[tweaks.interface]` and
  `[tweaks.metrics]` options).
- Keys without Ctrl or Alt (Enter, arrows, letters) act where the keyboard focus is (the folder tree, a path
  field); other shortcuts work anywhere in the window.

## Status bar, jobs, messages and notices

- The status bar shows the active view's state on the left and its details on the right (format, encodings,
  line endings). While comparisons run, a job indicator shows how many; click it to see them and cancel one.
- A view's message line shows hints about what just happened (for example a drop it cannot use); close it with
  ×.
- Notices appear in the top-right corner: settings changed on disk and reloaded, a compared file changed on disk
  (reloaded automatically, or with a Reload button), a comparison in a background tab that failed. Information
  notices disappear after a few seconds; the others stay until dismissed.

## Dialogs

Dialogs open inside their window and take its input until they close. Enter presses the default button, Esc
cancels, F1 opens the dialog's help topic. "Do not ask again" turns the warning off in `options.toml`.
Help ▸ About shows the version.

## Drag and drop

Drop one file or folder on a path field of the Home view to fill it (two on the left field fill both). Drop two
files or two folders on a window to compare them.

## Help

- F1 (Help ▸ Context Sensitive Help) opens the topic of the active view in the help panel.
- Help ▸ Contents opens the [contents](topic:index); links in the panel open other topics.
- Esc or the panel's close button hides it.

## Colours and fonts

The colours come from the scheme of `theme.toml` (light and dark, following the system by default); the font
sizes of `theme.toml` set the text panes and the folder tree.

## Starting the window

`hikasa-gui <left> <right>` opens a comparison directly; a second start sends its request to the running
window and exits. See [hikasa gui](topic:gui), [Folder Compare](topic:view-folder-compare) and
[Text Compare](topic:view-text-compare).
