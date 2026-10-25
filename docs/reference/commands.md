# Command Catalogue (default shortcuts)

Generated reference. Columns: command id, category (menu), caption, default shortcut, description.
Captions ending with "…" open a dialog. Commands without a caption are toolbar/gutter-only or dynamic (caption computed at runtime).
Shortcuts use Ctrl on Linux/Windows; on macOS Ctrl maps to Cmd unless noted.

## Global commands (all views)

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `SessionNewList` | Session | New Session |  | Opens empty session of picked type in new view. |
| `TabNew` | Session | New Tab | Ctrl+T | Opens Home view in new tab. |
| `WindowNew` | Session | New Window |  | Opens Home view in new window. |
| `SessionOpen` | Session | Open Session | Shift+Ctrl+O | Opens Home in new view. |
| `WorkspaceLoadList` | Session | Load Workspace |  | Restores picked saved view configuration. |
| `WorkspaceManager` | Session | Manage Workspaces... |  | Manages the saved view configurations that can be restored. |
| `WorkspaceSaveAs` | Session | Save Workspace As... |  | Stores current view configuration. |
| `CloseTab` | Session | Close Tab | Ctrl+W | Closes active tab. |
| `Exit` | Session | Exit | Ctrl+Q | Quits the application, prompts to save edits. |
| `Undo` | Edit |  | Ctrl+Z | Undoes previous action. |
| `Redo` | Edit |  | Ctrl+Y | Redoes previously undone action. |
| `Cut` | Edit | Cut | Ctrl+X | Cuts selection and puts it on Clipboard. |
| `Copy` | Edit | Copy | Ctrl+C | Copies selection and puts it on Clipboard. |
| `Paste` | Edit | Paste | Ctrl+V | Inserts Clipboard contents. |
| `SelectAll` | Edit | Select All | Ctrl+A | Selects all visible items. |
| `ProgramOptions` | Tools | Options... |  | Sets global preferences. |
| `FileFormatManager` | Tools | File Formats... |  | Manages file formats. |
| `ProfileManager` | Tools | Profiles... |  | Manages profiles. |
| `Export` | Tools | Export Settings... |  | Saves user settings as a file. |
| `Import` | Tools | Import Settings... |  | Loads user settings from a file. |
| `Restore` | Tools | Restore Factory Defaults... |  | Loads factory default settings. |
| `SaveSnapshot` | Tools | Save Snapshot... |  | Saves details of specified folder for future comparison. |
| `EditTextFile` | Tools | Edit Text File |  | Opens new Text Edit view. |
| `ViewPatch` | Tools | View Patch... |  | Opens picked patch file in new Text Patch view. |
| `TabNext` | MacWindow |  | Ctrl+Tab | Selects tab after active one. |
| `TabPrev` | MacWindow |  | Shift+Ctrl+Tab | Selects tab before active one. |
| `HelpContents` | Help | Contents |  | Shows help file. |
| `ContextHelp` | Help | Context Sensitive Help | F1 | Shows context sensitive help for the active window. |
| `About` | Help | About |  | Displays program information, version number and copyright. |

