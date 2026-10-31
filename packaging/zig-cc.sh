#!/usr/bin/env bash
# Linker for the portable Linux binaries, set by build-linux.sh: `zig cc` links against the symbol versions of an
# older glibc (HIKASA_GLIBC), whatever the system has, so the binaries also start on older distributions. Two
# arguments are dropped: rustc asks for its own lld, and zig warns about the linker's -O1.
args=()
for arg in "$@"; do
    case $arg in
        -fuse-ld=lld | -B*/gcc-ld | -Wl,-O1) ;;
        *) args+=("$arg") ;;
    esac
done
exec zig cc -target "x86_64-linux-gnu.$HIKASA_GLIBC" "${args[@]}"
