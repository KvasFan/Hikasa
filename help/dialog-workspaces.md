# Workspaces

A workspace is all open windows with their tabs: named sessions by reference, unsaved views as a copy, plus each
window's size and active tab. Home tabs are not saved.

## Saving and loading

Session ▸ Save Workspace As… names the current set; an existing name is replaced after a question. Session ▸ Load
Workspace ▸ lists the saved ones. Loading first closes every open view (Cancel on a "Save changes?" question stops
the load, with nothing closed), then restores the windows. To stay fast, only the active tab of each window starts
its comparison; the others start when you first select them. A tab whose session no longer exists opens Home, with
a notice.

## Manage Workspaces

Session ▸ Manage Workspaces… lists them with their windows, tabs and shortcuts. Select one to load, rename or delete
it, or to set a shortcut such as `Ctrl+Alt+1`, which loads it from any window. A shortcut used by a global command or
another workspace is reported but allowed; the command wins the key.

## At startup and exit

`[startup] load_workspace` loads a workspace when the window starts without views; `[startup]
on_exit_save_workspace_as` saves one when it exits (typically the same name, such as `last`). `hikasa-gui <name>`
loads one from the command line. See [hikasa workspace](topic:workspace) and [Contents](topic:index).
