# Confirm an operation

Before a file operation of the [Folder Compare view](topic:view-folder-compare) writes anything, its plan is made and
its questions are asked here, one at a time. Nothing is written until the last question is answered. Above the question
the dialog names the row and its sides (`docs/a.txt: from the left to the right`, `…: on both sides`). If you select
another row while the plan is made, nothing is asked and nothing is written: a notice says the selection changed.

## The questions

- **The operation**: what will run, as items and bytes per action (`copy`, `remove`, `trash`, …). Yes runs it; No or
  Cancel writes nothing. `[file_operations] confirm_copy`, `confirm_delete` and `confirm_move` decide whether it is asked.
- **Per item**, in plan order: a read-only target (`confirm_read_only`), a system file (`confirm_system`), a target
  newer than its source (`confirm_overwrite_newer`), a move that replaces (`confirm_replace_during_move`), an item that
  cannot go to the recycle bin (always asked), a backup that would replace a file (`confirm_overwrite_newer`).

## The buttons

- **Yes**, **No**: this question. No to an item leaves it as it is, and what lies in a declined folder with it.
- **Yes to All**, **No to All**: this question and the rest of its kind in this operation (yes to read-only files is
  not yes to system files).
- **Cancel** (Esc, or closing the dialog): nothing is written.

Enter answers Yes to the operation and No to an item's question. F1 shows this page.

## Do not ask again

Ticked with Yes or Yes to All, it sets the question's key in `options.toml` to `false` once the questions are answered,
so later operations proceed without that question; the questions of this operation stay as they were asked. Ticked with
No it writes nothing. The recycle bin's question has no such box. The keys are on the File Operations page of
[Options](topic:dialog-options).

## After the operation

The items that ran, were skipped or failed go to the log, one line each with *Operation details* on, and a notice
sums them up: information when every item ran, a warning with *Show log* when one failed, did not run or the operation
was stopped. Then the rows the operation wrote are read again; a row you select or a folder you close meanwhile stays so.
An item whose file changed since the comparison is not written: it fails "changed since the plan was made" (refresh,
then try again). Closing the tab, loading a workspace, applying Session Settings or quitting while an operation runs
stops it after its current item; its summary still goes to the log, with a notice naming the tab, and Quit waits for
it. See also
[hikasa folder op](topic:folder-op), the same operations from the command line.
