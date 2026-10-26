#!/usr/bin/env bash
# Rotate the log files of a service.
set -euo pipefail

LOG_DIR="${LOG_DIR:-/var/log/demo}"
keep=7

rotate() {
    local file=$1
    for i in $(seq $keep -1 1); do
        if [ -f "$file.$i" ]; then
            mv "$file.$i" "$file.$((i + 1))"
        fi
    done
    echo "rotated $file" >&2
}

for f in "$LOG_DIR"/*.log; do
    rotate "$f"
done
exit 0
