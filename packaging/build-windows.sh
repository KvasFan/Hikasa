#!/usr/bin/env bash
# Builds `hikasa.exe` and `hikasa-gui.exe` for the Windows packages with the mingw GNU targets, each with its icon and
# version resources (windows/hikasa.rc.in, compiled here with windres and handed to the linker).
#
#   build-windows.sh [x64] [x86]     default: both
#
# +crt-static links libgcc and winpthreads into the executables; the check at the end refuses anything but system DLLs
# and a GUI executable that is not a windows-subsystem program. One `cargo rustc` per binary: the resources differ.
source "$(dirname "$0")/common.sh"
require cargo objdump

IFS=. read -r major minor patch <<<"${version%%[-+]*}"

build() { # <arch> <package> <binary> <description>
    local arch=$1 package=$2 name=$3 description=$4 target triple folder
    target=$(windows_target "$arch")
    triple=$([[ $arch == x64 ]] && echo x86_64-w64-mingw32 || echo i686-w64-mingw32)
    require "$triple-gcc" "$triple-windres"
    folder=$work/windows-res/$arch
    mkdir -p "$folder"
    sed -e "s|@ICON@|$repo/assets/brand/hikasa.ico|" -e "s|@NAME@|$name|g" -e "s|@DESCRIPTION@|$description|" \
        -e "s|@VERSION@|$version|g" -e "s|@MAJOR@|$major|g" -e "s|@MINOR@|$minor|g" -e "s|@PATCH@|$patch|g" \
        "$repo/packaging/windows/hikasa.rc.in" >"$folder/$name.rc"
    "$triple-windres" --output-format=coff --output "$folder/$name.res.o" "$folder/$name.rc"
    (
        cd "$repo"
        export CARGO_TARGET_DIR=$target_root/windows-$arch
        RUSTFLAGS="-C target-feature=+crt-static $base_rustflags" nice -n 10 cargo rustc --locked --release \
            --jobs "$jobs" --target "$target" -p "$package" --bin "$name" -- -C "link-arg=$folder/$name.res.o"
    )
}

check() { # <arch>
    local bin exe dlls headers kind
    bin=$(windows_bin "$1")
    for exe in hikasa hikasa-gui; do
        headers=$(objdump -p "$bin/$exe.exe")
        dlls=$(awk '/DLL Name:/ {print $3}' <<<"$headers" | sort -fu | paste -sd' ')
        kind=$(grep -E '^Subsystem' <<<"$headers" | sed 's/.*(\(.*\))/\1/')
        echo "$1 $exe.exe: $kind; DLLs: $dlls"
        if grep -qiE '(^| )lib[^ ]*\.dll' <<<"$dlls"; then
            die "$exe.exe needs a DLL that is not part of Windows"
        fi
        case $exe:$kind in
            hikasa:"Windows CUI" | hikasa-gui:"Windows GUI") ;;
            *) die "$exe.exe has the subsystem $kind" ;;
        esac
    done
}

arches=("$@")
((${#arches[@]})) || arches=(x64 x86)
for arch in "${arches[@]}"; do
    build "$arch" hikasa-cli hikasa "Hikasa command line"
    build "$arch" hikasa-gui hikasa-gui "Hikasa"
    check "$arch"
done
