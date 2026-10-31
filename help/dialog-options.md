# Options

Tools ▸ Options… sets the program's preferences. The tree on the left lists the pages; the search box above it finds
a page by its title, a label, a key such as `tweaks.metrics.hint_show_ms` or an id such as `OPT-012`. The right side
shows the page.

## Apply, OK and Cancel

The dialog edits a copy. **Apply** writes the files that changed and keeps the dialog open, **OK** does the same and
closes it, **Cancel** drops what was not applied. A file with nothing changed is not written; what you did not touch
stays, unknown keys and options without a page included. Values are checked as you type: while a file you changed has
a problem, Apply and OK are off; a problem in a file you did not touch stops nothing. OK closes only when every file was
saved: a file that failed keeps your edit and the reason is shown. Esc closes an open list or colour picker first; with
changes not applied it, and the window's close button, ask before discarding them. F1 shows this topic. Drag the title
to move the dialog: the theme preview shows in the window under it.

The window, and Exit, wait while the dialog is saving: a notice says so.

When a file changed on disk since the dialog read it (a `hikasa settings set`, for example), Apply asks what to do:
**Overwrite** writes your version, **Reload** takes the file from disk and drops your edits of it, **Cancel** keeps
your edits in the dialog. Overwrite keeps what the file holds that this version does not know. A notice says so as soon
as the change is seen.

A file this version cannot read (written by a newer version, or not valid TOML) is never overwritten from here. The page
names it and says why, and Apply offers only **Reload** and **Cancel** for it. `hikasa settings restore-defaults` or an
editor can change it.

## Pages

- **Startup**: the workspace loaded at start and the one saved on exit.
- **Tabs**: where sessions open, warnings when closing several tabs or windows.
- **Theme, Colors, Fonts**: below.
- **Text Editing**, **Next Difference**, **Backups**, **File Operations**: as their names say.
- **Archive Types**: the mask of each archive type; empty turns the type off. Masks apply to comparisons started
  later.
- **Toolbars, Shortcuts, Menus**: below.
- **Open With**: the external applications, with New, Edit, Clone and Delete; each one is checked when you press OK.
  Its shortcut is set by pressing the key. Apply and OK take an open item too, or refuse it and show why.
- **Tweaks**: Editor Display, File Operations, Interface, Log Panel, Metrics, Name Filter Presets, Scripts.

## Theme, Colors, Fonts

Choose the theme mode and the scheme of light and of dark mode. A scheme is a set of colours based on the built-in
`light` or `dark`. New makes an empty one on the base you pick, Clone copies the edited one, Delete removes it (not
the built-ins). The Folder Views, File Views and Picture Compare pages list every element; Reset takes an element back
to its base, Reset All does it for the whole scheme (on a built-in this removes its override file, including anything
a later version wrote there; resetting the last element one by one keeps such a file).

Every change shows at once in every window; Cancel brings the applied theme back. Default text below 4.5:1 contrast on
a background is reported as a warning, never refused. The Fonts page sets the four fonts; a family other than the
built-in font is stored but not used in this version.

## Toolbars, Shortcuts, Menus

Pick a view type and search its commands by id, caption, description or shortcut. Menu and Toolbar choose where a
command shows (Toolbar only for commands with an icon). For the selected command, **Add** takes the next key you
press (Esc gives up, and so does leaving the page), **Remove** drops the selected shortcut and **Default** returns to
the catalogue's; it is on only when the command's shortcuts were changed. Views and commands this version does not know
stay in the file untouched. A shortcut
used twice in a view is shown on both rows and does not stop Apply. The page also sets the view's display filter
mode, large buttons, show all in menu and show all on toolbar; Reset View drops the view's customization. The same
dialog opens from the toolbar's right-click menu, Toolbar Options….

See [hikasa settings](topic:settings), [The main window](topic:main-window) and [Contents](topic:index).
