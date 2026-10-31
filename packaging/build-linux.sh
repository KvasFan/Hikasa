#!/usr/bin/env bash
# Builds `hikasa` and `hikasa-gui` for the Linux packages: baseline x86-64, linked with zig against an old glibc.
# check-linux-binaries.sh then prints the measured glibc floor and checks the baseline.
source "$(dirname "$0")/common.sh"
require cargo zig objdump

export CARGO_TARGET_DIR=$target_root/linux
export HIKASA_GLIBC=$glibc
export ZIG_GLOBAL_CACHE_DIR=$work/zig-cache ZIG_LOCAL_CACHE_DIR=$work/zig-cache
export CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_LINKER=$repo/packaging/zig-cc.sh
export RUSTFLAGS="-C target-cpu=x86-64 $base_rustflags"

cd "$repo"
nice -n 10 cargo build --locked --release --jobs "$jobs" --target "$linux_target" -p hikasa-cli -p hikasa-gui

"$repo/packaging/check-linux-binaries.sh" "$linux_bin/hikasa" "$linux_bin/hikasa-gui"
