# hikasa completions

`hikasa completions <shell>` prints a completion script for `bash`, `elvish`, `fish`, `powershell` or `zsh`.

```
hikasa completions bash > ~/.local/share/bash-completion/completions/hikasa
hikasa completions fish > ~/.config/fish/completions/hikasa.fish
```

It reads no configuration. With `--json` the script is the `script` field of `hikasa.completions.v1`. See
[Using the command line](topic:cli).
