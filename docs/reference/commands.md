# Command Catalogue (default shortcuts)

Generated reference. Columns: command id, category (menu), caption, default shortcut, description.
Captions ending with "…" open a dialog. Commands without a caption are toolbar/gutter-only or dynamic (caption computed at runtime).
Shortcuts use Ctrl on Linux/Windows; on macOS Ctrl maps to Cmd unless noted.

## Global commands (all views)

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `SessionNewList` | Session | New Session |  | Opens empty session of picked type in new view. | implemented |
| `TabNew` | Session | New Tab | Ctrl+T | Opens Home view in new tab. | implemented |
| `WindowNew` | Session | New Window |  | Opens Home view in new window. | implemented |
| `SessionOpen` | Session | Open Session | Shift+Ctrl+O | Opens Home in new view. | implemented |
| `WorkspaceLoadList` | Session | Load Workspace |  | Restores picked saved view configuration. | implemented |
| `WorkspaceManager` | Session | Manage Workspaces... |  | Manages the saved view configurations that can be restored. | implemented |
| `WorkspaceSaveAs` | Session | Save Workspace As... |  | Stores current view configuration. | implemented |
| `CloseTab` | Session | Close Tab | Ctrl+W | Closes active tab. | implemented |
| `TabDuplicate` |  | Duplicate Tab |  | Opens current session again in new tab. | implemented |
| `TabMoveToNewWindow` |  | Move Tab to New Window |  | Moves current tab into new window. | implemented |
| `TabMoveToWindow` |  | Move Tab to Window |  | Moves current tab into picked window. | implemented |
| `TabCloseOthers` |  | Close Other Tabs |  | Closes all other tabs of this window. | implemented |
| `Exit` | Session | Exit | Ctrl+Q | Quits the application, prompts to save edits. | implemented |
| `Undo` | Edit |  | Ctrl+Z | Undoes previous action. | planned |
| `Redo` | Edit |  | Ctrl+Y | Redoes previously undone action. | planned |
| `Cut` | Edit | Cut | Ctrl+X | Cuts selection and puts it on Clipboard. | planned |
| `Copy` | Edit | Copy | Ctrl+C | Copies selection and puts it on Clipboard. | planned |
| `Paste` | Edit | Paste | Ctrl+V | Inserts Clipboard contents. | planned |
| `SelectAll` | Edit | Select All | Ctrl+A | Selects all visible items. | planned |
| `ProgramOptions` | Tools | Options... |  | Sets global preferences. | implemented |
| `FileFormatManager` | Tools | File Formats... |  | Manages file formats. | planned |
| `ProfileManager` | Tools | Profiles... |  | Manages profiles. | planned |
| `Export` | Tools | Export Settings... |  | Saves user settings as a file. | implemented |
| `Import` | Tools | Import Settings... |  | Loads user settings from a file. | implemented |
| `Restore` | Tools | Restore Factory Defaults... |  | Loads factory default settings. | implemented |
| `SaveSnapshot` | Tools | Save Snapshot... |  | Saves details of specified folder for future comparison. | planned |
| `EditTextFile` | Tools | Edit Text File |  | Opens new Text Edit view. | planned |
| `ViewPatch` | Tools | View Patch... |  | Opens picked patch file in new Text Patch view. | planned |
| `TabNext` | MacWindow |  | Ctrl+Tab | Selects tab after active one. | implemented |
| `TabPrev` | MacWindow |  | Shift+Ctrl+Tab | Selects tab before active one. | implemented |
| `HelpContents` | Help | Contents |  | Shows help file. | implemented |
| `ContextHelp` | Help | Context Sensitive Help | F1 | Shows context sensitive help for the active window. | implemented |
| `About` | Help | About |  | Displays program information, version number and copyright. | implemented |

