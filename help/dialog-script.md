# Scripts in the window

A [script](topic:script) can run in the window, with a status window that shows what it does. It runs exactly as
`hikasa script run` runs it: the same commands, the same log lines, the same exit codes.

## Starting a script

- **Tools ▸ Run Script…**: the script (**Browse…** picks it in the [Browse](topic:dialog-browse) dialog; a file name is looked up in the current folder, then in
  `[tweaks.scripts] shared_scripts_folder`), its arguments (split as a script line is: `"a b" c` is two arguments),
  *Stop on error* and the confirm mode (*Ask*, *Yes to all*, *No to all*). **Run** checks the script first: a script
  that cannot be read or does not check stays in the dialog with its errors (`<script>:<line>:<column>: error: …`), and
  nothing runs.
- `hikasa gui script <file> [<arg>…]` and `hikasa script run <file> … --gui` check the script in the terminal, then run
  it in the window ([hikasa gui](topic:gui)). The script's relative paths and `%NAME%` variables mean what they mean
  in that terminal. With `--dry-run` the window runs it as a dry run: each command shows its plan, nothing is executed
  and no file is written.
- `hikasa-gui @<file> [<arg>…]` (and `hikasa gui open @<file> …`) starts the window with the script, or sends it to the
  window that runs. With `--silent` the window is never used: the script runs where it was started, headless; Ctrl-C
  stops it there as it stops `hikasa script run --silent`.

One script runs at a time: another one is refused while it runs ("a script is running in the window; wait for it or
stop it").

## The status window

- The script's path; "Command 3 of 7, line 5: copy left->right" for the command that runs; the progress of what it
  does now (comparing, copying, writing a report: the phase and the counts).
- The log: the lines of the script's log at its level (`run`, `info`, `warning`, `error`; `detail` lines at `log
  verbose`), errors in the error colour. The last `[tweaks.log_panel] display_up_to` lines stay.
- At the end: "Script completed in 2.4 s", "Script stopped at line 4" or "Script failed (exit 100)". A notice says
  "Script <name> completed", or "Script <name> failed: exit <code>" with **Show**, which brings the status window back.
  The log panel keeps a summary of each command and every error, from the source `script <name>`.

## Stop and Close

- **Stop** (Esc, or the window's close button, while it runs) cancels the script as Ctrl-C does: the command that runs
  stops as its command line command would (a copy finishes the current file), the rest is not run, exit 100.
- **Close** (Esc) closes the status window once the script ended.
- A status window that goes while its script runs (its window closes, the program quits) stops the script the same way;
  the program waits for the command that runs to end (a copy finishes its file, no temporary file is left) before it
  exits, and before another script can start.
- `[tweaks.scripts] close_when_finished` closes it after a script that ended with exit 0; `--closescript` closes it
  after any script. `beep_when_finished` beeps at the end: the window asks for attention and the title flashes.

## Questions

- A command whose plan has questions (`option confirm:prompt`, the default) asks them with
  [Confirm an operation](topic:dialog-confirm) over the status window. Cancel declines that command only; the script goes
  on. "Do not ask again" is not offered: a script line means the same whatever was answered before.
- With *Stop on error* (`--stop-on-error`, `option stop-on-error`), a failed command asks "The command at line <n> failed:
  <message>. Continue the script?": **Yes** goes on, **No** (Esc) ends the script there.

See [hikasa script](topic:script), [Exit codes](topic:exit-codes) and [The main window](topic:main-window).
