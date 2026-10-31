#!/usr/bin/env bash
# hikasa_<version>-1_amd64.deb, written without dpkg-deb: a .deb is an ar archive of debian-binary, control.tar.xz and
# data.tar.xz, in that order, and the members below have the header dpkg-deb gives them.
#
# The libraries after libc6 are loaded at run time (winit, wgpu, the system font lookup), which is why they do not show
# up in `objdump -p`. No maintainer scripts: Debian's triggers refresh the icon, desktop and MIME caches.
source "$(dirname "$0")/common.sh"
"$repo/packaging/build-linux.sh"
require ar xz

revision=1
arch=amd64
name=hikasa_$version-${revision}_$arch
stage=$work/stage/deb
root=$stage/root
control=$stage/control
epoch=$(git -C "$repo" log -1 --format=%ct)
rm -rf "$stage"
mkdir -p "$root" "$control"

"$repo/packaging/install-tree.sh" "$root" "$linux_bin" usr/share/zsh/vendor-completions
stage_metainfo "$root/usr/share/metainfo/io.github.kvasfan.hikasa.metainfo.xml"
doc=$root/usr/share/doc/hikasa
mkdir -p "$doc"
copy_licenses "$doc" "$linux_target"
# Debian keeps the licences in one file, usr/share/doc/<package>/copyright.
{
    printf 'Format: https://www.debian.org/doc/packaging-manuals/copyright-format/1.0/\n'
    printf 'Upstream-Name: Hikasa\nSource: https://github.com/KvasFan/Hikasa\n\n'
    printf 'Files: *\nCopyright: Hikasa contributors\nLicense: MIT or Apache-2.0\n\n'
    printf 'Files: embedded crates and fonts\nCopyright: see THIRD_PARTY_LICENSES.txt\nLicense: see THIRD_PARTY_LICENSES.txt\n'
} >"$doc/copyright"

floor=$(objdump -T "$linux_bin/hikasa" "$linux_bin/hikasa-gui" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)
floor=${floor#GLIBC_}
installed=$(find "$root" -type f -printf '%s\n' | awk '{kib += int(($1 + 1023) / 1024)} END {print kib}')
cat >"$control/control" <<CONTROL
Package: hikasa
Version: $version-$revision
Architecture: $arch
Maintainer: Hikasa contributors <https://github.com/KvasFan/Hikasa/issues>
Installed-Size: $installed
Depends: libc6 (>= $floor), libegl1, libfontconfig1, libwayland-client0, libwayland-egl1, libx11-6, libx11-xcb1, libxcursor1, libxi6, libxkbcommon0, libxkbcommon-x11-0
Section: utils
Priority: optional
Homepage: https://github.com/KvasFan/Hikasa
Description: compare folders and files
 Hikasa compares folder trees and text files side by side. The desktop
 application (hikasa-gui) and the command line (hikasa) share the same
 engines, so every comparison can also run headless, with exit codes and
 JSON output.
CONTROL
(cd "$root" && find usr -type f | LC_ALL=C sort | xargs md5sum) >"$control/md5sums"

deterministic_tar=(tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$epoch")
"${deterministic_tar[@]}" -C "$control" -cf - . | xz -9e -T2 >"$stage/control.tar.xz"
"${deterministic_tar[@]}" -C "$root" -cf - . | xz -9e -T2 >"$stage/data.tar.xz"
printf '2.0\n' >"$stage/debian-binary"

member() { # <file>: an ar header (name, mtime, uid, gid, mode, size, magic), the bytes, a pad to an even offset
    local size
    size=$(stat -c %s "$1")
    printf '%-16s%-12s%-6s%-6s%-8s%-10s`\n' "$(basename "$1")" "$epoch" 0 0 100644 "$size"
    cat "$1"
    if ((size % 2)); then printf '\n'; fi
}
{
    printf '!<arch>\n'
    member "$stage/debian-binary"
    member "$stage/control.tar.xz"
    member "$stage/data.tar.xz"
} >"$dist/$name.deb"

[[ $(ar t "$dist/$name.deb" | paste -sd' ') == 'debian-binary control.tar.xz data.tar.xz' ]] || die "bad ar layout"
write_sums
echo "$dist/$name.deb"