## Folder Compare / Folder Merge

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `NameFilterEdit` | Session |  |  | Focuses on edit listing current name file filters. | planned |
| `NameFilter` | Session |  |  | Edits name file filters. | planned |
| `SpecsBack` | Session | Back | Alt+Left | Changes to previous base folders. | planned |
| `SpecsForward` | Session | Forward | Alt+Right | Changes to next base folders. | planned |
| `UpOneLevelList` | Session | Up One Level |  | Changes base folders on picked sides to their parent folders. | planned |
| `UpOneLevelLeft` | Session | Left Side |  | Changes left base folder to its parent folder. | planned |
| `UpOneLevelCenter` | Session | Center Side |  | Changes center base folder to its parent folder. | planned |
| `UpOneLevelRight` | Session | Right Side |  | Changes right base folder to its parent folder. | planned |
| `UpOneLevelOutput` | Session | Output Side |  | Changes output base folder to its parent folder. | planned |
| `MergeBaseView` | Session | Merge Base Folders |  | Merges base folders in new view. | planned |
| `SyncBaseView` | Session | Sync Base Folders |  | Synchronizes base folders in new view. | planned |
| `CompareToOutputList` | Session | Compare to Output |  | Compares picked side and output in new view. | planned |
| `CompareLeftToOutput` | Session | Left Side |  | Compares left and output in new view. | planned |
| `CompareCenterToOutput` | Session | Center Side |  | Compares center and output in new view. | planned |
| `CompareRightToOutput` | Session | Right Side |  | Compares right and output in new view. | planned |
| `QuickCompare` | Actions | Quick Compare |  | Quick compare selected files. | planned |
| `CompareTo` | Actions | Compare To... | F7 | Marks item for comparing in new view. | planned |
| `AlignWith` | Actions |  | F6 | Marks item for manual realignment. | planned |
| `Compare` | Actions | Compare Contents |  | Determines overall comparison state for selected items. | planned |
| `Merge` | Actions | Merge... |  | Performs automated merge of selected items. | planned |
| `CopyRight` | Actions | Copy to Right | Ctrl+R | Copies selected items from left to right. | planned |
| `CopyLeft` | Actions | Copy to Left | Ctrl+L | Copies selected items from right to left. | planned |
| `CopyPrompt` | Actions | Copy to Side... |  | Prompts which side to copy selected items to. | planned |
| `CopySide` | Actions |  |  | Copies selected items to other side. | planned |
| `CopyToOutput` | Actions | Copy to Output... |  | Copies selected items to output folder. | planned |
| `MoveRight` | Actions | Move to Right |  | Moves selected items from left to right. | planned |
| `MoveLeft` | Actions | Move to Left |  | Moves selected items from right to left. | planned |
| `MovePrompt` | Actions | Move to Side... |  | Prompts which side to move selected items to. | planned |
| `MoveSide` | Actions |  |  | Moves selected items to other side. | planned |
| `MoveBoth` | Actions | Exchange |  | Moves selected items to other side. | planned |
| `CopyTo` | Actions | Copy to Folder... |  | Copies selected items to another folder. | planned |
| `MoveTo` | Actions | Move to Folder... |  | Moves selected items to another folder. | planned |
| `Delete` | Actions | Delete |  | Deletes selected items. | planned |
| `PermanentDelete` | Actions |  | Shift+Del | Deletes selected items without moving them to the recycle bin. | planned |
| `Rename` | Actions | Rename | F2 | Renames selected items. | planned |
| `Attributes` | Actions | Attributes... |  | Sets the DOS attributes of selected files. | planned |
| `Touch` | Actions | Touch... |  | Sets timestamps of selected items. | planned |
| `NewFolder` | Actions | New Folder... | Ins | Creates a new folder. | planned |
| `Ignored` | Actions | Ignored |  | Ignores or unignores differences in selected items. | planned |
| `FileReport` | Actions | File Compare Report... |  | Produces file comparison report. | planned |
| `SyncList` | Actions | Synchronize |  | Synchronizes by picked method. | planned |
| `UpdateRight` | Actions | Update Right... |  | Synchronizes by copying newer and orphan items from left to right. | planned |
| `UpdateLeft` | Actions | Update Left... |  | Synchronizes by copying newer and orphan items from right to left. | planned |
| `UpdateBoth` | Actions | Update Both... |  | Synchronizes by copying newer and orphan items from left to right and from right to left. | planned |
| `MirrorRight` | Actions | Mirror to Right... | Shift+Ctrl+R | Synchronizes by replacing all different items on right, deleting any orphan items on right, and copying any left orphan items to right. | planned |
| `MirrorLeft` | Actions | Mirror to Left... | Shift+Ctrl+L | Synchronizes by replacing all different items on left, deleting any orphan items on left, and copying any right orphan items to left. | planned |
| `SelectAllFiles` | Edit | Select All Files | Shift+Ctrl+A | Selects all visible files. | planned |
| `SelectNewerList` | Edit | Select Newer |  | Selects all newer files on picked sides. | planned |
| `SelectNewerLeft` | Edit | Left Side |  | Selects all newer files on the left. | planned |
| `SelectNewerRight` | Edit | Right Side |  | Selects all newer files on the right. | planned |
| `SelectNewerBoth` | Edit | Both Sides |  | Selects all newer files on both sides. | planned |
| `SelectOrphanList` | Edit | Select Orphans |  | Selects all orphan files on picked sides. | planned |
| `SelectOrphanLeft` | Edit | Left Side |  | Selects all orphan files on the left. | planned |
| `SelectOrphanRight` | Edit | Right Side |  | Selects all orphan files on the right. | planned |
| `SelectOrphanBoth` | Edit | Both Sides |  | Selects all orphan files on both sides. | planned |
| `DiffNext` | Search | Next Difference | Ctrl+N | Positions at next difference. | implemented |
| `DiffPrev` | Search | Previous Difference | Ctrl+P | Positions at previous difference. | implemented |
| `FavCompareAll` | Favorites | Show All |  | Shows same, left orphan, right orphan, left newer, right newer and difference items. | implemented |
| `FavDiff` | DropDown | Show Differences |  | Shows left orphan, right orphan, left newer, right newer and difference items. | implemented |
| `FavSame` | Favorites | Show Same |  | Shows same items. | implemented |
| `FavNoOrphan` | DropDown | Show No Orphans |  | Shows same, left newer, right newer and difference items. | implemented |
| `FavDiffNoOrphan` | DropDown | Show Differences but No Orphans |  | Shows left newer, right newer and difference items. | implemented |
| `FavOrphan` | DropDown | Show Orphans |  | Shows left orphan and right orphan items. | implemented |
| `FavLeftNewer` | DropDown | Show Left Newer |  | Shows left newer items. | implemented |
| `FavRightNewer` | DropDown | Show Right Newer |  | Shows right newer items. | implemented |
| `FavLeftNewerOrphan` | DropDown | Show Left Newer and Left Orphans |  | Shows left orphan and left newer items. | implemented |
| `FavRightNewerOrphan` | DropDown | Show Right Newer and Right Orphans |  | Shows right orphan and right newer items. | implemented |
| `FavLeftOrphan` | DropDown | Show Left Orphans |  | Shows left orphan items. | implemented |
| `FavRightOrphan` | DropDown | Show Right Orphans |  | Shows right orphan items. | implemented |
| `FavCompareNone` | View | Show None |  | Hides all files. | implemented |
| `FavMergeAll` | Favorites | Show All |  | Shows unchanged, same change, left add, right add, left delete, right delete, left change, right change, difference change and conflict items. | planned |
| `FavChanged` | DropDown | Show Changes |  | Shows left add, right add, left delete, right delete, left change, right change and conflict items. | planned |
| `FavConflict` | DropDown | Show Conflicts |  | Shows conflict items. | planned |
| `FavLeft` | DropDown | Show Left Changes |  | Shows left add, left delete and left change items. | planned |
| `FavRight` | DropDown | Show Right Changes |  | Shows right add, right delete and right change items. | planned |
| `FavMergeable` | DropDown | Show Mergeable |  | Shows same change and difference change items. | planned |
| `FavUnchanged` | Favorites | Show Unchanged |  | Shows unchanged items. | planned |
| `FavMergeNone` | View | Show None |  | Shows no items. | planned |
| `TogSame` | Toggles |  |  | Shows or hides same items. | implemented |
| `TogLeftOrphan` | Toggles |  |  | Shows or hides left orphan items. | implemented |
| `TogRightOrphan` | Toggles |  |  | Shows or hides right orphan items. | implemented |
| `TogLeftNewer` | Toggles |  |  | Shows or hides left newer items. | implemented |
| `TogRightNewer` | Toggles |  |  | Shows or hides right newer items. | implemented |
| `TogDiff` | Toggles |  |  | Shows or hides difference items. | implemented |
| `TogNoMod` | Toggles |  |  | Shows or hides unchanged items. | planned |
| `TogSameMod` | Toggles |  |  | Shows or hides same change items. | planned |
| `TogLeftAdd` | Toggles |  |  | Shows or hides left add items. | planned |
| `TogRightAdd` | Toggles |  |  | Shows or hides right add items. | planned |
| `TogLeftDel` | Toggles |  |  | Shows or hides left delete items. | planned |
| `TogRightDel` | Toggles |  |  | Shows or hides right delete items. | planned |
| `TogLeftMod` | Toggles |  |  | Shows or hides left change items. | planned |
| `TogRightMod` | Toggles |  |  | Shows or hides right change items. | planned |
| `TogDiffMod` | Toggles |  |  | Shows or hides mergeable items. | planned |
| `TogConflict` | Toggles |  |  | Shows or hides conflict items. | planned |
| `DirShowAll` | View | Always Show Folders |  | Shows all folders, ignoring the display filter. | implemented |
| `DirCompareStructure` | View | Compare Files and Folder Structure |  | Shows folders if they, or items they contain, match the display filter. | implemented |
| `DirCompareFilesOnly` | View | Only Compare Files |  | Shows folders only if they contain files matching the display filter. | implemented |
| `DirFlatten` | View | Ignore Folder Structure |  | Compares all files as if they were in a single folder. | planned |
| `ShowCenter` | View | Center Pane |  | Shows or hides center pane. | planned |

