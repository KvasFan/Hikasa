# Copy File and Open Next

In a Text Compare view opened from a folder comparison, Search ▸ **Copy File to Right and Open Next Difference**
(Ctrl+Alt+R) and **Copy File to Left…** (Ctrl+Alt+L) copy the shown file over the other side's and move on.

The question lists the steps as they will run: *Save the edits of* the source (when it has edits), *Discard the edits
of* the target (when it has some), *Copy … to the right*, then *Open the next pair*. **Yes** runs them in order; a
step that fails or is declined stops the rest with its message. **No** does nothing.

The copy is the folder view's own, so it asks its questions (a read-only or newer target) as it would there; toward a
read-only side the command is greyed. The parent folder view, when it is open, shows the copied row refreshed.
Search ▸ **Next/Previous Difference Files** (Ctrl+M, Shift+Ctrl+M) open the next pair without copying. See [The Text
Compare view](topic:view-text-compare).
