#!/usr/bin/env bash
# Measures what an ELF binary needs: its libraries, the glibc floor from `objdump -T`, and whether the code was built
# for baseline x86-64. Dependencies select AVX2 code at run time (blake3, memchr), so VEX instructions are expected;
# the instructions a target-cpu above x86-64 spreads through all code (BMI2, MOVBE) are not, nor a floor above
# HIKASA_GLIBC_MAX when that is set.
set -euo pipefail
status=0
for binary in "$@"; do
    floor=$(objdump -T "$binary" | grep -o 'GLIBC_[0-9.]*' | sort -Vu | tail -1)
    needed=$(objdump -p "$binary" | awk '/NEEDED/ {printf "%s ", $2}')
    total=$(objdump -d --no-show-raw-insn "$binary" | awk '{print $2}' | grep -c . || true)
    bmi=$(objdump -d --no-show-raw-insn "$binary" | awk '{print $2}' | grep -cE '^(shlx|shrx|sarx|mulx|rorx|bzhi|pdep|pext|andn|movbe)' || true)
    printf '%s: glibc floor %s; needs %s; %s of %s instructions are BMI/MOVBE\n' "$binary" "$floor" "$needed" "$bmi" "$total"
    # One per thousand would mean a machine-wide target-cpu: with it, every shift and multiply uses them.
    if ((bmi * 1000 > total)); then
        echo "  not a baseline x86-64 build" >&2
        status=1
    fi
    if [[ -n ${HIKASA_GLIBC_MAX:-} ]] && [[ $(printf '%s\n%s\n' "GLIBC_$HIKASA_GLIBC_MAX" "$floor" | sort -V | tail -1) != "GLIBC_$HIKASA_GLIBC_MAX" ]]; then
        echo "  glibc floor above $HIKASA_GLIBC_MAX" >&2
        status=1
    fi
done
exit "$status"
