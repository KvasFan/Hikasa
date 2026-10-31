#!/usr/bin/env bash
# hikasa-<version>-x86_64-linux.tar.gz: the two executables, the licences and a short README.
source "$(dirname "$0")/common.sh"
"$repo/packaging/build-linux.sh"

name=hikasa-$version-x86_64-linux
stage=$work/stage/targz
rm -rf "$stage"
mkdir -p "$stage/$name"
install -m755 "$linux_bin/hikasa" "$linux_bin/hikasa-gui" "$stage/$name/"
install -m644 "$repo/packaging/README-portable.txt" "$stage/$name/README.txt"
copy_licenses "$stage/$name" "$linux_target"
tar --sort=name --owner=0 --group=0 --numeric-owner --mtime="@$(git -C "$repo" log -1 --format=%ct)" -C "$stage" -cf - "$name" |
    gzip -9n >"$dist/$name.tar.gz"
write_sums
echo "$dist/$name.tar.gz"
