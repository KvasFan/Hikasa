# Quick Compare at Startup

When Hikasa is asked from the command line to compare two files — `hikasa a.txt b.txt`, `hikasa gui open a.txt b.txt`,
or a file manager's *Open With* — and **Show the quick compare dialog when starting with a file comparison** is on
(Options ▸ Startup, `[startup] show_quick_compare_dialog`), this dialog comes before the view
([Options](topic:dialog-options), [hikasa quick](topic:quick)):

- both files with their size and modification time;
- the **Method**: *Binary* (byte by byte) or *Rules-based* (the text rules; Options ▸ Startup's quick compare method
  is the default); a change compares again;
- the **Result**, as the folder view's Quick Compare says it (identical, same by the rules, unimportant differences
  only, different, or the error);
- the view **Open View** opens, chosen by the files' formats.

With **Open the view automatically if the files are different** on (`[startup] open_view_if_different`), a result that
is not the same closes the dialog and opens the view at once; an error (a file missing or that cannot be read) stays
in the dialog. Otherwise **Open View** opens the view and **Close**
closes the dialog (Enter presses Open View when the files differ, Close when they are the same). The check **Show
this dialog when starting with two files** turns the option off or on at once.

**Stop** stops a comparison that runs; the result then says it was stopped. The window's close button (or Esc) stops
it too and closes the dialog.

A command that waits (`--wait`) gets the quick comparison's exit code when the dialog is closed (1 identical, 2 same
by the rules, 11 binary different, 12 unimportant differences only, 13 different, 100 an error), no verdict (0) when
the comparison was stopped, or the view's when the view opens.

The dialog is not shown for a view kind given on the command line (`--fv`, `hikasa text compare --gui`, …), view
options (`--display`, `--context`, …), two folders, `--edit`, a session, a workspace or a script, nor for files opened
in a window: dropped on it, compared from Home, opened from a folder view's row or its *Open With*.