## Folder Compare / Folder Merge

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `NameFilterEdit` | Session |  |  | Focuses on edit listing current name file filters. |
| `NameFilter` | Session |  |  | Edits name file filters. |
| `SpecsBack` | Session | Back | Alt+Left | Changes to previous base folders. |
| `SpecsForward` | Session | Forward | Alt+Right | Changes to next base folders. |
| `UpOneLevelList` | Session | Up One Level |  | Changes base folders on picked sides to their parent folders. |
| `UpOneLevelLeft` | Session | Left Side |  | Changes left base folder to its parent folder. |
| `UpOneLevelCenter` | Session | Center Side |  | Changes center base folder to its parent folder. |
| `UpOneLevelRight` | Session | Right Side |  | Changes right base folder to its parent folder. |
| `UpOneLevelOutput` | Session | Output Side |  | Changes output base folder to its parent folder. |
| `MergeBaseView` | Session | Merge Base Folders |  | Merges base folders in new view. |
| `SyncBaseView` | Session | Sync Base Folders |  | Synchronizes base folders in new view. |
| `CompareToOutputList` | Session | Compare to Output |  | Compares picked side and output in new view. |
| `CompareLeftToOutput` | Session | Left Side |  | Compares left and output in new view. |
| `CompareCenterToOutput` | Session | Center Side |  | Compares center and output in new view. |
| `CompareRightToOutput` | Session | Right Side |  | Compares right and output in new view. |
| `QuickCompare` | Actions | Quick Compare |  | Quick compare selected files. |
| `CompareTo` | Actions | Compare To... | F7 | Marks item for comparing in new view. |
| `AlignWith` | Actions |  | F6 | Marks item for manual realignment. |
| `Compare` | Actions | Compare Contents |  | Determines overall comparison state for selected items. |
| `Merge` | Actions | Merge... |  | Performs automated merge of selected items. |
| `CopyRight` | Actions | Copy to Right | Ctrl+R | Copies selected items from left to right. |
| `CopyLeft` | Actions | Copy to Left | Ctrl+L | Copies selected items from right to left. |
| `CopyPrompt` | Actions | Copy to Side... |  | Prompts which side to copy selected items to. |
| `CopySide` | Actions |  |  | Copies selected items to other side. |
| `CopyToOutput` | Actions | Copy to Output... |  | Copies selected items to output folder. |
| `MoveRight` | Actions | Move to Right |  | Moves selected items from left to right. |
| `MoveLeft` | Actions | Move to Left |  | Moves selected items from right to left. |
| `MovePrompt` | Actions | Move to Side... |  | Prompts which side to move selected items to. |
| `MoveSide` | Actions |  |  | Moves selected items to other side. |
| `MoveBoth` | Actions | Exchange |  | Moves selected items to other side. |
| `CopyTo` | Actions | Copy to Folder... |  | Copies selected items to another folder. |
| `MoveTo` | Actions | Move to Folder... |  | Moves selected items to another folder. |
| `Delete` | Actions | Delete |  | Deletes selected items. |
| `PermanentDelete` | Actions |  | Shift+Del | Deletes selected items without moving them to the recycle bin. |
| `Rename` | Actions | Rename | F2 | Renames selected items. |
| `Attributes` | Actions | Attributes... |  | Sets the DOS attributes of selected files. |
| `Touch` | Actions | Touch... |  | Sets timestamps of selected items. |
| `NewFolder` | Actions | New Folder... | Ins | Creates a new folder. |
| `Ignored` | Actions | Ignored |  | Ignores or unignores differences in selected items. |
| `FileReport` | Actions | File Compare Report... |  | Produces file comparison report. |
| `SyncList` | Actions | Synchronize |  | Synchronizes by picked method. |
| `UpdateRight` | Actions | Update Right... |  | Synchronizes by copying newer and orphan items from left to right. |
| `UpdateLeft` | Actions | Update Left... |  | Synchronizes by copying newer and orphan items from right to left. |
| `UpdateBoth` | Actions | Update Both... |  | Synchronizes by copying newer and orphan items from left to right and from right to left. |
| `MirrorRight` | Actions | Mirror to Right... | Shift+Ctrl+R | Synchronizes by replacing all different items on right, deleting any orphan items on right, and copying any left orphan items to right. |
| `MirrorLeft` | Actions | Mirror to Left... | Shift+Ctrl+L | Synchronizes by replacing all different items on left, deleting any orphan items on left, and copying any right orphan items to left. |
| `SelectAllFiles` | Edit | Select All Files | Shift+Ctrl+A | Selects all visible files. |
| `SelectNewerList` | Edit | Select Newer |  | Selects all newer files on picked sides. |
| `SelectNewerLeft` | Edit | Left Side |  | Selects all newer files on the left. |
| `SelectNewerRight` | Edit | Right Side |  | Selects all newer files on the right. |
| `SelectNewerBoth` | Edit | Both Sides |  | Selects all newer files on both sides. |
| `SelectOrphanList` | Edit | Select Orphans |  | Selects all orphan files on picked sides. |
| `SelectOrphanLeft` | Edit | Left Side |  | Selects all orphan files on the left. |
| `SelectOrphanRight` | Edit | Right Side |  | Selects all orphan files on the right. |
| `SelectOrphanBoth` | Edit | Both Sides |  | Selects all orphan files on both sides. |
| `DiffNext` | Search | Next Difference | Ctrl+N | Positions at next difference. |
| `DiffPrev` | Search | Previous Difference | Ctrl+P | Positions at previous difference. |
| `FavCompareAll` | Favorites | Show All |  | Shows same, left orphan, right orphan, left newer, right newer and difference items. |
| `FavDiff` | DropDown | Show Differences |  | Shows left orphan, right orphan, left newer, right newer and difference items. |
| `FavSame` | Favorites | Show Same |  | Shows same items. |
| `FavNoOrphan` | DropDown | Show No Orphans |  | Shows same, left newer, right newer and difference items. |
| `FavDiffNoOrphan` | DropDown | Show Differences but No Orphans |  | Shows left newer, right newer and difference items. |
| `FavOrphan` | DropDown | Show Orphans |  | Shows left orphan and right orphan items. |
| `FavLeftNewer` | DropDown | Show Left Newer |  | Shows left newer items. |
| `FavRightNewer` | DropDown | Show Right Newer |  | Shows right newer items. |
| `FavLeftNewerOrphan` | DropDown | Show Left Newer and Left Orphans |  | Shows left orphan and left newer items. |
| `FavRightNewerOrphan` | DropDown | Show Right Newer and Right Orphans |  | Shows right orphan and right newer items. |
| `FavLeftOrphan` | DropDown | Show Left Orphans |  | Shows left orphan items. |
| `FavRightOrphan` | DropDown | Show Right Orphans |  | Shows right orphan items. |
| `FavCompareNone` | View | Show None |  | Hides all files. |
| `FavMergeAll` | Favorites | Show All |  | Shows unchanged, same change, left add, right add, left delete, right delete, left change, right change, difference change and conflict items. |
| `FavChanged` | DropDown | Show Changes |  | Shows left add, right add, left delete, right delete, left change, right change and conflict items. |
| `FavConflict` | DropDown | Show Conflicts |  | Shows conflict items. |
| `FavLeft` | DropDown | Show Left Changes |  | Shows left add, left delete and left change items. |
| `FavRight` | DropDown | Show Right Changes |  | Shows right add, right delete and right change items. |
| `FavMergeable` | DropDown | Show Mergeable |  | Shows same change and difference change items. |
| `FavUnchanged` | Favorites | Show Unchanged |  | Shows unchanged items. |
| `FavMergeNone` | View | Show None |  | Shows no items. |
| `TogSame` | Toggles |  |  | Shows or hides same items. |
| `TogLeftOrphan` | Toggles |  |  | Shows or hides left orphan items. |
| `TogRightOrphan` | Toggles |  |  | Shows or hides right orphan items. |
| `TogLeftNewer` | Toggles |  |  | Shows or hides left newer items. |
| `TogRightNewer` | Toggles |  |  | Shows or hides right newer items. |
| `TogDiff` | Toggles |  |  | Shows or hides difference items. |
| `TogNoMod` | Toggles |  |  | Shows or hides unchanged items. |
| `TogSameMod` | Toggles |  |  | Shows or hides same change items. |
| `TogLeftAdd` | Toggles |  |  | Shows or hides left add items. |
| `TogRightAdd` | Toggles |  |  | Shows or hides right add items. |
| `TogLeftDel` | Toggles |  |  | Shows or hides left delete items. |
| `TogRightDel` | Toggles |  |  | Shows or hides right delete items. |
| `TogLeftMod` | Toggles |  |  | Shows or hides left change items. |
| `TogRightMod` | Toggles |  |  | Shows or hides right change items. |
| `TogDiffMod` | Toggles |  |  | Shows or hides mergeable items. |
| `TogConflict` | Toggles |  |  | Shows or hides conflict items. |
| `DirShowAll` | View | Always Show Folders |  | Shows all folders, ignoring the display filter. |
| `DirCompareStructure` | View | Compare Files and Folder Structure |  | Shows folders if they, or items they contain, match the display filter. |
| `DirCompareFilesOnly` | View | Only Compare Files |  | Shows folders only if they contain files matching the display filter. |
| `DirFlatten` | View | Ignore Folder Structure |  | Compares all files as if they were in a single folder. |
| `ShowCenter` | View | Center Pane |  | Shows or hides center pane. |

