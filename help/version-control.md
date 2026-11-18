# Using Hikasa with version control

A version control system shows the changes of a file with an external diff tool: it writes the two versions to
files, starts the tool with their paths and waits for it to end. `hikasa gui open <left> <right> --wait` is that
tool: it opens the pair in the window (starting it when needed) and returns only when you close the view
([hikasa gui](topic:gui)). The view is chosen by the files' formats, so a table, a picture or a binary file opens in
its own view; two folders open a Folder Compare view.

`hikasa quick` and the headless compare subcommands (`hikasa text compare`, `hikasa folder compare`, …) return at
once and never need the window: use them where a script only needs the verdict or a report.

## Exit codes of a waiting view

`--wait` exits with the code of the view when it closes ([Exit codes](topic:exit-codes)):

| Code | Meaning |
|---|---|
| 0 | the view has no verdict (a Text Edit view, a comparison stopped before its verdict) |
| 1, 2 | identical bytes; the same under the rules |
| 11, 12, 13 | binary differences; only unimportant differences; important differences |
| 100 | the view could not load its files (folder views: also closed while still scanning) |
| 102 | the command could not wait for the window (the connection was lost, or `--no-launch` found no window) |
| 103 | no window executable next to `hikasa` or on `PATH` |

Most version control systems read any code other than 0 (or 0 and 1) as a failure of the tool. Hikasa's codes tell
differences apart from errors, so either tell the system not to trust the code, as below, or end a wrapper script
with `exit 0`.

## git

```text
git config --global diff.tool hikasa
git config --global difftool.hikasa.cmd 'hikasa gui open "$LOCAL" "$REMOTE" --wait'
git config --global difftool.prompt false
git config --global difftool.trustExitCode false
```

`git difftool` then shows each changed file in turn: the next one opens when you close the view. `git difftool -d`
(`--dir-diff`) gives Hikasa two folders, which open in one Folder Compare view. On Windows the command is the same
with `hikasa.exe`.

## Mercurial

Enable the `extdiff` extension and name the command in `hgrc`:

```text
[extensions]
extdiff =

[extdiff]
cmd.hikasa = hikasa
opts.hikasa = gui open --wait
```

`hg hikasa` compares the working folder with its parent; for a single file it opens the file's view, for several
files a Folder Compare view of two temporary folders.

## Subversion

`svn diff --diff-cmd <program>` starts the program with `-u`, two labels (`-L <label>`) and then the two files, and
treats a code above 1 as an error. A short script maps that onto Hikasa:

```sh
#!/bin/sh
# hikasa-svn-diff: the two labels become the panes' titles; the last two arguments are the files.
title1= title2=
while [ $# -gt 2 ]; do
    case $1 in
        -L) if [ -z "$title1" ]; then title1=$2; else title2=$2; fi; shift ;;
    esac
    shift
done
hikasa gui open "$1" "$2" --title1 "${title1:-$1}" --title2 "${title2:-$2}" --wait
exit 0
```

Use it with `svn diff --diff-cmd hikasa-svn-diff`, or for every diff with `diff-cmd = hikasa-svn-diff` in the
`[helpers]` section of `~/.subversion/config`.

## Perforce

Perforce starts the program of `P4DIFF` with the two files as its last arguments:

```text
P4DIFF="hikasa gui open --wait"
```

`p4 diff` then opens each changed file in the window and goes on when you close it.

## Merging

Three-way merging (Text Merge and the merge tool setting of version control systems) is a later milestone; until
then Hikasa is configured as a diff tool only. To review the result of a merge, compare the merged file with either
side.

## See also

- [hikasa gui](topic:gui): every switch of `gui open`, the titles (`--title1`, `--title2`), read-only panes.
- [Exit codes](topic:exit-codes) and [Using the command line](topic:cli).