## Folder views — shared

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `Swap` |  |  |  | Exchanges left, right base folders. | planned |
| `BrowseList` | Session | Browse for Folder |  | Browses for an existing folder on picked side. | planned |
| `BrowseLeft` | Session | Left Side |  | Browses for an existing folder on left side. | planned |
| `BrowseCenter` | Session | Center Side |  | Browses for an existing folder on center side. | planned |
| `BrowseRight` | Session | Right Side |  | Browses for an existing folder on right side. | planned |
| `BrowseOutput` | Session | Output Side |  | Browses for an existing folder on output side. | planned |
| `Stop` | Session |  |  | Terminates comparison. | implemented |
| `Expand` | Actions | Open Folder | Right | Opens selected folder. | implemented |
| `Collapse` | Actions | Close Folder | Left | Closes selected folder. | implemented |
| `ExpandSubfolders` | Actions | Open Subfolders |  | Opens selected folders and all their subfolders. | implemented |
| `CollapseSubfolders` | Actions | Close Subfolders |  | Closes selected folders and their subfolders. | implemented |
| `SetAsBaseFolders` | Actions |  |  | Sets selected folders as base folders. | planned |
| `SetAsOtherBaseFolder` | Actions | Set as Base on Other Side |  | Sets selected folder as base folder on other side. | planned |
| `Open` | Actions |  | Enter | Opens selected items in new view. | implemented |
| `OpenWithList` | Actions | Open With |  | Opens selected items in new view of picked type. | planned |
| `AssocApp` | Actions | Associated Application |  | Opens selected items with associated application. | planned |
| `Exclude` | Actions | Exclude |  | Filters selected items to remove them from comparison. | planned |
| `CopyFilenames` | Actions | Copy Filename |  | Copies selected items' paths to Clipboard. | planned |
| `RefreshSelection` | Actions | Refresh Selection | Shift+F5 | Refreshes selected items. | planned |
| `ExplorerList` | Actions | File Manager |  | Shows file manager context menu for selected items. | planned |
| `ExpandAll` | Edit | Expand All |  | Opens all folders. | implemented |
| `CollapseAll` | Edit | Collapse All |  | Closes all folders. | implemented |
| `SelectInvert` | Edit | Invert Selection |  | Selects all items not currently selected, and deselects all items currently selected. | planned |
| `Refresh` | Edit | Refresh | F5 | Refreshes open folders. | planned |
| `FullRefresh` | Edit | Full Refresh | Ctrl+F5 | Refreshes entire comparison. | implemented |
| `ExistsNext` | Search |  | Alt+Down | Positions at next filename, skipping over gaps. | planned |
| `ExistsPrev` | Search |  | Alt+Up | Positions at previous filename, skipping over gaps. | planned |
| `FindFilename` | Search | Find Filename... | Ctrl+F | Finds filename matching search string. | planned |
| `FindFilenameNext` | Search | Find Next Filename | F3 | Finds next filename matching search string. | planned |
| `FindFilenamePrev` | Search | Find Previous Filename | Shift+F3 | Finds previous filename matching search string. | planned |
| `SuppressFilters` | View | Suppress Filters |  | Temporarily suppresses display and file filters. | implemented |
| `ColList` | View | Columns |  | Shows or hides picked column. | planned |
| `Legend` | View | Legend | Ctrl+Alt+L | Shows or hides legend window. | planned |
| `ShowLog` | View | Log |  | Shows or hides log panel. | implemented |

Enter, Right and Left (keys without Ctrl or Alt that focused controls use themselves) act on the folder tree
only while the tree has the keyboard focus; on a focused button, list or check box they keep their usual
meaning (M1 wave 6). In M1, `Open` opens the selected file pair in a Text Compare view (FC-062).