## Folder views — shared

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `Swap` |  |  |  | Exchanges left, right base folders. |
| `BrowseList` | Session | Browse for Folder |  | Browses for an existing folder on picked side. |
| `BrowseLeft` | Session | Left Side |  | Browses for an existing folder on left side. |
| `BrowseCenter` | Session | Center Side |  | Browses for an existing folder on center side. |
| `BrowseRight` | Session | Right Side |  | Browses for an existing folder on right side. |
| `BrowseOutput` | Session | Output Side |  | Browses for an existing folder on output side. |
| `Stop` | Session |  |  | Terminates comparison. |
| `Expand` | Actions | Open Folder | Right | Opens selected folder. |
| `Collapse` | Actions | Close Folder | Left | Closes selected folder. |
| `ExpandSubfolders` | Actions | Open Subfolders |  | Opens selected folders and all their subfolders. |
| `CollapseSubfolders` | Actions | Close Subfolders |  | Closes selected folders and their subfolders. |
| `SetAsBaseFolders` | Actions |  |  | Sets selected folders as base folders. |
| `SetAsOtherBaseFolder` | Actions | Set as Base on Other Side |  | Sets selected folder as base folder on other side. |
| `Open` | Actions |  | Enter | Opens selected items in new view. |
| `OpenWithList` | Actions | Open With |  | Opens selected items in new view of picked type. |
| `AssocApp` | Actions | Associated Application |  | Opens selected items with associated application. |
| `Exclude` | Actions | Exclude |  | Filters selected items to remove them from comparison. |
| `CopyFilenames` | Actions | Copy Filename |  | Copies selected items' paths to Clipboard. |
| `RefreshSelection` | Actions | Refresh Selection | Shift+F5 | Refreshes selected items. |
| `ExplorerList` | Actions | File Manager |  | Shows file manager context menu for selected items. |
| `ExpandAll` | Edit | Expand All |  | Opens all folders. |
| `CollapseAll` | Edit | Collapse All |  | Closes all folders. |
| `SelectInvert` | Edit | Invert Selection |  | Selects all items not currently selected, and deselects all items currently selected. |
| `Refresh` | Edit | Refresh | F5 | Refreshes open folders. |
| `FullRefresh` | Edit | Full Refresh | Ctrl+F5 | Refreshes entire comparison. |
| `ExistsNext` | Search |  | Alt+Down | Positions at next filename, skipping over gaps. |
| `ExistsPrev` | Search |  | Alt+Up | Positions at previous filename, skipping over gaps. |
| `FindFilename` | Search | Find Filename... | Ctrl+F | Finds filename matching search string. |
| `FindFilenameNext` | Search | Find Next Filename | F3 | Finds next filename matching search string. |
| `FindFilenamePrev` | Search | Find Previous Filename | Shift+F3 | Finds previous filename matching search string. |
| `SuppressFilters` | View | Suppress Filters |  | Temporarily suppresses display and file filters. |
| `ColList` | View | Columns |  | Shows or hides picked column. |
| `Legend` | View | Legend | Ctrl+Alt+L | Shows or hides legend window. |
| `ShowLog` | View | Log |  | Shows or hides log panel. |

Enter, Right and Left (keys without Ctrl or Alt that focused controls use themselves) act on the folder tree
only while the tree has the keyboard focus; on a focused button, list or check box they keep their usual
meaning (M1 wave 6). In M1, `Open` opens the selected file pair in a Text Compare view (FC-062).

