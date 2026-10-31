#!/usr/bin/env bash
# Smoke tests of the packages in the output folder. Everything happens below $work/smoke, with a private HOME and
# XDG folders and a private wine prefix; windows open on Xvfb only. Nothing is installed on the system.
#
#   smoke-test.sh [targz] [deb] [appimage] [arch] [windows-zip] [windows-msi]     default: all
#
# Each test: `hikasa version`, a folder and a text compare with known verdicts, then `hikasa gui open` (which must
# find hikasa-gui in that layout), `gui list`, `gui quit`, and the end of the process. Results go to
# $work/smoke/results.txt, one line per package.
source "$(dirname "$0")/common.sh"
require xvfb-run
if [[ -z ${HIKASA_SMOKE_DISPLAY:-} ]]; then
    HIKASA_SMOKE_DISPLAY=1 exec env -u WAYLAND_DISPLAY xvfb-run -a "$0" "$@"
fi

root=$work/smoke
results=$root/results.txt
mkdir -p "$root"
: >>"$results"

fail() {
    echo "  FAIL: $*" >&2
    failures+=("$*")
}

report() { # <package>
    if ((${#failures[@]})); then
        echo "FAIL $1: ${failures[*]}" | tee -a "$results"
    else
        echo "PASS $1" | tee -a "$results"
    fi
    failures=()
}
failures=()

# Two folders with a known result: one same file, one different file, one file on the left only.
make_fixture() { # <folder>
    mkdir -p "$1/a" "$1/b"
    printf 'same\n' >"$1/a/same.txt"
    cp -p "$1/a/same.txt" "$1/b/same.txt"
    printf 'one\ntwo\nthree\n' >"$1/a/diff.txt"
    printf 'one\nTWO and more\nthree\n' >"$1/b/diff.txt"
    touch -r "$1/a/diff.txt" "$1/b/diff.txt"
    printf 'only\n' >"$1/a/only.txt"
}

isolate() { # <folder>: everything a program may write stays there
    mkdir -p "$1/home" "$1/config" "$1/cache" "$1/data" "$1/run"
    chmod 700 "$1/run"
    export HOME=$1/home XDG_CONFIG_HOME=$1/config XDG_CACHE_HOME=$1/cache XDG_DATA_HOME=$1/data XDG_RUNTIME_DIR=$1/run
    export HIKASA_CLIPBOARD_FILE=$1/clipboard PATH=/usr/bin:/bin
    unset HIKASA_GUI_EXE HIKASA_IPC_ENDPOINT
}

check_cli() { # <hikasa command…>, with $fixture and $to_path (how the program spells a path)
    local output code
    # A program that does not end is a failure, not a stalled test run.
    set -- timeout -s KILL 120 "$@"
    output=$("$@" version | tr -d '\r') || fail "version exit $?"
    [[ $output == "hikasa $version" ]] || fail "version says '$output'"
    code=0
    "$@" folder compare "$("$to_path" "$fixture/a")" "$("$to_path" "$fixture/b")" >"$fixture/folder.out" 2>&1 || code=$?
    [[ $code == 13 ]] || fail "folder compare exit $code"
    grep -q '1 same.*1 different.*1 left only' "$fixture/folder.out" || fail "folder compare summary: $(tail -1 "$fixture/folder.out")"
    code=0
    "$@" text compare "$("$to_path" "$fixture/a/diff.txt")" "$("$to_path" "$fixture/b/diff.txt")" >"$fixture/text.out" 2>&1 || code=$?
    [[ $code == 13 ]] || fail "text compare exit $code"
    grep -q '^+TWO and more' "$fixture/text.out" || fail "text compare output"
}

same_path() { printf '%s\n' "$1"; }

# The CLI starts the GUI, which is then asked to quit. <process> is the pattern of what must end.
gui_session() { # <process> <hikasa command…>
    local process=$1 list="" i
    shift
    "$@" gui open "$("$to_path" "$fixture/a")" "$("$to_path" "$fixture/b")" >"$fixture/gui.out" 2>&1 || {
        fail "gui open exit $?: $(tail -1 "$fixture/gui.out")"
        return
    }
    for i in $(seq 1 60); do
        list=$("$@" gui list 2>/dev/null | tr -d '\r')
        [[ -n $list ]] && break
        sleep 0.5
    done
    [[ $list == *folder_compare* ]] || fail "the view did not open: '$list'"
    if [[ -n ${check_icon:-} ]]; then
        xprop -name Hikasa -notype 32c _NET_WM_ICON | head -1 | grep -q '= 64, 64,' || fail "no 64 px window icon"
    fi
    "$@" gui quit >>"$fixture/gui.out" 2>&1 || fail "gui quit exit $?"
    for i in $(seq 1 40); do
        pgrep -f "$process" >/dev/null || return 0
        sleep 0.5
    done
    fail "the GUI did not end"
    pkill -f "$process"
}

test_linux_tree() { # <label> <folder with hikasa and hikasa-gui>
    local name=$1 bin=$2
    fixture=$root/$name/fixture
    to_path=same_path
    make_fixture "$fixture"
    check_cli "$bin/hikasa"
    gui_session "$bin/hikasa-gui" "$bin/hikasa"
}

test_targz() (
    set +e
    folder=$root/targz
    rm -rf "$folder"
    mkdir -p "$folder/unpack"
    tar -xzf "$dist/hikasa-$version-x86_64-linux.tar.gz" -C "$folder/unpack"
    tree=$folder/unpack/hikasa-$version-x86_64-linux
    for file in hikasa hikasa-gui LICENSE-MIT LICENSE-APACHE THIRD_PARTY_LICENSES.txt README.txt; do
        [[ -f $tree/$file ]] || fail "missing $file"
    done
    isolate "$folder"
    check_icon=1
    test_linux_tree targz "$tree"
    report targz
)

test_deb() (
    set +e
    folder=$root/deb
    deb=$dist/hikasa_$version-1_amd64.deb
    rm -rf "$folder"
    mkdir -p "$folder/ar" "$folder/data" "$folder/control"
    python3 -I "$repo/packaging/check-deb.py" "$deb" || fail "archive layout"
    (cd "$folder/ar" && ar x "$deb")
    tar -xJf "$folder/ar/control.tar.xz" -C "$folder/control"
    tar -xJf "$folder/ar/data.tar.xz" -C "$folder/data"
    (cd "$folder/data" && md5sum --check --quiet ../control/md5sums) || fail "md5sums"
    for field in Package Version Architecture Maintainer Installed-Size Depends Section Priority Description; do
        grep -q "^$field: " "$folder/control/control" || fail "no $field"
    done
    for file in usr/share/doc/hikasa/copyright usr/share/doc/hikasa/LICENSE-MIT usr/share/doc/hikasa/THIRD_PARTY_LICENSES.txt \
        usr/share/applications/hikasa.desktop usr/share/mime/packages/hikasa.xml usr/share/icons/hicolor/48x48/apps/hikasa.png; do
        [[ -f $folder/data/$file ]] || fail "missing $file"
    done
    desktop-file-validate "$folder/data/usr/share/applications/hikasa.desktop" >/dev/null || fail "desktop file"
    isolate "$folder"
    test_linux_tree deb "$folder/data/usr/bin"
    report deb
)

test_arch() (
    set +e
    folder=$root/arch
    rm -rf "$folder"
    mkdir -p "$folder/unpack"
    package=$(find "$dist" -maxdepth 1 -name "hikasa-$version-*-x86_64.pkg.tar.zst" | head -1)
    [[ -n $package ]] || { fail "no package"; report arch; exit; }
    bsdtar -xf "$package" -C "$folder/unpack"
    for file in usr/share/licenses/hikasa/LICENSE-MIT usr/share/licenses/hikasa/THIRD_PARTY_LICENSES.txt \
        usr/share/bash-completion/completions/hikasa usr/share/zsh/site-functions/_hikasa \
        usr/share/fish/vendor_completions.d/hikasa.fish .PKGINFO; do
        [[ -f $folder/unpack/$file ]] || fail "missing $file"
    done
    isolate "$folder"
    test_linux_tree arch "$folder/unpack/usr/bin"
    report arch
)

test_appimage() (
    set +e
    folder=$root/appimage
    image=$dist/Hikasa-$version-x86_64.AppImage
    rm -rf "$folder"
    mkdir -p "$folder"
    cp "$image" "$folder/Hikasa.AppImage"
    chmod +x "$folder/Hikasa.AppImage"
    ln -s Hikasa.AppImage "$folder/hikasa"
    isolate "$folder"
    fixture=$folder/fixture
    to_path=same_path
    make_fixture "$fixture"

    # Over FUSE: the name `hikasa`, the --cli argument, the GUI through the CLI (the mount must outlive the CLI).
    check_cli "$folder/hikasa"
    check_cli "$folder/Hikasa.AppImage" --cli
    gui_session '/usr/bin/hikasa-gui' "$folder/hikasa"

    # D-134 inside the mount: the GUI replaces itself exactly once, keeps its arguments and lives in the mount.
    mkdir "$folder/mount"
    "$folder/Hikasa.AppImage" --appimage-mount >"$folder/mountpoint" &
    mounter=$!
    for i in $(seq 1 40); do [[ -s $folder/mountpoint ]] && break; sleep 0.25; done
    mountpoint=$(head -1 "$folder/mountpoint")
    gui=$mountpoint/usr/bin/hikasa-gui cli=$mountpoint/usr/bin/hikasa
    strace -f -qq -e trace=execve -o "$folder/execve.log" "$gui" "$fixture/a" "$fixture/b" >"$folder/d134.gui" 2>&1 &
    tracer=$!
    for i in $(seq 1 60); do
        list=$("$cli" gui list 2>/dev/null) && [[ -n $list ]] && break
        sleep 0.5
    done
    process=$(pgrep -f "^$gui" | head -1)
    {
        echo "$list"
        readlink "/proc/$process/exe"
        tr '\0' '\n' <"/proc/$process/environ" | grep -a MALLOC_MMAP_THRESHOLD_
    } >"$folder/d134.out" 2>&1
    "$cli" gui quit >>"$folder/d134.out" 2>&1
    wait "$tracer"
    kill "$mounter" 2>/dev/null
    wait "$mounter" 2>/dev/null
    execs=$(grep -c "execve(\"$gui\"" "$folder/execve.log")
    [[ $execs == 2 ]] || fail "D-134: $execs execs of hikasa-gui, expected 2"
    grep -q 'folder_compare' "$folder/d134.out" || fail "D-134: the arguments were lost"
    grep -q "$mountpoint/usr/bin/hikasa-gui" "$folder/d134.out" || fail "D-134: not running from the mount"
    grep -q 'MALLOC_MMAP_THRESHOLD_=4194304' "$folder/d134.out" || fail "D-134: variable not set"

    # The default entry, with and without FUSE: the GUI, with its arguments, ended by the CLI.
    direct_gui() { # <AppImage command…>
        "$@" "$fixture/a" "$fixture/b" >"$folder/direct.out" 2>&1 &
        local started=$! i list=""
        for i in $(seq 1 60); do
            list=$("$folder/hikasa" gui list 2>/dev/null)
            [[ -n $list ]] && break
            sleep 0.5
        done
        [[ $list == *folder_compare* ]] || fail "$*: the view did not open"
        "$folder/hikasa" gui quit >/dev/null 2>&1
        wait "$started" || fail "$*: exit $?"
    }
    direct_gui "$folder/Hikasa.AppImage"
    direct_gui "$folder/Hikasa.AppImage" --appimage-extract-and-run

    # Without FUSE.
    output=$("$folder/Hikasa.AppImage" --appimage-extract-and-run --cli version)
    [[ $output == "hikasa $version" ]] || fail "extract-and-run says '$output'"
    report appimage
)

# Windows: paths through wine's Z: drive, and the wine server gone at the end of each package. wgpu's default on
# Windows, DX12, needs a shader compiler wine does not have (no Shader Model 5.1 arrays), so the window here uses OpenGL.
to_wine() { wine_path "$1"; }

wine_tests() { # <label> <folder with the .exe files>
    export WGPU_BACKEND=gl
    fixture=$root/$1/fixture
    to_path=to_wine
    make_fixture "$fixture"
    check_cli wine "$2/hikasa.exe"
    gui_session 'hikasa-gui\.exe' wine "$2/hikasa.exe"
}

test_windows_zip() ( # <arch>
    set +e
    arch=$1
    folder=$root/windows-zip-$arch
    rm -rf "$folder"
    mkdir -p "$folder/unpack"
    unzip -q "$dist/hikasa-$version-windows-$arch-portable.zip" -d "$folder/unpack"
    tree=$folder/unpack/hikasa-$version-windows-$arch
    for file in hikasa.exe hikasa-gui.exe hikasa-portable.toml LICENSE-MIT LICENSE-APACHE THIRD_PARTY_LICENSES.txt; do
        [[ -f $tree/$file ]] || fail "missing $file"
    done
    isolate "$folder"
    wine_env smoke-zip
    wine_tests "windows-zip-$arch" "$tree"
    # Portable mode: the settings live beside the programs, not in the prefix.
    [[ -n $(find "$tree" -name '*.toml' ! -name hikasa-portable.toml -print -quit) ]] || fail "no settings beside the programs"
    [[ -z $(find "$WINEPREFIX/drive_c/users" -type d -name Hikasa -print -quit) ]] || fail "settings in the profile"
    wineserver -w
    report "windows-zip-$arch"
)

reg_value() { # <key> [<value name>]: the data, or nothing when absent
    if [[ -n ${2:-} ]]; then wine reg query "$1" /v "$2" 2>/dev/null; else wine reg query "$1" /ve 2>/dev/null; fi | tr -d '\r' | grep -E 'REG_(EXPAND_)?SZ' || true
}

# wine's msi ignores MSIINSTALLPERUSER, so the default install lands in the machine's scope here. ALLUSERS="" takes the
# per-user branch of the package (HKCU, the user's PATH) that the default takes on Windows; the folder it uses here is
# still Program Files, while Windows redirects the default install to %LOCALAPPDATA%\Programs.
test_windows_msi() ( # <arch> <per-user|per-machine>
    set +e
    arch=$1 scope=$2
    label=windows-msi-$arch-$scope
    folder=$root/$label
    rm -rf "$folder"
    mkdir -p "$folder"
    isolate "$folder"
    wine_env "smoke-$scope"
    msi=$(wine_path "$dist/hikasa-$version-windows-$arch.msi")
    if [[ $scope == per-user ]]; then
        all_users=ALLUSERS= classes='HKCU\Software\Classes' environment='HKCU\Environment'
    else
        all_users=ALLUSERS=1 classes='HKLM\Software\Classes'
        environment='HKLM\SYSTEM\CurrentControlSet\Control\Session Manager\Environment'
    fi
    # ALLUSERS= comes last: wine's msiexec reads the next argument as the value of an empty one.
    wine msiexec /i "$msi" /qn ADDTOPATH=1 "$all_users" || fail "install exit $?"
    wineserver -w
    exe=$(find "$WINEPREFIX/drive_c" -name hikasa-gui.exe -path '*Hikasa*' | head -1)
    [[ -n $exe ]] || { fail "nothing installed"; report "$label"; exit; }
    tree=$(dirname "$exe")
    echo "  installed in ${tree#"$WINEPREFIX"/}"
    for file in hikasa.exe LICENSE-MIT LICENSE-APACHE THIRD_PARTY_LICENSES.txt; do
        [[ -f $tree/$file ]] || fail "missing $file"
    done
    [[ -n $(find "$WINEPREFIX/drive_c" -name Hikasa.lnk) ]] || fail "no Start Menu shortcut"
    reg_value "$classes\\.hikasa-pkg" | grep -q Hikasa.Package || fail "no association"
    reg_value "$classes\\Hikasa.Package\\shell\\open\\command" | grep -q 'hikasa-gui.exe' || fail "no open command"
    reg_value "$environment" PATH | grep -qi 'hikasa' || fail "not on PATH"
    wine_tests "$label" "$tree"
    wine msiexec /x "$msi" /qn "$all_users" || fail "uninstall exit $?"
    wineserver -w
    [[ ! -e $tree/hikasa.exe ]] || fail "hikasa.exe left behind"
    [[ -z $(find "$WINEPREFIX/drive_c" -name Hikasa.lnk) ]] || fail "shortcut left behind"
    [[ -z $(reg_value "$classes\\.hikasa-pkg") ]] || fail "association left behind"
    if reg_value "$environment" PATH | grep -qi 'hikasa'; then fail "PATH left behind"; fi
    wineserver -w
    report "$label"
)

targets=("$@")
((${#targets[@]})) || targets=(targz deb appimage arch windows-zip windows-msi)
for target in "${targets[@]}"; do
    case $target in
        targz | deb | appimage | arch) "test_$target" ;;
        windows-zip) for arch in x64 x86; do test_windows_zip "$arch"; done ;;
        windows-msi)
            for arch in x64 x86; do
                test_windows_msi "$arch" per-machine
                test_windows_msi "$arch" per-user
            done
            ;;
        *) die "unknown test $target" ;;
    esac
done
