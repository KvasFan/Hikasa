#!/usr/bin/env bash
# Generates the brand files from assets/brand/logo-light.svg with rsvg-convert and magick:
#   assets/brand/hicolor/<N>x<N>/apps/hikasa.png   the Linux icon theme (16…512)
#   assets/brand/hikasa.ico                        the Windows icon (.exe resources, installer)
#   assets/brand/window-icon-64.rgba               the window icon, raw RGBA, 64×64 (include_bytes! in hikasa-gui)
# The results stay in git: the build needs them and rsvg-convert is not a build dependency. Run again after a
# change to the logo. The light logo is the icon: it also reads on dark panels.
#
# Optional: --preview <file.png> also writes a sheet of the sizes, 16 px enlarged, on light and dark.
set -euo pipefail
cd "$(dirname "$0")/.."

for tool in rsvg-convert magick; do
    command -v "$tool" >/dev/null || { echo "generate-brand.sh: $tool is required" >&2; exit 1; }
done

svg=assets/brand/logo-light.svg
out=assets/brand

render() { # <size> <file>
    rsvg-convert --width "$1" --height "$1" --output "$2" "$svg"
    magick "$2" -define png:exclude-chunks=date,time -strip "PNG32:$2"
}

for n in 16 22 24 32 48 64 128 256 512; do
    mkdir -p "$out/hicolor/${n}x${n}/apps"
    render "$n" "$out/hicolor/${n}x${n}/apps/hikasa.png"
done

work=$(mktemp -d)
trap 'rm -rf "$work"' EXIT

ico_sizes=()
for n in 16 20 24 32 40 48 64; do
    render "$n" "$work/$n.png"
    ico_sizes+=("$work/$n.png")
done
magick "${ico_sizes[@]}" "$work/small.ico"
# magick stores 256×256 uncompressed (260 KiB): that entry is added as the PNG itself, as Windows expects it.
python3 -I - "$work/small.ico" "$out/hicolor/256x256/apps/hikasa.png" "$out/hikasa.ico" <<'PY'
import struct, sys
ico = open(sys.argv[1], "rb").read()
png = open(sys.argv[2], "rb").read()
count = struct.unpack_from("<H", ico, 4)[0]
directory = 6 + 16 * count
entries = b""
for i in range(count):
    entry = bytearray(ico[6 + 16 * i : 22 + 16 * i])
    struct.pack_into("<I", entry, 12, struct.unpack_from("<I", entry, 12)[0] + 16)
    entries += entry
last = struct.pack("<BBBBHHII", 0, 0, 0, 0, 1, 32, len(png), len(ico) + 16)
with open(sys.argv[3], "wb") as f:
    f.write(struct.pack("<HHH", 0, 1, count + 1) + entries + last + ico[directory:] + png)
PY

magick "$out/hicolor/64x64/apps/hikasa.png" -depth 8 "RGBA:$out/window-icon-64.rgba"

if [[ ${1:-} == --preview ]]; then
    sheet=${2:?--preview needs a file}
    row() { # <background> <svg> <file>
        local cells=()
        for n in 16 24 32 48 64 128; do
            rsvg-convert --width "$n" --height "$n" --output "$work/p$n.png" "$2"
            magick -size 140x140 "xc:$1" "$work/p$n.png" -gravity center -composite "$work/c$n.png"
            cells+=("$work/c$n.png")
        done
        rsvg-convert --width 16 --height 16 --output "$work/p16.png" "$2"
        magick -size 140x140 "xc:$1" \( "$work/p16.png" -filter point -resize 128x128 \) -gravity center -composite "$work/z.png"
        cells+=("$work/z.png")
        magick "${cells[@]}" +append "$3"
    }
    row '#ffffff' assets/brand/logo-light.svg "$work/r1.png"
    row '#1e1e1e' assets/brand/logo-dark.svg "$work/r2.png"
    row '#1e1e1e' assets/brand/logo-light.svg "$work/r3.png"
    row '#d9d9d9' assets/brand/logo-light.svg "$work/r4.png"
    magick "$work/r1.png" "$work/r2.png" "$work/r3.png" "$work/r4.png" -append "$sheet"
fi