## Folder Sync

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `Run` | Session |  |  | Performs all sync operations. |
| `CompareBaseView` | Session | Compare Base Folders |  | Compares base folders in new view. |
| `SyncLeave` | Actions | Leave Alone |  | Changes sync operation to leave alone. |
| `SyncCopyLeft` | Actions | Copy Right to Left |  | Changes sync operation to copy right to left. |
| `SyncCopyRight` | Actions | Copy Left to Right |  | Changes sync operation to copy left to right. |
| `SyncDeleteLeft` | Actions | Delete Left |  | Changes sync operation to delete left. |
| `SyncDeleteRight` | Actions | Delete Right |  | Changes sync operation to delete right. |
| `ConflictNext` | Search | Next Conflict | Ctrl+N | Positions at next conflict. |
| `ConflictPrev` | Search | Previous Conflict | Ctrl+P | Positions at previous conflict. |

## File views — shared

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `FileFormat` | Session |  |  | Edits current File Format. |
| `Reload` | Session | Reload Files | F5 | Reloads displayed files. |
| `Recompare` | Session | Recompare Files | Ctrl+F5 | Recompares displayed files without reloading them. |
| `CompareUsingViewList` | Session | Compare Files Using |  | Compares files in picked new view. |
| `MergeView` | Session | Merge Files |  | Merges files in new view. |
| `CompareToOutputList` | Session | Compare to Output |  | Compares picked side and output in new view. |
| `CompareLeftToOutput` | Session | Left Side |  | Compares left and output in new view. |
| `CompareCenterToOutput` | Session | Center Side |  | Compares center and output in new view. |
| `CompareRightToOutput` | Session | Right Side |  | Compares right and output in new view. |
| `Open` | File | Open File... | Ctrl+O | Opens existing file into picked pane. |
| `OpenLocal` | File | Open File on File System... |  | Opens existing local file into picked pane. |
| `OpenProfile` | File | Open File Using Profile... |  | Opens existing file into picked pane using profile. |
| `OpenClipboard` | File | Open Clipboard | Shift+Ctrl+V | Opens Clipboard contents into picked pane. |
| `OpenWithList` | File | Open With |  | Opens picked file with picked application. |
| `AssocApp` | File | Associated Application |  | Opens picked file with associated application. |
| `Save` | File | Save File | Ctrl+S | Saves picked file which has been modified. |
| `SaveAs` | File | Save File As... | F12 | Saves picked file with new name. |
| `SaveAsLocal` | File | Save File on File System As... |  | Saves picked file with new name on file system. |
| `SaveAsProfile` | File | Save File Using Profile As... |  | Saves picked file with new name using profile. |
| `ExplorerList` | File | File Manager |  | Shows file manager context menu for picked file. |
| `DiffItemNext` | Search | Next Difference Item | Ctrl+N | Positions at next difference item. |
| `DiffItemPrev` | Search | Previous Difference Item | Ctrl+P | Positions at previous difference item. |
| `DiffFileNext` | Search | Next Difference Files | Ctrl+M | Opens folder compare's next difference files. |
| `DiffFilePrev` | Search | Previous Difference Files | Shift+Ctrl+M | Opens folder compare's previous difference files. |
| `CopyFileRight` | Search | Copy File to Right and Open Next Difference | Ctrl+Alt+R | Copies left file to right and opens folder compare's next difference. |
| `CopyFileLeft` | Search | Copy File to Left and Open Next Difference | Ctrl+Alt+L | Copies right file to left and opens folder compare's next difference. |
| `TakeFileLeft` | Search | Take Left File and Open Next Difference | Ctrl+Alt+L | Takes left file and opens folder merge's next difference. |
| `TakeFileRight` | Search | Take Right File and Open Next Difference | Ctrl+Alt+R | Takes right file and opens folder merge's next difference. |
| `ExistsNext` | Search |  | Alt+Down | Positions at next item, skipping over gaps. |
| `ExistsPrev` | Search |  | Alt+Up | Positions at previous item, skipping over gaps. |
| `Find` | Search | Find... | Ctrl+F | Finds search string. |
| `FindNext` | Search | Find Next | F3 | Finds next occurrence of search string. |
| `FindPrev` | Search | Find Previous | Shift+F3 | Finds previous occurrence of search string. |
| `GoLeft` | Search |  | Alt+Left | Positions in pane left of current one. |
| `GoRight` | Search |  | Alt+Right | Positions in pane right of current one. |
| `GoNext` | Search |  | F6 | Positions in pane after current one. |
| `GoPrev` | Search |  | Shift+F6 | Positions in pane before current one. |
| `FavCompareAll` | Favorites | Show All |  | Shows same, left orphan, right orphan and difference items. |
| `FavDiff` | Favorites | Show Differences |  | Shows left orphan, right orphan and difference items. |
| `FavSame` | Favorites | Show Same |  | Shows same items. |
| `FavCompareNone` | View | Show None |  | Shows no items. |
| `TogSame` | Toggles |  |  | Shows or hides same items. |
| `TogLeftOrphan` | Toggles |  |  | Shows or hides left orphan items. |
| `TogRightOrphan` | Toggles |  |  | Shows or hides right orphan items. |
| `TogDiff` | Toggles |  |  | Shows or hides difference items. |
| `ShowWS` | View | Visible Whitespace |  | Shows or hides visible spaces, tabs. |
| `DisplayFont` | View | Display Font... |  |  |
| `DisplayFontIncrease` | View | Increase Display Font Size | Ctrl+Num + |  |
| `DisplayFontDecrease` | View | Decrease Display Font Size | Ctrl+Num - |  |
| `DisplayFontReset` | View | Reset Display Font Size | Ctrl+Num 0 |  |
| `SideBySide` | View | Side-by-Side Layout |  | Arranges panes left and right of each other. |
| `OverUnder` | View | Over-Under Layout |  | Arranges panes above and below each other. |
| `ShowSpecInfo` | View | File Info |  | Shows or hides file information. |