## Folder Sync

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `Run` | Session |  |  | Performs all sync operations. | planned |
| `CompareBaseView` | Session | Compare Base Folders |  | Compares base folders in new view. | planned |
| `SyncLeave` | Actions | Leave Alone |  | Changes sync operation to leave alone. | planned |
| `SyncCopyLeft` | Actions | Copy Right to Left |  | Changes sync operation to copy right to left. | planned |
| `SyncCopyRight` | Actions | Copy Left to Right |  | Changes sync operation to copy left to right. | planned |
| `SyncDeleteLeft` | Actions | Delete Left |  | Changes sync operation to delete left. | planned |
| `SyncDeleteRight` | Actions | Delete Right |  | Changes sync operation to delete right. | planned |
| `ConflictNext` | Search | Next Conflict | Ctrl+N | Positions at next conflict. | planned |
| `ConflictPrev` | Search | Previous Conflict | Ctrl+P | Positions at previous conflict. | planned |

## File views — shared

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `FileFormat` | Session |  |  | Edits current File Format. | planned |
| `Reload` | Session | Reload Files | F5 | Reloads displayed files. | implemented |
| `Recompare` | Session | Recompare Files | Ctrl+F5 | Recompares displayed files without reloading them. | planned |
| `CompareUsingViewList` | Session | Compare Files Using |  | Compares files in picked new view. | planned |
| `MergeView` | Session | Merge Files |  | Merges files in new view. | planned |
| `CompareToOutputList` | Session | Compare to Output |  | Compares picked side and output in new view. | planned |
| `CompareLeftToOutput` | Session | Left Side |  | Compares left and output in new view. | planned |
| `CompareCenterToOutput` | Session | Center Side |  | Compares center and output in new view. | planned |
| `CompareRightToOutput` | Session | Right Side |  | Compares right and output in new view. | planned |
| `Open` | File | Open File... | Ctrl+O | Opens existing file into picked pane. | planned |
| `OpenLocal` | File | Open File on File System... |  | Opens existing local file into picked pane. | planned |
| `OpenProfile` | File | Open File Using Profile... |  | Opens existing file into picked pane using profile. | planned |
| `OpenClipboard` | File | Open Clipboard | Shift+Ctrl+V | Opens Clipboard contents into picked pane. | planned |
| `OpenWithList` | File | Open With |  | Opens picked file with picked application. | planned |
| `AssocApp` | File | Associated Application |  | Opens picked file with associated application. | planned |
| `Save` | File | Save File | Ctrl+S | Saves picked file which has been modified. | planned |
| `SaveAs` | File | Save File As... | F12 | Saves picked file with new name. | planned |
| `SaveAsLocal` | File | Save File on File System As... |  | Saves picked file with new name on file system. | planned |
| `SaveAsProfile` | File | Save File Using Profile As... |  | Saves picked file with new name using profile. | planned |
| `ExplorerList` | File | File Manager |  | Shows file manager context menu for picked file. | planned |
| `DiffItemNext` | Search | Next Difference Item | Ctrl+N | Positions at next difference item. | planned |
| `DiffItemPrev` | Search | Previous Difference Item | Ctrl+P | Positions at previous difference item. | planned |
| `DiffFileNext` | Search | Next Difference Files | Ctrl+M | Opens folder compare's next difference files. | planned |
| `DiffFilePrev` | Search | Previous Difference Files | Shift+Ctrl+M | Opens folder compare's previous difference files. | planned |
| `CopyFileRight` | Search | Copy File to Right and Open Next Difference | Ctrl+Alt+R | Copies left file to right and opens folder compare's next difference. | planned |
| `CopyFileLeft` | Search | Copy File to Left and Open Next Difference | Ctrl+Alt+L | Copies right file to left and opens folder compare's next difference. | planned |
| `TakeFileLeft` | Search | Take Left File and Open Next Difference | Ctrl+Alt+L | Takes left file and opens folder merge's next difference. | planned |
| `TakeFileRight` | Search | Take Right File and Open Next Difference | Ctrl+Alt+R | Takes right file and opens folder merge's next difference. | planned |
| `ExistsNext` | Search |  | Alt+Down | Positions at next item, skipping over gaps. | planned |
| `ExistsPrev` | Search |  | Alt+Up | Positions at previous item, skipping over gaps. | planned |
| `Find` | Search | Find... | Ctrl+F | Finds search string. | planned |
| `FindNext` | Search | Find Next | F3 | Finds next occurrence of search string. | planned |
| `FindPrev` | Search | Find Previous | Shift+F3 | Finds previous occurrence of search string. | planned |
| `GoLeft` | Search |  | Alt+Left | Positions in pane left of current one. | planned |
| `GoRight` | Search |  | Alt+Right | Positions in pane right of current one. | planned |
| `GoNext` | Search |  | F6 | Positions in pane after current one. | planned |
| `GoPrev` | Search |  | Shift+F6 | Positions in pane before current one. | planned |
| `FavCompareAll` | Favorites | Show All |  | Shows same, left orphan, right orphan and difference items. | planned |
| `FavDiff` | Favorites | Show Differences |  | Shows left orphan, right orphan and difference items. | planned |
| `FavSame` | Favorites | Show Same |  | Shows same items. | planned |
| `FavCompareNone` | View | Show None |  | Shows no items. | planned |
| `TogSame` | Toggles |  |  | Shows or hides same items. | planned |
| `TogLeftOrphan` | Toggles |  |  | Shows or hides left orphan items. | planned |
| `TogRightOrphan` | Toggles |  |  | Shows or hides right orphan items. | planned |
| `TogDiff` | Toggles |  |  | Shows or hides difference items. | planned |
| `ShowWS` | View | Visible Whitespace |  | Shows or hides visible spaces, tabs. | planned |
| `DisplayFont` | View | Display Font... |  |  | planned |
| `DisplayFontIncrease` | View | Increase Display Font Size | Ctrl+Num + |  | implemented |
| `DisplayFontDecrease` | View | Decrease Display Font Size | Ctrl+Num - |  | implemented |
| `DisplayFontReset` | View | Reset Display Font Size | Ctrl+Num 0 |  | implemented |
| `SideBySide` | View | Side-by-Side Layout |  | Arranges panes left and right of each other. | planned |
| `OverUnder` | View | Over-Under Layout |  | Arranges panes above and below each other. | planned |
| `ShowSpecInfo` | View | File Info |  | Shows or hides file information. | planned |

