# hikasa script

`hikasa script run <file> [<arg>…]` runs a script headless: a batch of commands over one invisible folder comparison.
`hikasa script check <file> [<arg>…]` checks it without running anything. A script is a text file (UTF-8) with one
command per line; `-` reads it from standard input.

```
hikasa script run nightly.txt /data/project /backups --confirm yes-to-all
hikasa script check nightly.txt
```

## The flags

| Flag | Meaning |
|---|---|
| `<arg>…` | the arguments `%1`…`%9`; one that starts with `-` follows `--` (`script run s.txt -- -x`) |
| `--confirm <mode>` | the confirm mode before line 1: `prompt` (default), `yes-to-all`, `no-to-all` |
| `--stop-on-error` | as `option stop-on-error` on line 1 |
| `--silent` | ask nothing: a question declines its command, stop on error ends the run |
| `--dry-run` | run every command as it would, but execute no plan and write no file |
| `--json` | the document `hikasa.script.v1` ([JSON output](topic:json-output)) |
| `--gui` | run it in the window with its [status window](topic:dialog-script), as `hikasa gui script`; with `--wait`, `--no-launch`, `--closescript` ([hikasa gui](topic:gui)); not with `--silent` |

`script check` runs no command and reads no base; with arguments it also checks `%n` against them. It prints
`ok: <n> commands`, or one line per error.

## The language

- One command per line; blank lines and lines that only hold a comment are nothing. LF, CRLF and CR end lines.
- Words are separated by spaces and tabs. `"` quotes any part of a word: `output-to:"%3"` is one word, `a" b"c` is
  `a bc`; inside quotes `""` is one `"`. A quote closes on its line. A backslash is always literal (`"C:\dir\"`).
- `#` where a word could start begins a comment to the end of the line; inside a word (`a#b`) it is literal.
- A trailing `&` (outside quotes) continues the command on the next line; comment lines between are skipped.
- Command names and keywords are case-insensitive; paths, masks and names are as written.
- A positional word that starts like a keyword (`title:x`) is written `./title:x`.

## Expansion

Each word is expanded just before its command runs, after the line was split into words: a value never splits or
joins words.

| Form | Value |
|---|---|
| `%1` … `%9` | the arguments after the script name (`%10` is `%1` then `0`) |
| `%0` | the script as located |
| `%%` | one `%` |
| `%date%`, `%time%`, `%fn_time%` | the local date `YYYY-MM-DD`, time `HH:MM:SS`, time `HH-MM-SS` (one instant per command) |
| `%NAME%` | the environment variable `NAME` (case-sensitive, except on Windows) |
| any other `%` | itself (`50%`) |

A `%n` above the number of arguments given, or a variable that is not set, is an error before anything runs. `lt` and
`rt` mean `left` and `right` where a side is expected: a side word, both ends of a direction, the side of a select mask,
`timezone:` of `criteria`. Relative paths are relative to the current folder, not to the script's.

## The commands

The script holds one folder comparison: settings (start: the folder-compare defaults), the comparison of the last
`load`, the open folders (none, or what the rules open), the selection of the last `select`, the confirm mode, stop on
error and the log. A command that needs a loaded comparison before any `load` is an error before anything runs.

| Command | Meaning |
|---|---|
| `load <session>`, `load <default>`, `load [create:all\|left\|right] <left> <right>` | load a Folder Compare or Folder Sync session (its display filter is not used), drop the comparison, or compare two folders with the current settings (`create:` makes missing bases) |
| `criteria <word>…` | the words of `--criteria`; a loaded comparison runs again |
| `filter <masks>`, `filter cutoff:\|size:\|attrib:\|unixtype:<expr>\|none`, `filter exclude-protected\|include-protected` | the filters of `folder compare`; a loaded comparison runs again |
| `expand all\|<path>…`, `collapse all\|<path>…` | open or close folders: `select` sees only what is open |
| `select <mask>…` | replace the selection; masks are OR-ed (`left.diff.files`, `all.orphan.all`, `empty.folders`) |
| `compare [crc\|binary\|rules-based]` | compare the contents of the selection |
| `copy left->right`, `move right->left` | as `folder op copy\|move --to` |
| `copyto [left\|right\|all] [path:relative\|base\|none] <target>`, `moveto …` | copy or move to a folder or a new archive outside the comparison |
| `delete [recyclebin=yes\|no] left\|right\|all` | delete on those sides |
| `rename <mask>`, `rename regexpr <old> <new>` | rename by a DOS mask or a regular expression |
| `touch left->right`, `touch left\|right\|all:now\|<time>` | set times from the other side, now, or an RFC 3339 time |
| `attrib (+\|-)<a s h r>…` | set or clear attributes (`+r`, `-hs`); on Unix `r` is the write bits, `a`, `s`, `h` are skipped; on a Windows disk only `r` changes, `a`, `s`, `h` are skipped with a warning (`+r+h` sets `r`) |
| `sync [visible] [create-empty] [update:(left->right\|right->left\|all) \| mirror:(left->right\|right->left)]` | as `folder sync` (`update:all` is `--update-both`; `mirror:all` is refused by the check); without a direction, the preset of the Folder Sync session the last `load` loaded, else it fails "no sync direction and the session holds none"; ignores the selection |
| `text-report`, `file-report` `layout:… options:… output-options:… title:… output-to:… <session> \| <left> <right>` | as `report text`; `output-to:-` writes standard output |
| `folder-report [just-selection] layout:… … output-to:…` | as `report folder` on the loaded comparison |
| `option stop-on-error`, `option confirm:prompt\|yes-to-all\|no-to-all` | the script's state |
| `log [none\|normal\|verbose] [[append:]<file>]` | the level of the log, and a log file |
| `beep` | BEL on a terminal (`[tweaks.scripts] beep_when_finished` also beeps once at the end of a run that is not silent) |

