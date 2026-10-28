# The main window

`hikasa-gui` shows one window with a menu bar, a tab bar, the active view with its toolbar, and a status bar.
Every action is a command with an id, a caption, a hint and a default shortcut; menus, toolbar and keys come
from the same catalogue.

## Tabs

- Ctrl+T opens a new tab with the [Home view](topic:view-home); Ctrl+W closes the active tab.
- Ctrl+Tab and Shift+Ctrl+Tab select the next and the previous tab.
- Ctrl+Q quits; every view that a waiting `hikasa gui open --wait` started reports its result first.

## Help

- F1 (Help ▸ Context Sensitive Help) opens the topic of the active view in the help panel.
- Help ▸ Contents opens the [contents](topic:index); links in the panel open other topics.
- Esc or the panel's close button hides it.

## Starting the window

`hikasa-gui <left> <right>` opens a comparison directly; a second start sends its request to the running
window and exits. See [hikasa gui](topic:gui), [Folder Compare](topic:view-folder-compare) and
[Text Compare](topic:view-text-compare).
