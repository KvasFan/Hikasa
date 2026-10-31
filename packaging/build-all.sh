#!/usr/bin/env bash
# Every package, one after the other, into the output folder with SHA256SUMS. A format that cannot run here stops the
# run with the reason; the formats before it stay in the folder.
source "$(dirname "$0")/common.sh"
cd "$repo/packaging"
./package-targz.sh
./package-deb.sh
./package-appimage.sh
./package-arch.sh
./package-windows-zip.sh
./package-windows-msi.sh
ls -l "$dist"
cat "$dist/SHA256SUMS"
