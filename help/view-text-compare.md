# The Text Compare view

The Text Compare view shows two text files side by side, aligned line by line. Changed lines are highlighted,
and inside them the changed characters; unimportant differences use a lighter colour. The thumbnail on the
left shows where the differences are.

## Navigating

- Ctrl+N and Ctrl+P go to the next and the previous difference section.
- F5 reloads the files and compares them again.
- Ctrl+Num + and Ctrl+Num - change the font size; Ctrl+Num 0 restores it.

## Display filters

- View ▸ Show All, Show Differences, Show Same, Show Context and Show None choose which lines are listed; the
  toggles on the toolbar combine same, orphan and different lines.
- Ignore Unimportant Differences treats unimportant differences as same; Thumbnail shows or hides the
  thumbnail.

The same comparison is available as [hikasa text compare](topic:text-compare); the file format decides what is
unimportant ([hikasa format](topic:format)).
