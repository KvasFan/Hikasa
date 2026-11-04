# Browse

The **Browse** buttons of Hikasa open this dialog to pick a folder or a file: the **…** button of a
[Folder Compare view](topic:view-folder-compare)'s base folder field (Session ▸ Browse for Folder ▸ Left Side, Right
Side), the Script field of [Run Script](topic:dialog-script), and the package file of the
[Export Settings wizard](topic:wizard-export). It reads through Hikasa's own file access, so archives and every
supported location browse alike.

## Places

The list at the left: your home folder, the current folder, `/` (or every drive that exists on Windows), the view's
two base folders, then the folders you picked recently.

## The listing

- The field at the top is the folder shown; type a path and press Enter to go there. **Up** goes to the parent folder;
  inside an archive it goes up through the archive's folders and then out of the archive into the folder holding it.
- Folders come first, then the files by name. An archive (a file that Tools ▸ Options ▸ Archives knows) is shown with
  its own mark and opens as a folder on a double-click, so a member can be picked.
- Items whose names start with `.` are hidden unless **Show hidden items** is on (remembered for the next time).
- **Filter** narrows the list to the names that contain its text, ignoring case.
- A folder that cannot be read shows why in the list.

## OK

- Picking a **folder**: OK takes the selected folder, or the folder shown when none is selected. A base folder field
  takes it as if typed and accepted: the view compares again.
- Picking a **file**: OK or a double-click takes the selected file.
- **Save As**: type a name (or click a file to take its name); OK takes the name in the folder shown. A file that exists
  asks *Replace …?* first.

Cancel or Esc picks nothing. Every folder picked is remembered among the recent folders (`[mru] folders`), every file
among the recent files (`[mru] files`).
