#!/usr/bin/env bash
# hikasa-<version>-windows-<x64|x86>.msi: compiles windows/hikasa.wxs with wixl from msitools.
#
#   package-windows-msi.sh [x64] [x86]     default: both
source "$(dirname "$0")/common.sh"
for tool in wixl msiinfo msibuild; do
    command -v "$tool" >/dev/null || die "$tool is required to build the installers: install msitools (Arch: pacman -S msitools)"
done
arches=("$@")
((${#arches[@]})) || arches=(x64 x86)
"$repo/packaging/build-windows.sh" "${arches[@]}"

licenses=$(third_party_licenses x86_64-pc-windows-gnu i686-pc-windows-gnu)

for arch in "${arches[@]}"; do
    bin=$(windows_bin "$arch")
    stage=$work/stage/msi-$arch
    rm -rf "$stage"
    mkdir -p "$stage"
    install -m644 "$bin/hikasa.exe" "$bin/hikasa-gui.exe" \
        "$repo/LICENSE-MIT" "$repo/LICENSE-APACHE" "$licenses" "$repo/assets/brand/hikasa.ico" "$stage/"
    out=$dist/hikasa-$version-windows-$arch.msi
    wixl --arch "$arch" --define "Version=${version%%[-+]*}" --define "Arch=$arch" --define "Stage=$stage" \
        --output "$out" "$repo/packaging/windows/hikasa.wxs"
    # Two things wixl 0.106 writes wrongly. Environment: it drops the `-` of Permanent="no", without which an uninstall
    # leaves the PATH entry behind. Registry: it stores HKMU as 4, a root Windows does not know, where -1 is meant.
    msiinfo export "$out" Environment | sed -E 's/\t=(\*?)PATH\t/\t=-\1PATH\t/' >"$stage/Environment.idt"
    msiinfo export "$out" Registry | sed -E 's/^(reg[0-9A-F]+)\t4\t/\1\t-1\t/' >"$stage/Registry.idt"
    msibuild "$out" -i "$stage/Environment.idt" -i "$stage/Registry.idt"
    echo "$out"
done
write_sums
