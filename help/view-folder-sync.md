# The Folder Sync view

A Folder Sync view is a [Folder Compare view](topic:view-folder-compare) that plans a synchronization of its two base
folders and shows, in the centre column, what the sync does to each row. It opens from Home's **Folder Sync** type,
Session ▸ New Session ▸ **Folder Sync**, Session ▸ **Sync Base Folders** in a Folder Compare view, a folder-sync
[session](topic:session) or [workspace](topic:workspace), `hikasa gui open <left> <right> --sync`, and
[hikasa folder sync](topic:folder-sync) `--gui`.

## The header

- **Operations**: Update Left, Update Right, Update Both, Mirror to Left, Mirror to Right, or Custom (the matrix of the
  session's `[rules.sync]`). Changing it plans again; the comparison does not run again.
- The plan's state: *Planning…*, the counts of the plan (copies right and left, deletes left and right, conflicts,
  unresolved rows), or the reason no plan can be made (an empty source, deletes on a comparison that did not finish, an
  invalid matrix): then Run Sync is off.
- **Run Sync**: runs the plan, after its questions (see below).

The comparison reads every folder, orphans too; archives are folders with *Sync within archives*, else files to copy.
Changing *Sync within archives* runs the comparison again.

## The centre column

Each row shows its action: copy left to right, copy right to left, delete on the left, delete on the right, or leave
alone (the tooltip says why: same, filtered, read-only, out of scope, …). A **conflict** (different on both sides and
the matrix leaves it) has its own mark; an action an override chose is drawn in the accent colour and its tooltip says
*overridden*. A folder whose rows below act shows a small mark, and its tooltip sums them up ("contains files that will
be copied to the right (3), deleted on the left (1)"). The [Legend](topic:panel-legend) lists the marks.

## Overrides

Actions ▸ **Leave Alone**, **Copy Left to Right**, **Copy Right to Left**, **Delete Left**, **Delete Right** set the
action of each selected row; an override of a folder covers what lies below it. The plan is made again. An override a
row cannot take (a left orphan has nothing to copy to the left) is removed again with a notice once the comparison is
complete; one that names no row of the comparison (a file not there yet, a folder not scanned, a row a filter hides) is
kept, with one notice and a warning above the tree. The overrides are part
of the session (`[rules.sync.overrides]`): the session is changed until it is saved. Search ▸ **Next Conflict**
(Ctrl+N) and **Previous Conflict** (Ctrl+P) walk the conflicts.

## Running

Run Sync asks the questions of the plan in [Confirm an operation](topic:dialog-confirm), as `[file_operations]
sync_confirmations` says: the operation question with the counts, then per item a read-only or system target, a newer
file a mirror overwrites, an item without a recycle bin. Yes to All answers the rest. The sync then runs (Stop ends it
after its current item), its summary and the counts go to the log, a notice sums it up, the rows are read again and the
plan follows. With `beep_after_long_operations` a sync of 10 seconds or more beeps at its end.

Session ▸ **Compare Base Folders** opens a Folder Compare view of the same bases. Session ▸ **Swap Sides** exchanges the
sides and mirrors the options (Update Right becomes Update Left, overrides and the matrix too), so the same folders get
the same actions. Session ▸ **Folder Sync Info** adds the plan's counts to the statistics.

A view opened with `--wait` ends with the exit code of its last Run (0, or 100 when an item failed, did not run or the
sync was stopped); without a Run, 0 when the comparison completed, else 100. See [Exit codes](topic:exit-codes) and
[hikasa folder sync](topic:folder-sync), the same sync from the command line.
