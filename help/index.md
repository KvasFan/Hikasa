# Hikasa Help

Hikasa compares folders and files. Every function is available from the `hikasa` command line; the desktop
window `hikasa-gui` shows the same comparisons and is reached from the command line with `hikasa gui`.

## The command line

- [Using the command line](topic:cli): global flags, standard input, output files.
- [Exit codes](topic:exit-codes) and [JSON output](topic:json-output).
- [hikasa quick](topic:quick): compare two files, exit code only.
- [hikasa text compare](topic:text-compare): compare two text files.
- [hikasa text convert](topic:text-convert): trim trailing whitespace, convert tabs, line endings and encodings,
  in place or to another file.
- [hikasa text find](topic:text-find): find text in files and folders, and replace it in every file found.
- [hikasa table compare](topic:table-compare): compare two tables, cell by cell: every sheet of two workbooks
  (xlsx, xlsm, xlsb, xls, ods), CSV and other delimited or fixed-width text, or one of each;
  [Table formats](topic:table-formats).
- [hikasa hex compare](topic:hex-compare): compare two files byte by byte, aligned; [hikasa hex
  dump](topic:hex-dump) prints the bytes of a file and [hikasa hex find](topic:hex-find) finds text or bytes in files.
- [hikasa picture compare](topic:picture-compare): compare two pictures pixel by pixel, with a tolerance, rotations,
  Auto Scale and a difference image.
- [hikasa folder compare](topic:folder-compare): compare two folder trees.
- [hikasa format](topic:format): list, show, test and change file formats.
- [hikasa settings](topic:settings): read and change the settings files; export and import settings packages.
- [hikasa session](topic:session): named sessions, defaults, auto-saved sessions; run a saved comparison.
- [hikasa workspace](topic:workspace): saved sets of windows and tabs.
- [hikasa folder op](topic:folder-op): copy, move, delete, rename and more, on what a comparison shows.
- [hikasa folder sync](topic:folder-sync): update or mirror one folder from another, by a preset or a matrix.
- [hikasa report](topic:report): reports to a file, the clipboard, the printer or the browser; XML; links to file
  reports.
- [hikasa script](topic:script): run and check scripts: a batch of commands over one folder comparison.
- [hikasa vfs](topic:vfs): list, inspect and read any spec.
- [Archives](topic:archives): zip, tar and compressed streams as specs and folder sides.
- [hikasa gui](topic:gui): open views in the window and control it; [Using Hikasa with version
  control](topic:version-control): git, Mercurial, Subversion and Perforce wait for the window.
- [hikasa completions](topic:completions), [hikasa version](topic:version), [hikasa help](topic:help).

## The window

- [The main window](topic:main-window): menus, tabs, toolbar, shortcuts, help.
- [The Home view](topic:view-home): sessions tree, search, management panel.
- [Session Settings](topic:dialog-session-settings), [Save Session As](topic:dialog-save-session) and
  [Workspaces](topic:dialog-workspaces).
- [Options](topic:dialog-options), and the settings wizards [Export](topic:wizard-export),
  [Import](topic:wizard-import) and [Restore Factory Defaults](topic:wizard-restore).
- [The Folder Compare view](topic:view-folder-compare), with its file operations: [Confirm an
  operation](topic:dialog-confirm), [Delete](topic:dialog-delete), [Copy to Folder](topic:dialog-copy-to-folder),
  [Rename](topic:dialog-rename), [Touch](topic:dialog-touch), [New Folder](topic:dialog-new-folder),
  [Exclude](topic:dialog-exclude), [Compare Contents](topic:dialog-compare-contents),
  [Quick Compare](topic:dialog-quick-compare) and [Synchronize](topic:dialog-sync-preview).
- [The Folder Sync view](topic:view-folder-sync).
- [The Text Compare view](topic:view-text-compare): [Find and Replace](topic:panel-find-replace), [Find in
  Files](topic:dialog-find-in-files) and [its results](topic:panel-find-in-files), [Go To](topic:dialog-go-to), [the
  details panel](topic:panel-details), [Display Font](topic:dialog-display-font), [Text Compare
  Info](topic:dialog-text-info), [Copy File and Open Next](topic:dialog-copy-file-next).
- [The Text Edit view](topic:view-text-edit).
- [The Table Compare view](topic:view-table-compare): [Go To](topic:dialog-table-goto), [Table Compare
  Info](topic:dialog-table-info), [the report dialog](topic:dialog-table-report) and the [table rule
  tabs](topic:dialog-session-table).
- [The report dialog](topic:dialog-report) of every view kind: Write Report, View in Browser, Print Preview.
- [Quick Compare at startup](topic:dialog-quick-compare-startup): two files from the command line.
- [The Hex Compare view](topic:view-hex-compare): [Layout](topic:dialog-hex-layout), [Go To](topic:dialog-hex-go-to),
  [Find](topic:panel-hex-find), [the value panel](topic:panel-hex-values) and [Hex Compare Info](topic:dialog-hex-info).
- [The Picture Compare view](topic:view-picture-compare): [Picture Compare Info](topic:dialog-picture-info).
- [Scripts in the window](topic:dialog-script): Tools ▸ Run Script…, the status window, Stop.
- [File Formats](topic:dialog-file-formats): the formats in detection order; the [format
  editor](topic:dialog-format-editor), its [Grammar Item](topic:dialog-grammar-item) dialog and the [regex
  helper](topic:regex-helper).
