# Sourced by every script in packaging/: the paths, the version from Cargo.toml and the small helpers they share.
# See packaging/README.md.
#
#   HIKASA_WORK   scratch folder for builds, downloads and tests   (default /var/tmp/hikasa-work/release)
#   HIKASA_DIST   output folder, git-ignored                       (default target/dist in the repository)
#   HIKASA_JOBS   cargo build jobs                                 (default 6: LTO builds on a shared machine)
#   HIKASA_GLIBC  glibc the Linux binaries link against            (default 2.31)
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
work=${HIKASA_WORK:-/var/tmp/hikasa-work/release}
dist=${HIKASA_DIST:-$repo/target/dist}
jobs=${HIKASA_JOBS:-6}
glibc=${HIKASA_GLIBC:-2.31}

# One target folder per platform: sharing one, each platform rebuilds what depends on a build script another one ran.
target_root=${CARGO_TARGET_DIR:-$work/target}

export TMPDIR=$work/tmp
mkdir -p "$TMPDIR" "$dist"

# The one source of the version.
version=$(python3 -I -c 'import sys, tomllib; print(tomllib.load(open(sys.argv[1], "rb"))["workspace"]["package"]["version"])' "$repo/Cargo.toml")

# Release builds strip symbols; the profile itself stays as the repository has it.
export CARGO_PROFILE_RELEASE_STRIP=symbols

# RUSTFLAGS is set here on purpose: it replaces `build.rustflags` of any cargo configuration, so a machine's
# `target-cpu=native` cannot reach a package. The two paths keep the build machine out of the panic messages.
base_rustflags="--remap-path-prefix=$repo=/hikasa --remap-path-prefix=$HOME/.cargo=/cargo --remap-path-prefix=$HOME/.rustup=/rustup"

linux_target=x86_64-unknown-linux-gnu
linux_bin=$target_root/linux/$linux_target/release

windows_target() { # <x64|x86>
    case $1 in
        x64) echo x86_64-pc-windows-gnu ;;
        x86) echo i686-pc-windows-gnu ;;
        *) die "unknown Windows architecture $1" ;;
    esac
}

die() {
    echo "${0##*/}: $*" >&2
    exit 1
}

require() { # <tool>…
    local tool
    for tool in "$@"; do
        command -v "$tool" >/dev/null || die "$tool is required (see packaging/README.md)"
    done
}

windows_bin() { # <x64|x86>
    echo "$target_root/windows-$1/$(windows_target "$1")/release"
}

# A pinned download: <url> <sha256> <file name>. Each tool gets a folder of its own under $work/downloads; the file
# is kept there, checked on every use, and never comes from anywhere else.
fetch() {
    local url=$1 sum=$2 name=$3 dir file
    dir=$work/downloads/${name%.*}-${sum:0:12}
    file=$dir/$name
    mkdir -p "$dir"
    if [[ ! -f $file ]]; then
        require curl
        echo "downloading $url" >&2
        curl --fail --location --silent --show-error --proto '=https' --tlsv1.2 --output "$file.part" "$url"
        mv "$file.part" "$file"
    fi
    echo "$sum  $file" | sha256sum --check --quiet >&2 || die "$file does not match its pinned sha256"
    echo "$file"
}

# Environment for wine: a private prefix and a private HOME, no desktop integration (winemenubuilder would write menu
# entries and file associations for the real user), no Mono or Gecko prompts.
wine_env() { # <prefix name>
    export WINEPREFIX=$work/wine/$1 HOME=$work/wine/home-$1
    export XDG_CONFIG_HOME=$HOME/.config XDG_DATA_HOME=$HOME/.local/share XDG_CACHE_HOME=$HOME/.cache
    export WINEDEBUG=-all WINEDLLOVERRIDES='winemenubuilder.exe=d;mscoree=d;mshtml=d'
    mkdir -p "$WINEPREFIX" "$HOME"
}

# A Windows path for wine, through the Z: drive that maps /.
wine_path() { # <unix path>
    local path=$1
    printf 'Z:%s\n' "${path//\//\\}"
}

# SHA256SUMS holds every package in the output folder.
write_sums() {
    (
        cd "$dist"
        find . -maxdepth 1 -type f -iname 'hikasa*' ! -name '*.part' -printf '%f\n' | LC_ALL=C sort | xargs -r sha256sum >SHA256SUMS
    )
}

# THIRD_PARTY_LICENSES.txt for a list of targets, cached per target list and Cargo.lock under $work/licenses.
third_party_licenses() { # <target>… ; prints the file
    local key file
    key=$(cat "$repo/Cargo.lock" "$repo/packaging/third-party-licenses.py" <(printf '%s\n' "$@") | sha256sum | cut -c1-16)
    file=$work/licenses/$key/THIRD_PARTY_LICENSES.txt
    if [[ ! -f $file ]]; then
        mkdir -p "$(dirname "$file")"
        python3 -I "$repo/packaging/third-party-licenses.py" --manifest-path "$repo/Cargo.toml" --output "$file" "$@" >&2
    fi
    echo "$file"
}

# The files every package carries beside the executables, copied into <folder>.
copy_licenses() { # <folder> <target>…
    local folder=$1
    shift
    install -m644 "$repo/LICENSE-MIT" "$repo/LICENSE-APACHE" "$(third_party_licenses "$@")" "$folder/"
}

# The metainfo file with this version in its release list: a version the file does not name yet is added on top, so
# a package never contradicts itself between the bump and the release notes.
stage_metainfo() { # <output file>
    local src=$repo/packaging/io.github.kvasfan.hikasa.metainfo.xml
    if grep -q "<release version=\"$version\"" "$src"; then
        install -m644 "$src" "$1"
    else
        sed "s|^\( *\)<releases>|&\n\1  <release version=\"$version\" date=\"$(date -u +%F)\"/>|" "$src" >"$1"
        chmod 644 "$1"
    fi
}