## Hex Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible bytes. | planned |
| `DiffItemNext` |  | Next Difference Bytes |  | Positions at next difference bytes. | planned |
| `DiffItemPrev` |  | Previous Difference Bytes |  | Positions at previous difference bytes. | planned |
| `ExistsNext` |  |  |  | Positions at next existing byte, skipping over gaps. | planned |
| `ExistsPrev` |  |  |  | Positions at previous existing byte, skipping over gaps. | planned |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference bytes. | planned |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference bytes. | planned |
| `FavSame` |  |  |  | Shows same bytes. | planned |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected or current bytes to right pane. | planned |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected or current bytes to left pane. | planned |
| `CopySide` | Edit |  |  | Copies selected or current bytes to pane on other side. | planned |
| `Delete` | Edit | Delete |  | Deletes selection. | planned |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. | planned |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified byte. | planned |
| `LittleEndianList` | View | Little Endian Values |  | Displays little endian values in picked way. | planned |
| `BigEndianList` | View | Big Endian Values |  | Displays big endian values in picked way. | planned |
| `Layout` | View | Layout... |  | Configures layout options. | planned |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. | planned |
| `Overwriting` | View |  | Ins | Typing replaces bytes instead of inserting them. | planned |

## Home view

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `HomeCompare` | Session | Compare | Enter (in a path field) | Compares the two typed paths; the comparison replaces the Home view (M1 addition, UI-001). | implemented |
| `Find` | Search | Find... | Ctrl+F | Filters the sessions tree by name, folder or description. | implemented |
| `ShowManage` | View | Show Session Management |  | Shows or hides management panel. | implemented |
| `MaximizeManage` | View | Maximize Session Management |  | Maximizes or restores management panel. | implemented |
| `TreeOpen` |  | Open | Enter | Opens the selected session. | implemented |
| `TreeOpenAsNew` |  | Open as New |  | Opens the selected session as a new unnamed one. | implemented |
| `TreeOpenAll` |  | Open All |  | Opens every session directly in the selected folder. | implemented |
| `TreeNewSession` |  | New Session... |  | Creates a session in the selected folder. | implemented |
| `TreeNewFolder` |  | New Folder... |  | Creates a folder in the selected folder. | implemented |
| `TreeClear` |  | Clear |  | Resets the selected session to the defaults of its type. | implemented |
| `TreeDelete` |  | Delete... | Del | Deletes the selected session or folder. | implemented |
| `TreeSaveAs` |  | Save As... |  | Saves a copy of the selected session under a new name. | implemented |
| `TreeRename` |  | Rename | F2 | Renames the selected session or folder. | implemented |
| `TreeEdit` |  | Session Settings... |  | Edits the selected session or the defaults of a type. | implemented |
| `TreeExpandAll` |  | Expand All |  | Opens every folder of the tree. | implemented |
| `TreeCollapseAll` |  | Collapse All |  | Closes every folder of the tree. | implemented |
| `TreeFavorite` |  | Favorite |  | Pins the selected session to the top of its folder. | implemented |
| `ShowWeb` | View | Show Web Resources |  | Shows or hides web panel. | planned |

## Media Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `Play2` | Session |  |  | Plays both audio tracks. | planned |
| `Play` | File |  |  | Plays audio track. | planned |
| `IsSig` | View |  |  | Marks selected item as important or unimportant. | planned |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. | planned |

## Picture Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `DiffTolerance` | View | Tolerance Mode |  | Detects pixel differences greater than configurable tolerance. | planned |
| `DiffRange` | View | Mismatch Range Mode |  | Measures degree of pixel difference. | planned |
| `DiffBlend` | View | Blend Mode |  | Blends pictures based on configurable percentage. | planned |
| `AutoScale` | View | Auto Scale |  | Makes smaller picture as large as other picture. | planned |
| `RotateRight` | View | Rotate Clockwise |  | Spins current picture to right by 90 degrees. | planned |
| `RotateLeft` | View | Rotate Counterclockwise |  | Spins current picture to left by 90 degrees. | planned |
| `FlipHorz` | View | Flip Horizontally |  | Swaps pixels of current picture relative to its horizontal center. | planned |
| `FlipVert` | View | Flip Vertically |  | Swaps pixels of current picture relative to its vertical center. | planned |
| `OffsetReset` | View | Reset Difference Offset |  | Realigns top left corner of both pictures. | planned |
| `CompareTags` | View | Compare Metadata |  | Compares and shows metadata. | planned |
| `BlendToggle` | View | Blend Toggle | F8 | Switches between left and right pictures. | planned |
| `FullScreen` | View | Full Screen | F11 | Makes current picture show as large as possible. | planned |
| `ZoomList` | View | Zoom |  | Makes picture appear to change size in picked way. | planned |
| `ZoomIn` | View | Zoom In | Ctrl+Num + | Makes pictures appear to be larger. | planned |
| `ZoomOut` | View | Zoom Out | Ctrl+Num - | Makes pictures appear to be smaller. | planned |
| `ZoomFit` | View | Fit to Window |  | Makes pictures appear to fill panes. | planned |

