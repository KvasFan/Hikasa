# Save Session As

Session ▸ Save Session As… gives a view's session a name, and with it a file below the settings folder. Pick an
existing folder or type a new one, then the name; a name that exists is offered for replacing after a question.
After the save the view belongs to that session, and its auto-saved copy is deleted.

Session ▸ Save Session saves a named session. On a view without a name it opens this dialog.

## What happens when a view closes

- A view without a name is auto-saved, and appears under *Auto-saved* in the [Home view](topic:view-home). Views are
  also auto-saved every five minutes while they change. `[tweaks.metrics] max_auto_saved_sessions` limits how many
  are kept; 0 turns auto-save off.
- A named session with changed inputs or rules asks "Save changes to session?". Yes saves (if the save fails the tab
  stays open and the close stops, also at Exit; use Save Session As then), No closes without saving,
  Cancel keeps the view open and stops whatever was closing: the tab, the window, the exit or a workspace load.
- A named session without such changes stores only its layout and display filter, without asking.
- A locked session (Session ▸ Locked) is never written when it closes. Session Save on it asks "Save changes to
  locked session?"; if you answer No, the question comes back when the view closes.
- Closing from the command line (`hikasa gui close`, `hikasa gui quit`) never asks: named sessions keep their files.

See [hikasa session](topic:session) and [Contents](topic:index).
