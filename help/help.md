# hikasa help

`hikasa help [topic…]` prints a help topic; without a topic it prints the [contents](topic:index).

- The words name the topic: `hikasa help text compare` shows the topic `text-compare`. A subcommand without
  its own topic shows its parent's (`hikasa help format list` shows `format`).
- `hikasa help --list` lists every topic: id and title.
- An unknown topic exits 100 and suggests similar ones.
- With `--json`: `hikasa.help.v1` with the topic's `id`, `title`, `markdown` source and `links`; with `--list`
  the `topics` array.

`hikasa <subcommand> --help` prints the flags of a subcommand; the topics explain them. In the window, F1 opens
the topic of the active view ([The main window](topic:main-window)).