## Registry Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `Swap` |  |  |  | Exchanges left, right registries. | planned |
| `Reload` |  | Reload |  | Reloads displayed registries. | planned |
| `Recompare` |  | Recompare |  | Recompares displayed registries without reloading them. | planned |
| `Open` |  | Open... |  | Opens registry into picked pane. | planned |
| `OpenLocal` |  | Open Registry File on File System... |  | Opens existing local registry file into picked pane. | planned |
| `OpenProfile` |  | Open Registry File Using Profile... |  | Opens existing registry file into picked pane using profile. | planned |
| `UpOneLevelList` | Session | Up One Level |  | Changes base keys on picked sides to their parent keys. | planned |
| `UpOneLevelLeft` | Session | Left Side |  | Changes left base key to its parent key. | planned |
| `UpOneLevelRight` | Session | Right Side |  | Changes right base key to its parent key. | planned |
| `UpOneLevelBoth` | Session | Both Sides |  | Changes both base keys to their parent keys. | planned |
| `OpenReg` | File | Open Registry... |  | Opens live registry into picked pane. | planned |
| `SetAsBases` | Edit |  |  | Sets selected keys as base keys. | planned |
| `SetBothAsBases` | Edit | Set Both as Base Keys |  | Sets selected key and key on other side as base keys. | planned |
| `SetAsOtherBase` | Edit | Set as Base Key on Other Side |  | Sets selected key as base key on other side. | planned |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected items to right pane. | planned |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected items to left pane. | planned |
| `CopySide` | Edit |  |  | Copies selected items to pane on other side. | planned |
| `Delete` | Edit | Delete |  | Deletes selected items. | planned |
| `Rename` | Edit | Rename | F2 | Renames item. | planned |
| `NewKey` | Edit | New Key |  | Adds a new key. | planned |
| `NewVal` | Edit | New Value | Ins | Adds a new value. | planned |
| `Modify` | Edit | Modify... |  | Modifies value. | planned |
| `CopyKeyName` | Edit | Copy Key Name |  | Copies key's name to Clipboard. | planned |
| `ExportKey` | Edit | Export... |  | Saves key as registry file. | planned |
| `ExportAll` | Edit | Export All... |  | Saves all keys as registry file. | planned |
| `ExpandAll` | View | Expand All |  | Opens all items. | planned |
| `CollapseAll` | View | Collapse All |  | Closes all items. | planned |
| `ShowTextDetails` | View | Text Details |  | Shows or hides text details panel. | planned |
| `ShowHexDetails` | View | Hex Details |  | Shows or hides hexadecimal details panel. | planned |

## Session commands (all session views)

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `SessionHome` | Session | Go to Home | Alt+Home | Replaces current view with Home view. | implemented |
| `SessionsCombo` | Session |  |  | Focuses on dropdown button listing sessions that can be opened in this view. | implemented |
| `SessionSave` | Session | Save Session | Shift+Ctrl+S | Saves this session. | implemented |
| `SessionSaveAs` | Session | Save Session As... |  | Saves this session with new name. | implemented |
| `SessionEdit` | Session | Session Settings... |  | Edits current session settings. | implemented |
| `SessionLocked` | Session | Locked |  | Locks or unlocks current session. | implemented |
| `SessionClear` | Session | Clear Session | Shift+Ctrl+C | Replaces current view with empty session. | planned |
| `Rules` | Session |  |  | Edits current Rules definitions. | planned |
| `Swap` | Session | Swap Sides |  | Exchanges left, right files. | planned |
| `Report` | Session | <Type> Report... |  | Produces <type> report. | planned |
| `Info` | Session | <Type> Info | Ctrl+I | Displays <type> statistics. | planned |
| `CompareParents` | Session | Compare Parent Folders |  | Compares folders containing current items in new view. | planned |
| `MergeParents` | Session | Merge Parent Folders |  | Merges folders containing current items in new view. | planned |
| `InsigIgnored` | View | Ignore Unimportant Differences |  | Treats unimportant differences as same. | implemented |
| `SameModIgnored` | View | Ignore Same Changes |  | Treats same changes as same. | planned |
| `ShowToolbar` | View | Toolbar |  | Shows or hides toolbar. | implemented |

## Table Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible rows. | planned |
| `DiffItemNext` |  | Next Difference Row |  | Positions at next difference row. | planned |
| `DiffItemPrev` |  | Previous Difference Row |  | Positions at previous difference row. | planned |
| `ExistsNext` |  |  |  | Positions at next existing row, skipping over gaps. | planned |
| `ExistsPrev` |  |  |  | Positions at previous existing row, skipping over gaps. | planned |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference rows. | planned |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference rows. | planned |
| `FavSame` |  |  |  | Shows same rows. | planned |
| `FavCompareNone` |  |  |  | Shows no rows. | planned |
| `TogSame` |  |  |  | Shows or hides same rows. | planned |
| `TogLeftOrphan` |  |  |  | Shows or hides left orphan rows. | planned |
| `TogRightOrphan` |  |  |  | Shows or hides right orphan rows. | planned |
| `TogDiff` |  |  |  | Shows or hides difference rows. | planned |
| `AlignWith` | Edit | Align With... | F7 | Marks selected rows for manual realignment. | planned |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected rows to right pane. | planned |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected rows to left pane. | planned |
| `CopySide` | Edit |  |  | Copies selected rows to pane on other side. | planned |
| `CopyCellRight` | Edit | Copy Cells to Right | Shift+Ctrl+R | Copies selected cells to right pane. | planned |
| `CopyCellLeft` | Edit | Copy Cells to Left | Shift+Ctrl+L | Copies selected cells to left pane. | planned |
| `CopyCellSide` | Edit |  |  | Copies selected cells to pane on other side. | planned |
| `Delete` | Edit | Delete |  | Deletes selection. | planned |
| `DeleteCol` | Edit | Delete Column |  | Deletes selected columns. | planned |
| `InsertItemBefore` | Edit | Insert Row Before | Shift+Ctrl+Enter | Inserts new rows before selected ones. | planned |
| `InsertItemAfter` | Edit | Insert Row After | Ctrl+Enter | Inserts new rows after selected ones. | planned |
| `InsertColBefore` | Edit | Insert Column Before |  | Inserts new columns before selected ones. | planned |
| `InsertColAfter` | Edit | Insert Column After |  | Inserts new columns after selected ones. | planned |
| `Modify` | Edit | Modify... | F2 | Modifies cell text. | planned |
| `Sort` | Edit | Sort... |  | Sorts selected data. | planned |
| `DiffNext` | Search | Next Difference Cell | Shift+Ctrl+N | Positions at next difference cell. | planned |
| `DiffPrev` | Search | Previous Difference Cell | Shift+Ctrl+P | Positions at previous difference cell. | planned |
| `DiffSheetNext` | Search | Next Difference Sheet |  | Positions at next difference sheet. | planned |
| `DiffSheetPrev` | Search | Previous Difference Sheet |  | Positions at previous difference sheet. | planned |
| `EditNext` | Search | Next Edit |  | Positions at next edited text. | planned |
| `EditPrev` | Search | Previous Edit |  | Positions at previous edited text. | planned |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. | planned |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified row and column. | planned |
| `ColHideSame` | View | Hide Same Columns |  | Hides same columns. | planned |
| `ColDetectSize` | View | Resize Columns to Fit |  | Makes columns wide enough for data to be fully visible. | planned |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. | planned |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. | planned |

