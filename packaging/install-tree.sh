#!/usr/bin/env bash
# Installs Hikasa into a Linux file tree: <root>/usr/bin, the desktop entry, metainfo, MIME type, icons and shell
# completions. Shared by the .deb, the AppImage and the PKGBUILD; licences are each format's own business.
#
#   install-tree.sh <root> <folder with the two executables> [zsh completion folder below <root>]
set -euo pipefail

source=$(cd "$(dirname "$0")/.." && pwd)
root=$1
bin=$2
zsh=${3:-usr/share/zsh/site-functions}

install -Dm755 -t "$root/usr/bin" "$bin/hikasa" "$bin/hikasa-gui"
install -Dm644 -t "$root/usr/share/applications" "$source/packaging/hikasa.desktop"
install -Dm644 -t "$root/usr/share/metainfo" "$source/packaging/io.github.kvasfan.hikasa.metainfo.xml"
install -Dm644 "$source/packaging/hikasa-mime.xml" "$root/usr/share/mime/packages/hikasa.xml"
install -Dm644 "$source/assets/brand/logo-light.svg" "$root/usr/share/icons/hicolor/scalable/apps/hikasa.svg"
for icon in "$source"/assets/brand/hicolor/*/apps/hikasa.png; do
    size=$(basename "$(dirname "$(dirname "$icon")")")
    install -Dm644 "$icon" "$root/usr/share/icons/hicolor/$size/apps/hikasa.png"
done

mkdir -p "$root/usr/share/bash-completion/completions" "$root/usr/share/fish/vendor_completions.d" "$root/$zsh"
"$bin/hikasa" completions bash >"$root/usr/share/bash-completion/completions/hikasa"
"$bin/hikasa" completions fish >"$root/usr/share/fish/vendor_completions.d/hikasa.fish"
"$bin/hikasa" completions zsh >"$root/$zsh/_hikasa"
