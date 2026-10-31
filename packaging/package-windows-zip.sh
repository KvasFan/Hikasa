#!/usr/bin/env bash
# hikasa-<version>-windows-<x64|x86>-portable.zip: the two executables, the portable marker, licences and a README.
#
#   package-windows-zip.sh [x64] [x86]     default: both
source "$(dirname "$0")/common.sh"
require zip
arches=("$@")
((${#arches[@]})) || arches=(x64 x86)
"$repo/packaging/build-windows.sh" "${arches[@]}"

for arch in "${arches[@]}"; do
    bin=$(windows_bin "$arch")
    name=hikasa-$version-windows-$arch
    stage=$work/stage/zip-$arch
    rm -rf "$stage"
    mkdir -p "$stage/$name"
    install -m644 "$bin/hikasa.exe" "$bin/hikasa-gui.exe" "$stage/$name/"
    install -m644 "$repo/packaging/README-portable.txt" "$stage/$name/README.txt"
    : >"$stage/$name/hikasa-portable.toml"
    copy_licenses "$stage/$name" x86_64-pc-windows-gnu i686-pc-windows-gnu
    find "$stage/$name" -exec touch --date="@$(git -C "$repo" log -1 --format=%ct)" {} +
    rm -f "$dist/$name-portable.zip"
    (cd "$stage" && zip -X -9 -r -q "$dist/$name-portable.zip" "$name")
    echo "$dist/$name-portable.zip"
done
write_sums