## Hex Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible bytes. |
| `DiffItemNext` |  | Next Difference Bytes |  | Positions at next difference bytes. |
| `DiffItemPrev` |  | Previous Difference Bytes |  | Positions at previous difference bytes. |
| `ExistsNext` |  |  |  | Positions at next existing byte, skipping over gaps. |
| `ExistsPrev` |  |  |  | Positions at previous existing byte, skipping over gaps. |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference bytes. |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference bytes. |
| `FavSame` |  |  |  | Shows same bytes. |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected or current bytes to right pane. |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected or current bytes to left pane. |
| `CopySide` | Edit |  |  | Copies selected or current bytes to pane on other side. |
| `Delete` | Edit | Delete |  | Deletes selection. |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified byte. |
| `LittleEndianList` | View | Little Endian Values |  | Displays little endian values in picked way. |
| `BigEndianList` | View | Big Endian Values |  | Displays big endian values in picked way. |
| `Layout` | View | Layout... |  | Configures layout options. |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. |
| `Overwriting` | View |  | Ins | Typing replaces bytes instead of inserting them. |

## Home view

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `HomeCompare` | Session | Compare | Enter (in a path field) | Compares the two typed paths; the comparison replaces the Home view (M1 addition, UI-001). |
| `Find` | Search |  | Ctrl+F | Finds search string. |
| `ShowManage` | View | Show Session Management |  | Shows or hides management panel. |
| `MaximizeManage` | View | Maximize Session Management |  | Maximizes or restores management panel. |
| `ShowWeb` | View | Show Web Resources |  | Shows or hides web panel. |

## Media Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `Play2` | Session |  |  | Plays both audio tracks. |
| `Play` | File |  |  | Plays audio track. |
| `IsSig` | View |  |  | Marks selected item as important or unimportant. |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. |

## Picture Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `DiffTolerance` | View | Tolerance Mode |  | Detects pixel differences greater than configurable tolerance. |
| `DiffRange` | View | Mismatch Range Mode |  | Measures degree of pixel difference. |
| `DiffBlend` | View | Blend Mode |  | Blends pictures based on configurable percentage. |
| `AutoScale` | View | Auto Scale |  | Makes smaller picture as large as other picture. |
| `RotateRight` | View | Rotate Clockwise |  | Spins current picture to right by 90 degrees. |
| `RotateLeft` | View | Rotate Counterclockwise |  | Spins current picture to left by 90 degrees. |
| `FlipHorz` | View | Flip Horizontally |  | Swaps pixels of current picture relative to its horizontal center. |
| `FlipVert` | View | Flip Vertically |  | Swaps pixels of current picture relative to its vertical center. |
| `OffsetReset` | View | Reset Difference Offset |  | Realigns top left corner of both pictures. |
| `CompareTags` | View | Compare Metadata |  | Compares and shows metadata. |
| `BlendToggle` | View | Blend Toggle | F8 | Switches between left and right pictures. |
| `FullScreen` | View | Full Screen | F11 | Makes current picture show as large as possible. |
| `ZoomList` | View | Zoom |  | Makes picture appear to change size in picked way. |
| `ZoomIn` | View | Zoom In | Ctrl+Num + | Makes pictures appear to be larger. |
| `ZoomOut` | View | Zoom Out | Ctrl+Num - | Makes pictures appear to be smaller. |
| `ZoomFit` | View | Fit to Window |  | Makes pictures appear to fill panes. |

## Registry Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `Swap` |  |  |  | Exchanges left, right registries. |
| `Reload` |  | Reload |  | Reloads displayed registries. |
| `Recompare` |  | Recompare |  | Recompares displayed registries without reloading them. |
| `Open` |  | Open... |  | Opens registry into picked pane. |
| `OpenLocal` |  | Open Registry File on File System... |  | Opens existing local registry file into picked pane. |
| `OpenProfile` |  | Open Registry File Using Profile... |  | Opens existing registry file into picked pane using profile. |
| `UpOneLevelList` | Session | Up One Level |  | Changes base keys on picked sides to their parent keys. |
| `UpOneLevelLeft` | Session | Left Side |  | Changes left base key to its parent key. |
| `UpOneLevelRight` | Session | Right Side |  | Changes right base key to its parent key. |
| `UpOneLevelBoth` | Session | Both Sides |  | Changes both base keys to their parent keys. |
| `OpenReg` | File | Open Registry... |  | Opens live registry into picked pane. |
| `SetAsBases` | Edit |  |  | Sets selected keys as base keys. |
| `SetBothAsBases` | Edit | Set Both as Base Keys |  | Sets selected key and key on other side as base keys. |
| `SetAsOtherBase` | Edit | Set as Base Key on Other Side |  | Sets selected key as base key on other side. |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected items to right pane. |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected items to left pane. |
| `CopySide` | Edit |  |  | Copies selected items to pane on other side. |
| `Delete` | Edit | Delete |  | Deletes selected items. |
| `Rename` | Edit | Rename | F2 | Renames item. |
| `NewKey` | Edit | New Key |  | Adds a new key. |
| `NewVal` | Edit | New Value | Ins | Adds a new value. |
| `Modify` | Edit | Modify... |  | Modifies value. |
| `CopyKeyName` | Edit | Copy Key Name |  | Copies key's name to Clipboard. |
| `ExportKey` | Edit | Export... |  | Saves key as registry file. |
| `ExportAll` | Edit | Export All... |  | Saves all keys as registry file. |
| `ExpandAll` | View | Expand All |  | Opens all items. |
| `CollapseAll` | View | Collapse All |  | Closes all items. |
| `ShowTextDetails` | View | Text Details |  | Shows or hides text details panel. |
| `ShowHexDetails` | View | Hex Details |  | Shows or hides hexadecimal details panel. |

