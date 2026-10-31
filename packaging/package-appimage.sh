#!/usr/bin/env bash
# Hikasa-<version>-x86_64.AppImage. appimagetool and the runtime are pinned downloads; appimagetool runs extracted, so
# the build needs no FUSE. See packaging/README.md for how to start the result.
source "$(dirname "$0")/common.sh"
"$repo/packaging/build-linux.sh"

appimagetool_version=1.9.1
appimagetool_sha256=ed4ce84f0d9caff66f50bcca6ff6f35aae54ce8135408b3fa33abfc3cb384eb0
runtime_version=20251108
runtime_sha256=2fca8b443c92510f1483a883f60061ad09b46b978b2631c807cd873a47ec260d

tool=$(fetch "https://github.com/AppImage/appimagetool/releases/download/$appimagetool_version/appimagetool-x86_64.AppImage" \
    "$appimagetool_sha256" appimagetool-x86_64.AppImage)
runtime=$(fetch "https://github.com/AppImage/type2-runtime/releases/download/$runtime_version/runtime-x86_64" \
    "$runtime_sha256" runtime-x86_64)
chmod +x "$tool"

appdir=$work/stage/appimage/Hikasa.AppDir
rm -rf "$appdir"
mkdir -p "$appdir"
"$repo/packaging/install-tree.sh" "$appdir" "$linux_bin"
stage_metainfo "$appdir/usr/share/metainfo/io.github.kvasfan.hikasa.metainfo.xml"
doc=$appdir/usr/share/doc/hikasa
mkdir -p "$doc"
copy_licenses "$doc" "$linux_target"
install -m644 "$repo/packaging/appimage/RUNTIME-NOTICE.txt" "$doc/"

install -m755 "$repo/packaging/appimage/AppRun" "$appdir/AppRun"
install -m644 "$repo/packaging/hikasa.desktop" "$appdir/hikasa.desktop"
install -m644 "$repo/assets/brand/hicolor/256x256/apps/hikasa.png" "$appdir/hikasa.png"
ln -s hikasa.png "$appdir/.DirIcon"

out=$dist/Hikasa-$version-x86_64.AppImage
rm -f "$out"
(
    cd "$work/stage/appimage"
    ARCH=x86_64 VERSION=$version "$tool" --appimage-extract-and-run --no-appstream --runtime-file "$runtime" "$appdir" "$out"
)
write_sums
echo "$out"
