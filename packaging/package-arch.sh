#!/usr/bin/env bash
# hikasa-<version>-1-x86_64.pkg.tar.zst: runs makepkg on packaging/aur/PKGBUILD, never installs anything.
#
#   package-arch.sh          the recipe against the working tree (a tarball of the tracked and new files)
#   package-arch.sh --tag    the recipe as published: the tag's source from GitHub, which must exist
#
# makepkg gets a configuration of its own: the machine's flags (here RUSTFLAGS with target-cpu=native, CFLAGS with
# -march=native) must not reach a package meant for other machines.
source "$(dirname "$0")/common.sh"
require makepkg bsdtar

stage=$work/stage/arch
rm -rf "$stage"
mkdir -p "$stage/src" "$stage/build" "$stage/pkg" "$stage/log"

sed "s/^pkgver=.*/pkgver=$version/" "$repo/packaging/aur/PKGBUILD" >"$stage/src/PKGBUILD"
if [[ ${1:-} != --tag ]]; then
    tarball=$stage/src/hikasa-$version.tar.gz
    (
        cd "$repo"
        git ls-files --cached --others --exclude-standard -z | while IFS= read -r -d '' file; do
            if [[ -e $file ]]; then printf '%s\0' "$file"; fi
        done | tar --null --files-from=- --transform="s,^,Hikasa-$version/," --owner=0 --group=0 --numeric-owner -czf "$tarball"
    )
    sum=$(sha256sum "$tarball" | cut -d' ' -f1)
    sed -i -e "s|^source=.*|source=(\"hikasa-$version.tar.gz\")|" -e "s|^sha256sums=.*|sha256sums=('$sum')|" "$stage/src/PKGBUILD"
fi

cat >"$stage/makepkg.conf" <<CONF
source /etc/makepkg.conf
CFLAGS='-march=x86-64 -mtune=generic -O2 -pipe -fno-plt'
CXXFLAGS="\$CFLAGS"
RUSTFLAGS='-C target-cpu=x86-64 --remap-path-prefix=$stage/build=/build'
OPTIONS=(strip docs !libtool !staticlibs emptydirs zipman purge !debug !lto !autodeps)
BUILDDIR='$stage/build'
PKGDEST='$stage/pkg'
SRCDEST='$stage/src'
LOGDEST='$stage/log'
PKGEXT='.pkg.tar.zst'
COMPRESSZST=(zstd -c -T2 --ultra -20 -)
CONF

(
    cd "$stage/src"
    export CARGO_BUILD_JOBS=$jobs
    # A rustup toolchain provides cargo without the pacman package; everything else is the system's.
    nice -n 10 makepkg --config "$stage/makepkg.conf" --force --cleanbuild --nodeps --log
)

package=$(find "$stage/pkg" -name "hikasa-$version-*.pkg.tar.zst" | head -1)
[[ -n $package ]] || die "makepkg wrote no package"
install -m644 "$package" "$dist/"
mkdir -p "$stage/unpack"
bsdtar -xf "$package" -C "$stage/unpack"
"$repo/packaging/check-linux-binaries.sh" "$stage/unpack/usr/bin/hikasa" "$stage/unpack/usr/bin/hikasa-gui"
if command -v namcap >/dev/null; then namcap "$dist/$(basename "$package")" || true; fi
write_sums
echo "$dist/$(basename "$package")"