## Session commands (all session views)

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `SessionHome` | Session |  | Alt+Home | Replaces current view with Home view. |
| `SessionsCombo` | Session |  |  | Focuses on dropdown button listing sessions that can be opened in this view. |
| `SessionSave` | Session | Save Session | Shift+Ctrl+S | Saves this session. |
| `SessionSaveAs` | Session | Save Session As... |  | Saves this session with new name. |
| `SessionEdit` | Session | Session Settings... |  | Edits current session settings. |
| `SessionLocked` | Session | Locked |  | Locks or unlocks current session. |
| `SessionClear` | Session | Clear Session | Shift+Ctrl+C | Replaces current view with empty session. |
| `Rules` | Session |  |  | Edits current Rules definitions. |
| `Swap` | Session | Swap Sides |  | Exchanges left, right files. |
| `Report` | Session | <Type> Report... |  | Produces <type> report. |
| `Info` | Session | <Type> Info | Ctrl+I | Displays <type> statistics. |
| `CompareParents` | Session | Compare Parent Folders |  | Compares folders containing current items in new view. |
| `MergeParents` | Session | Merge Parent Folders |  | Merges folders containing current items in new view. |
| `InsigIgnored` | View | Ignore Unimportant Differences |  | Treats unimportant differences as same. |
| `SameModIgnored` | View | Ignore Same Changes |  | Treats same changes as same. |
| `ShowToolbar` | View | Toolbar |  | Shows or hides toolbar. |

## Table Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible rows. |
| `DiffItemNext` |  | Next Difference Row |  | Positions at next difference row. |
| `DiffItemPrev` |  | Previous Difference Row |  | Positions at previous difference row. |
| `ExistsNext` |  |  |  | Positions at next existing row, skipping over gaps. |
| `ExistsPrev` |  |  |  | Positions at previous existing row, skipping over gaps. |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference rows. |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference rows. |
| `FavSame` |  |  |  | Shows same rows. |
| `FavCompareNone` |  |  |  | Shows no rows. |
| `TogSame` |  |  |  | Shows or hides same rows. |
| `TogLeftOrphan` |  |  |  | Shows or hides left orphan rows. |
| `TogRightOrphan` |  |  |  | Shows or hides right orphan rows. |
| `TogDiff` |  |  |  | Shows or hides difference rows. |
| `AlignWith` | Edit | Align With... | F7 | Marks selected rows for manual realignment. |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected rows to right pane. |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected rows to left pane. |
| `CopySide` | Edit |  |  | Copies selected rows to pane on other side. |
| `CopyCellRight` | Edit | Copy Cells to Right | Shift+Ctrl+R | Copies selected cells to right pane. |
| `CopyCellLeft` | Edit | Copy Cells to Left | Shift+Ctrl+L | Copies selected cells to left pane. |
| `CopyCellSide` | Edit |  |  | Copies selected cells to pane on other side. |
| `Delete` | Edit | Delete |  | Deletes selection. |
| `DeleteCol` | Edit | Delete Column |  | Deletes selected columns. |
| `InsertItemBefore` | Edit | Insert Row Before | Shift+Ctrl+Enter | Inserts new rows before selected ones. |
| `InsertItemAfter` | Edit | Insert Row After | Ctrl+Enter | Inserts new rows after selected ones. |
| `InsertColBefore` | Edit | Insert Column Before |  | Inserts new columns before selected ones. |
| `InsertColAfter` | Edit | Insert Column After |  | Inserts new columns after selected ones. |
| `Modify` | Edit | Modify... | F2 | Modifies cell text. |
| `Sort` | Edit | Sort... |  | Sorts selected data. |
| `DiffNext` | Search | Next Difference Cell | Shift+Ctrl+N | Positions at next difference cell. |
| `DiffPrev` | Search | Previous Difference Cell | Shift+Ctrl+P | Positions at previous difference cell. |
| `DiffSheetNext` | Search | Next Difference Sheet |  | Positions at next difference sheet. |
| `DiffSheetPrev` | Search | Previous Difference Sheet |  | Positions at previous difference sheet. |
| `EditNext` | Search | Next Edit |  | Positions at next edited text. |
| `EditPrev` | Search | Previous Edit |  | Positions at previous edited text. |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified row and column. |
| `ColHideSame` | View | Hide Same Columns |  | Hides same columns. |
| `ColDetectSize` | View | Resize Columns to Fit |  | Makes columns wide enough for data to be fully visible. |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. |

