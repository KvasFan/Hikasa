# Regex helper

The ▸ button beside a regular expression field (the [Find and Replace](topic:panel-find-replace) panel with *Regular
expression* ticked, [Find in Files](topic:dialog-find-in-files), the [Grammar Item](topic:dialog-grammar-item) dialog,
the line weights of the [format editor](topic:dialog-format-editor), the unimportant text of [Session
Settings](topic:dialog-session-settings)) inserts a piece of a regular expression. The piece replaces the field's
selection; the caret goes after it, or inside the brackets of *Any of* and *None of*.

| Item | Inserts |
|---|---|
| Tab | `\t` |
| Any character | `.` |
| Word character | `\w` |
| Not a word character | `\W` |
| Digit | `\d` |
| Not a digit | `\D` |
| White space | `\s` |
| Any of | `[ ]` |
| None of | `[^ ]` |
| Line start | `^` |
| Line end | `$` |
| Form feed | `\x0C` |
| Or | `|` |
| Zero or more | `*` |
| One or more | `+` |
| Optional | `?` |

See [Contents](topic:index).