`snapshot`, `table-report` (`data-report`), `hex-report`, `picture-report`, `media-report`, `version-report`,
`registry-report` and `file-report` without specs pass the check and are refused when they run ("… is not available in
this version").

## Errors and exit codes

| Exit | When |
|---|---|
| 0 | every command succeeded |
| 100 | a command failed or was refused, a question was declined, the run was cancelled or stopped |
| 105 | the script cannot be found (not in the current folder nor in the shared scripts folder), is a folder, cannot be read, or is over 16 MiB |
| 106 | the check failed: nothing ran (also a word the run would refuse that holds no `%`: a base or a `copyto` target written `-`, `""` or `clipboard://`, `compare none`, a bad regular expression, `attrib +r -r`) |
| 107 | a `load` failed, whatever the cause (a base missing, not a folder or unreadable, a remote scheme, a snapshot file, an unknown session, an argument that makes a bad base, `create:` that cannot create): the script ends there, and no later line runs on an earlier comparison |

A check error is printed as `<script>:<line>:<column>: error: <message>`, every error in line order, a lexical one too (at most 100, then `… and <n> more`). Its codes
(`errors[].code` in JSON): `too_large` (105), `invalid_utf8`, `unterminated_quote`, `continuation_at_end`,
`unknown_command`, `missing_argument`, `unexpected_argument`, `duplicate_keyword`, `unknown_keyword`, `invalid_word`,
`no_comparison`, `missing_script_argument`, `unset_variable`. A script's exit code is never a comparison verdict.

With `option stop-on-error` a failed command asks "Continue the script?" on a terminal; silent, without a terminal or
on No, the script ends there.

## Questions and silent mode

Operations ask the questions of [hikasa folder op](topic:folder-op) ([Confirm an operation](topic:dialog-confirm)) in the mode of `option confirm:`:
`yes-to-all` and `no-to-all` ask nothing; `prompt` asks on a terminal (`[y]es [n]o [a]ll [s]kip all [c]ancel`). Without a
terminal the command is declined ("… questions need an answer and there is no terminal …"); with `--silent` it is
declined with "Attempted to open a dialog when running silently: …". A delete is always asked in `prompt`.

## The log

Every line is `<kind> <where> <text>` on stderr (`run L2 sync update:left->right`, `info L2 copied 12 of 12 items …`,
`error L5 …`, `info - Script completed in 2.4 s (exit 0)`); a log file gets the same lines with the date and time in
front. Levels: `none` (errors only), `normal` (default), `verbose` (also one `detail` line per item, at most 10,000 per
operation). `--quiet` keeps the error lines. A dry run writes no log file (warning `log_not_written`).

## Where a script is found

A relative name absent from the current folder is looked up below `[tweaks.scripts] shared_scripts_folder`
([hikasa settings](topic:settings); relative to the configuration folder). Only an absent script falls through.

## The examples

```
load "Sync Laptop"
sync update:left->right
```

Updates the right side of the Folder Sync session `Sync Laptop` (exit 0; with `--silent` the sync's question declines it:
exit 100).

```
log normal "/var/log/hikasa-sync.txt"
load <default>
filter "*.html;*.htm;*.png;*.jpg"
load "/srv/www" sftp://deploy@host/var/www
sync create-empty mirror:left->right
```

Ends with exit 107 at line 4: "sftp://deploy@host/var/www: remote folders are not available in this version".

```
text-report layout:side-by-side options:ignore-unimportant,display-context &
  output-to:"%3" output-options:html-color "%1" "%2"
```

`hikasa script run ex3.txt l.c r.c out.html` writes the HTML report `report text` writes for the same words (exit 0).

```
option confirm:yes-to-all
criteria crc
load "/data/project" "/backups/project.hksnap"
expand all
select left.diff.files left.orphan.files
copyto left path:relative "/backups/changes-%date%.zip"
snapshot save-crc left output:"/backups/project.hksnap"
```

Ends with exit 107 at line 3 ("…/project.hksnap: snapshots are not available in this version"). With a folder as the
right side it copies the differing and left-only files into the zip and ends with exit 100 at line 7 (`snapshot` is not
available in this version).