## Text Compare / Text Merge / Text Edit / Text Patch

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `SelectAll` |  |  |  | Selects all visible lines. |
| `ExistsNext` |  |  |  | Positions at next existing line, skipping over gaps. |
| `ExistsPrev` |  |  |  | Positions at previous existing line, skipping over gaps. |
| `FavCompareAll` |  |  |  | Shows same, left orphan, right orphan and difference lines. |
| `FavDiff` |  |  |  | Shows left orphan, right orphan and difference lines. |
| `FavSame` |  |  |  | Shows same lines. |
| `FavCompareNone` |  |  |  | Shows no lines. |
| `TogSame` |  |  |  | Shows or hides same lines. |
| `TogLeftOrphan` |  |  |  | Shows or hides left orphan lines. |
| `TogRightOrphan` |  |  |  | Shows or hides right orphan lines. |
| `TogDiff` |  |  |  | Shows or hides difference lines. |
| `ApplyPatch` | File | Apply Patch |  | Applies patch file. |
| `AlignWith` | Edit | Align With... | F7 | Marks selected lines for manual realignment. |
| `Isolate` | Edit | Isolate |  | Realigns comparison so that selected lines are by themselves. |
| `Replacement` | Edit | Replacement... |  | Adds item to replacement list based on current difference. |
| `CopyRight` | Edit | Copy to Right | Ctrl+R | Copies selected lines or current section to right pane. |
| `CopyLeft` | Edit | Copy to Left | Ctrl+L | Copies selected lines or current section to left pane. |
| `CopySide` | Edit |  |  | Copies selected lines or current section to pane on other side. |
| `CopyItemRight` | Edit | Copy Line to Right | Shift+Ctrl+R | Copies current line to right pane. |
| `CopyItemLeft` | Edit | Copy Line to Left | Shift+Ctrl+L | Copies current line to left pane. |
| `CopyItemSide` | Edit |  |  | Copies current line to pane on other side. |
| `Conflict` | Edit | Conflict |  | Clears or sets conflict for selected lines or current section. |
| `TakeLeft` | Edit | Take Left | Ctrl+L | Takes selected lines or current section from left pane. |
| `TakeCenter` | Edit | Take Center |  | Takes selected lines or current section from center pane. |
| `TakeRight` | Edit | Take Right | Ctrl+R | Takes selected lines or current section from right pane. |
| `TakeLeftRight` | Edit | Take Left Then Right | Ctrl+B | Takes selected lines or current section from left then right panes. |
| `TakeRightLeft` | Edit | Take Right Then Left | Shift+Ctrl+B | Takes selected lines or current section from right then left panes. |
| `TakeItemLeft` | Edit | Take Left Line | Shift+Ctrl+L | Takes current line from left pane. |
| `TakeItemCenter` | Edit | Take Center Line |  | Takes current line from center pane. |
| `TakeItemRight` | Edit | Take Right Line | Shift+Ctrl+R | Takes current line from right pane. |
| `IncreaseIndent` | Edit | Increase Indent | Shift+Ctrl+I | Increases indentation of selected lines. |
| `DecreaseIndent` | Edit | Decrease Indent | Shift+Ctrl+U | Decreases indentation of selected lines. |
| `Delete` | Edit | Delete |  | Deletes selection. |
| `DeleteItemBegin` | Edit | Delete to Start of Line | Shift+Ctrl+BkSp | Deletes to start of current line. |
| `DeleteItemEnd` | Edit | Delete to End of Line | Shift+Ctrl+Del | Deletes to end of current line. |
| `DeleteWord` | Edit | Delete Word |  | Deletes current word. |
| `DeleteWordBegin` | Edit | Delete to Start of Word | Ctrl+BkSp | Deletes to start of current word. |
| `DeleteWordEnd` | Edit | Delete to End of Word | Ctrl+Del | Deletes to end of current word. |
| `InsertItemBefore` | Edit | Insert Line Before | Shift+Ctrl+Enter | Inserts new line before current one. |
| `InsertItemAfter` | Edit | Insert Line After | Ctrl+Enter | Inserts new line after current one. |
| `SelectSection` | Edit | Select Section | Ctrl+D | Selects all lines in current section. |
| `CompareToClipboard` | Edit | Compare Selection to Clipboard |  | Compares selected text to clipboard in new view. |
| `ConvertList` | Edit | Convert File |  | Performs picked file conversion. |
| `TrimTrailWS` | Edit | Trim Trailing Whitespace |  | Strips off spaces and tab characters at the end of lines. |
| `LeadSpaceToTab` | Edit | Leading Spaces to Tabs |  | Replaces spaces at the beginning of lines with tab characters. |
| `TabToSpace` | Edit | Tabs to Spaces |  | Replaces tab characters with spaces. |
| `EndingStyleList` | Edit | Line Endings |  | Converts line endings to picked style. |
| `ToWindows` | Edit | To Windows/DOS |  | Converts line endings to Windows/DOS style. |
| `ToUnix` | Edit | To Unix |  | Converts line endings to Unix style. |
| `ConflictSectionOffNext` | Search | Clear Conflict Section, Next |  | Clears conflict for current section and positions at next conflict section. |
| `ConflictSectionNext` | Search | Next Conflict Section | Alt+N | Positions at next conflict section. |
| `ConflictSectionPrev` | Search | Previous Conflict Section | Alt+P | Positions at previous conflict section. |
| `DiffNext` | Search | Next Difference | Shift+Ctrl+N | Positions at next difference text. |
| `DiffPrev` | Search | Previous Difference | Shift+Ctrl+P | Positions at previous difference text. |
| `DiffSectionNext` | Search | Next Difference Section | Ctrl+N | Positions at next unignored difference section. |
| `DiffSectionPrev` | Search | Previous Difference Section | Ctrl+P | Positions at previous unignored difference section. |
| `DiffReplacedNext` | Search | Next Replacement | Shift+Ctrl+Alt+N | Positions at next replacement text. |
| `DiffReplacedPrev` | Search | Previous Replacement | Shift+Ctrl+Alt+P | Positions at previous replacement text. |
| `TakenLeftNext` | Search | Next Left Taken |  | Positions at next range of lines where left was taken. |
| `TakenLeftPrev` | Search | Previous Left Taken |  | Positions at previous range of lines where left was taken. |
| `TakenRightNext` | Search | Next Right Taken |  | Positions at next range of lines where right was taken. |
| `TakenRightPrev` | Search | Previous Right Taken |  | Positions at previous range of lines where right was taken. |
| `EditNext` | Search | Next Edit |  | Positions at next edited text. |
| `EditPrev` | Search | Previous Edit |  | Positions at previous edited text. |
| `Replace` | Search | Replace... | Ctrl+H | Replaces search string. |
| `FindInFiles` | Search | Find in Files... | Shift+Ctrl+F | Finds search string in multiple files. |
| `GoTo` | Search | Go To... | Ctrl+G | Positions at specified line and column. |
| `BookmarkToggleList` | Search | Toggle Bookmark |  | Sets or clears picked bookmark. |
| `BookmarkGoToList` | Search | Go to Bookmark |  | Positions at picked bookmark. |
| `BookmarkClear` | Search | Clear Bookmarks |  | Removes all bookmarks. |
| `GoInput` | Search |  | Alt+PgUp | Positions in input pane. |
| `GoOutput` | Search |  | Alt+PgDn | Positions in output pane. |
| `FavMergeAll` | Favorites | Show All |  | Shows unchanged, same change, left change, right change, different change and conflict lines. |
| `FavChanged` | DropDown | Show Changes |  | Shows same change, left change, right change, different change and conflict lines. |
| `FavConflict` | DropDown | Show Conflicts |  | Shows conflict lines. |
| `FavLeft` | DropDown | Show Left Changes |  | Shows same change, left change, different change and conflict lines. |
| `FavRight` | DropDown | Show Right Changes |  | Shows same change, right change, different change and conflict lines. |
| `FavMergeable` | DropDown | Show Mergeable |  | Shows same change, left change, right change and different change lines. |
| `FavUnchanged` | Favorites | Show Unchanged |  | Shows unchanged lines. |
| `FavMergeNone` | View | Show None |  | Shows no lines. |
| `TogNoMod` | Toggles |  |  | Shows or hides unchanged lines. |
| `TogSameMod` | Toggles |  |  | Shows or hides same change lines. |
| `TogLeftMod` | Toggles |  |  | Shows or hides left change lines. |
| `TogRightMod` | Toggles |  |  | Shows or hides right change lines. |
| `TogDiffMod` | Toggles |  |  | Shows or hides different change lines. |
| `TogConflict` | Toggles |  |  | Shows or hides conflict lines. |
| `ShowContext` | View | Show Context |  | Shows lines surrounding difference. |
| `FavorLeft` | View | Favor Left Changes |  | Suppresses highlighting of left-only changes in output. |
| `FavorRight` | View | Favor Right Changes |  | Suppresses highlighting of right-only changes in output. |
| `Ignored` | View | Ignored |  | Ignores or unignores differences in selected lines or current section. |
| `ShowSyntaxHighlighting` | View | Syntax Highlighting |  | Shows or hides syntax highlighting. |
| `WordWrap` | View | Word Wrap |  | Enables word wrapping in the main file view. |
| `OutputWindow` | View | Detached Output Pane |  | Shows output pane in separate window. |
| `ZoomLeft` | View |  | Ctrl+Alt+Right | Makes left pane incrementally larger. |
| `ZoomRight` | View |  | Ctrl+Alt+Left | Makes right pane incrementally larger. |
| `WebBrowser` | View | Webpages |  | Shows or hides webpage panels. |
| `ShowThumbnail` | View | Thumbnail |  | Shows or hides thumbnail view. |
| `ShowCenter` | View | Center Pane |  | Shows or hides center pane. |
| `ShowTextDetails` | View | Text Details |  | Shows or hides text details panel. |
| `ShowHexDetails` | View | Hex Details |  | Shows or hides hexadecimal details panel. |
| `ShowAlignDetails` | View | Alignment Details |  | Shows or hides alignment details panel. |
| `ShowRuler` | View | Ruler |  | Shows or hides ruler in details panel. |
| `Overwriting` | View |  | Ins | Typing replaces characters instead of inserting them. |

## Version Compare

| Command | Menu | Caption | Shortcut | Description |
|---|---|---|---|---|
| `IsSig` | View |  |  | Marks selected item as important or unimportant. |
| `ShowDetails` | View | Text Details |  | Shows or hides text details panel. |