## Text Compare / Text Merge / Text Edit / Text Patch

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible lines. | planned |
| `ExistsNext` |  |  |  | Positions at next existing line, skipping over gaps. | planned |
| `ExistsPrev` |  |  |  | Positions at previous existing line, skipping over gaps. | planned |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference lines. | implemented |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference lines. | implemented |
| `FavSame` |  |  |  | Shows same lines. | implemented |
| `FavCompareNone` |  |  |  | Shows no lines. | implemented |
| `TogSame` |  |  |  | Shows or hides same lines. | implemented |
| `TogLeftOrphan` |  |  |  | Shows or hides left orphan lines. | implemented |
| `TogRightOrphan` |  |  |  | Shows or hides right orphan lines. | implemented |
| `TogDiff` |  |  |  | Shows or hides difference lines. | implemented |
| `ApplyPatch` | File | Apply Patch |  | Applies patch file. | planned |
| `AlignWith` | Edit | Align With... | F7 | Marks selected lines for manual realignment. | planned |
| `Isolate` | Edit | Isolate |  | Realigns comparison so that selected lines are by themselves. | planned |
| `Replacement` | Edit | Replacement... |  | Adds item to replacement list based on current difference. | planned |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected lines or current section to right pane. | planned |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected lines or current section to left pane. | planned |
| `CopySide` | Edit |  |  | Copies selected lines or current section to pane on other side. | planned |
| `CopyItemRight` | Edit | Copy Line to Right | Shift+Ctrl+R | Copies current line to right pane. | planned |
| `CopyItemLeft` | Edit | Copy Line to Left | Shift+Ctrl+L | Copies current line to left pane. | planned |
| `CopyItemSide` | Edit |  |  | Copies current line to pane on other side. | planned |
| `Conflict` | Edit | Conflict |  | Clears or sets conflict for selected lines or current section. | planned |
| `TakeLeft` | Edit | Take Left | Ctrl+L | Takes selected lines or current section from left pane. | planned |
| `TakeCenter` | Edit | Take Center |  | Takes selected lines or current section from center pane. | planned |
| `TakeRight` | Edit | Take Right | Ctrl+R | Takes selected lines or current section from right pane. | planned |
| `TakeLeftRight` | Edit | Take Left Then Right | Ctrl+B | Takes selected lines or current section from left then right panes. | planned |
| `TakeRightLeft` | Edit | Take Right Then Left | Shift+Ctrl+B | Takes selected lines or current section from right then left panes. | planned |
| `TakeItemLeft` | Edit | Take Left Line | Shift+Ctrl+L | Takes current line from left pane. | planned |
| `TakeItemCenter` | Edit | Take Center Line |  | Takes current line from center pane. | planned |
| `TakeItemRight` | Edit | Take Right Line | Shift+Ctrl+R | Takes current line from right pane. | planned |
| `IncreaseIndent` | Edit | Increase Indent | Shift+Ctrl+I | Increases indentation of selected lines. | planned |
| `DecreaseIndent` | Edit | Decrease Indent | Shift+Ctrl+U | Decreases indentation of selected lines. | planned |
| `Delete` | Edit | Delete |  | Deletes selection. | planned |
| `DeleteItemBegin` | Edit | Delete to Start of Line | Shift+Ctrl+BkSp | Deletes to start of current line. | planned |
| `DeleteItemEnd` | Edit | Delete to End of Line | Shift+Ctrl+Del | Deletes to end of current line. | planned |
| `DeleteWord` | Edit | Delete Word |  | Deletes current word. | planned |
| `DeleteWordBegin` | Edit | Delete to Start of Word | Ctrl+BkSp | Deletes to start of current word. | planned |
| `DeleteWordEnd` | Edit | Delete to End of Word | Ctrl+Del | Deletes to end of current word. | planned |
| `InsertItemBefore` | Edit | Insert Line Before | Shift+Ctrl+Enter | Inserts new line before current one. | planned |
| `InsertItemAfter` | Edit | Insert Line After | Ctrl+Enter | Inserts new line after current one. | planned |
| `SelectSection` | Edit | Select Section | Ctrl+D | Selects all lines in current section. | planned |
| `CompareToClipboard` | Edit | Compare Selection to Clipboard |  | Compares selected text to clipboard in new view. | planned |
| `ConvertList` | Edit | Convert File |  | Performs picked file conversion. | planned |
| `TrimTrailWS` | Edit | Trim Trailing Whitespace |  | Strips off spaces and tab characters at the end of lines. | planned |
| `LeadSpaceToTab` | Edit | Leading Spaces to Tabs |  | Replaces spaces at the beginning of lines with tab characters. | planned |
| `TabToSpace` | Edit | Tabs to Spaces |  | Replaces tab characters with spaces. | planned |
| `EndingStyleList` | Edit | Line Endings |  | Converts line endings to picked style. | planned |
| `ToWindows` | Edit | To Windows/DOS |  | Converts line endings to Windows/DOS style. | planned |
| `ToUnix` | Edit | To Unix |  | Converts line endings to Unix style. | planned |
| `ConflictSectionOffNext` | Search | Clear Conflict Section, Next |  | Clears conflict for current section and positions at next conflict section. | planned |
| `ConflictSectionNext` | Search | Next Conflict Section | Alt+N | Positions at next conflict section. | planned |
| `ConflictSectionPrev` | Search | Previous Conflict Section | Alt+P | Positions at previous conflict section. | planned |
| `DiffNext` | Search | Next Difference | Shift+Ctrl+N | Positions at next difference text. | planned |
| `DiffPrev` | Search | Previous Difference | Shift+Ctrl+P | Positions at previous difference text. | planned |
| `DiffSectionNext` | Search | Next Difference Section | Ctrl+N | Positions at next unignored difference section. | implemented |
| `DiffSectionPrev` | Search | Previous Difference Section | Ctrl+P | Positions at previous unignored difference section. | implemented |
| `DiffReplacedNext` | Search | Next Replacement | Shift+Ctrl+Alt+N | Positions at next replacement text. | planned |
| `DiffReplacedPrev` | Search | Previous Replacement | Shift+Ctrl+Alt+P | Positions at previous replacement text. | planned |
| `TakenLeftNext` | Search | Next Left Taken |  | Positions at next range of lines where left was taken. | planned |
| `TakenLeftPrev` | Search | Previous Left Taken |  | Positions at previous range of lines where left was taken. | planned |
| `TakenRightNext` | Search | Next Right Taken |  | Positions at next range of lines where right was taken. | planned |
| `TakenRightPrev` | Search | Previous Right Taken |  | Positions at previous range of lines where right was taken. | planned |
| `EditNext` | Search | Next Edit |  | Positions at next edited text. | planned |
| `EditPrev` | Search | Previous Edit |  | Positions at previous edited text. | planned |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. | planned |
| `FindInFiles` | Search | Find in Files... | Shift+Ctrl+F | Finds search string in multiple files. | planned |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified line and column. | planned |
| `BookmarkToggleList` | Search | Toggle Bookmark |  | Sets or clears picked bookmark. | planned |
| `BookmarkGoToList` | Search | Go to Bookmark |  | Positions at picked bookmark. | planned |
| `BookmarkClear` | Search | Clear Bookmarks |  | Removes all bookmarks. | planned |
| `GoInput` | Search |  | Alt+PgUp | Positions in input pane. | planned |
| `GoOutput` | Search |  | Alt+PgDn | Positions in output pane. | planned |
| `FavMergeAll` | Favorites | Show All |  | Shows unchanged, same change, left change, right change, different change and conflict lines. | planned |
| `FavChanged` | DropDown | Show Changes |  | Shows same change, left change, right change, different change and conflict lines. | planned |
| `FavConflict` | DropDown | Show Conflicts |  | Shows conflict lines. | planned |
| `FavLeft` | DropDown | Show Left Changes |  | Shows same change, left change, different change and conflict lines. | planned |
| `FavRight` | DropDown | Show Right Changes |  | Shows same change, right change, different change and conflict lines. | planned |
| `FavMergeable` | DropDown | Show Mergeable |  | Shows same change, left change, right change and different change lines. | planned |
| `FavUnchanged` | Favorites | Show Unchanged |  | Shows unchanged lines. | planned |
| `FavMergeNone` | View | Show None |  | Shows no lines. | planned |
| `TogNoMod` | Toggles |  |  | Shows or hides unchanged lines. | planned |
| `TogSameMod` | Toggles |  |  | Shows or hides same change lines. | planned |
| `TogLeftMod` | Toggles |  |  | Shows or hides left change lines. | planned |
| `TogRightMod` | Toggles |  |  | Shows or hides right change lines. | planned |
| `TogDiffMod` | Toggles |  |  | Shows or hides different change lines. | planned |
| `TogConflict` | Toggles |  |  | Shows or hides conflict lines. | planned |
| `ShowContext` | View | Show Context |  | Shows lines surrounding difference. | implemented |
| `FavorLeft` | View | Favor Left Changes |  | Suppresses highlighting of left-only changes in output. | planned |
| `FavorRight` | View | Favor Right Changes |  | Suppresses highlighting of right-only changes in output. | planned |
| `Ignored` | View | Ignored |  | Ignores or unignores differences in selected lines or current section. | planned |
| `ShowSyntaxHighlighting` | View | Syntax Highlighting |  | Shows or hides syntax highlighting. | planned |
| `WordWrap` | View | Word Wrap |  | Enables word wrapping in the main file view. | planned |
| `OutputWindow` | View | Detached Output Pane |  | Shows output pane in separate window. | planned |
| `ZoomLeft` | View |  | Ctrl+Alt+Right | Makes left pane incrementally larger. | planned |
| `ZoomRight` | View |  | Ctrl+Alt+Left | Makes right pane incrementally larger. | planned |
| `WebBrowser` | View | Webpages |  | Shows or hides webpage panels. | planned |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. | implemented |
| `ShowCenter` | View | Center Pane |  | Shows or hides center pane. | planned |
| `ShowTextDetails` | View | Text Details |  | Shows or hides text details panel. | planned |
| `ShowHexDetails` | View | Hex Details |  | Shows or hides hexadecimal details panel. | planned |
| `ShowAlignDetails` | View | Alignment Details |  | Shows or hides alignment details panel. | planned |
| `ShowRuler` | View | Ruler |  | Shows or hides ruler in details panel. | planned |
| `Overwriting` | View |  | Ins | Typing replaces characters instead of inserting them. | planned |

## Version Compare

| Command | Menu | Caption | Shortcut | Description | Status |
|---|---|---|---|---|---|
| `IsSig` | View |  |  | Marks selected item as important or unimportant. | planned |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. | planned |
