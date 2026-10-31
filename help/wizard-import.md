# Import Settings

Tools ▸ Import Settings… reads a `.hikasa-pkg` file. The wizard also opens for `hikasa-gui file.hikasa-pkg` and for
`hikasa gui open file.hikasa-pkg`, with the path filled in. A package that arrives while the wizard is open replaces
the path on the first step; later the wizard asks whether to switch, and while it is working a notice says the package
was not opened. Every step shows the package in use. Enter a full path: a relative one is refused, and nothing is
expanded (`~`).

1. **Package.** The whole package is checked before anything else; a package that does not validate stops here with
   the reason, and nothing is written.
2. **Choose.** The categories and items of the package. An item is marked Add, Replace (it collides with an existing
   one, shown in red) or Kept (a locked session), under All too. "Include locked sessions" replaces those too. "Delete
   existing" removes all existing sessions, file formats or workspaces first; locked sessions stay unless included.
3. **Summary.** What will happen, per category, for exactly these choices: "2 sessions will be imported, replacing 1
   existing." Locked sessions kept are counted here too.

Finish plans once more and runs. The result lists each item; if the files changed since the summary, it says that
what ran differs. The settings apply at once and open views keep running; the window and Exit wait while it runs.
`hikasa settings import` does the same
from the command line, and `--silent` imports everything without a window. See
[Export Settings](topic:wizard-export), [hikasa settings](topic:settings) and [Contents](topic:index).
