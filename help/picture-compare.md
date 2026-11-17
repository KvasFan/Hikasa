# hikasa picture compare

`hikasa picture compare <left> <right>` compares two pictures pixel by pixel: PNG (an APNG's default image), JPEG,
GIF, BMP, ICO, TIFF, WebP, TGA, PNM, QOI, OpenEXR and Radiance HDR, recognised by their content whatever their name.
Each picture is decoded to 8-bit RGBA by fixed rules, so the same pictures give the same result on every system: the
EXIF orientation is applied, animations and multi-page files compare their first frame, colour profiles other than
sRGB are ignored (warning `color_profile_ignored`), and HDR and OpenEXR pictures go through a tone map. DDS, AVIF and
SVG are not supported (exit 100, `unsupported`). The [exit code](topic:exit-codes) is the verdict.

`-` reads standard input and `clipboard://` the clipboard's image ([Using the command line](topic:cli));
[archive](topic:archives) members are read like files.

## Rules

| Flag | Meaning |
|---|---|
| `--tolerance <0-255>` | a pixel whose largest channel difference is at most this is *similar* (an unimportant difference), above it *different* (default 0); two fully transparent pixels are the same whatever their colour |
| `--ignore-unimportant` (`--iu`) | similar pixels count as same for the verdict: 12 becomes 2 |
| `--auto-scale` | the smaller picture is resampled to the other's size (an exact fixed-point bilinear resampling) |
| `--offset <dx>,<dy>` | the right picture's origin in left coordinates (default `0,0`; each from −1048576 to 1048576): `--offset -3,10` |
| `--left-orientation <list>`, `--right-orientation <list>` | rotate or flip a side: `none`, `cw`, `ccw`, `180`, `flip-h`, `flip-v`, `transpose`, `transverse`; a comma list is applied in order (`cw,flip-h`) |
| `--tone-map clamp\|reinhard` | how HDR and OpenEXR values become 8-bit (default `clamp`) |
| `--max-megapixels <n>` | the largest picture decoded, 1 to 4096 (default 256, of 2²⁰ pixels); checked from the header before any pixel is decoded: a larger one is `too_large` |

The positions covered by either picture are compared: a position only one picture covers counts as that pixel
against nothing (an opaque one differs).

## Formats

A side is decoded through the format of the *picture* kind its name matches (*Picture Files* by default); a format
with a load conversion (an *External* program) is decoded from the program's output. `--format <id|name>` gives both
sides a format, `--left-format`/`--right-format` one side (with `--format` the combination is refused); the format
must be of the picture kind ([hikasa format](topic:format)). `--stdin-name <name>` names standard input.

## Output

```
left: a.png 6000×4000 png rgb8
right: b.jpg 6000×4000 jpeg rgb8
pixels 24000000  same 23760000 (99.0000 %)  similar 200000 (0.8333 %)  different 40000 (0.1667 %)  max distance 87  bounds 120,40 800×600
probe 10,20: left 10,20 #ff0000ff  right 10,20 #fe0000ff  distance 1 similar
verdict: important_differences
```

One line per side (spec, size, the decoder, the colour type of the source), the statistics (counts and percentages of
same, similar and different pixels, the largest distance, the bounds of what differs, `none` when nothing does),
one line per `--probe <x>,<y>` (repeatable: both pixels at that position of the compared area, `—` for a side that
does not cover it, the distance and the class), then the verdict line.

- `--diff-output <spec>` writes the difference image as a PNG at full resolution: a file (replaced atomically, also
  inside an archive), `-` (standard output: then nothing else is printed there) or `clipboard://`. `--diff-pane
  difference|left|right` chooses the pane (default the difference image); `--mode tolerance|range|blend` the
  difference image's mode (default tolerance: similar and different pixels in the theme's colours, same pixels shaded
  by their brightness; range: black for same, the distance as yellow brightness; blend: the right picture over the
  left, `--blend <0-100>` its weight in percent, default 50); `--checkerboard` draws transparent pixels over the
  checkerboard. The colours are the `[picture]` colours of the theme ([hikasa settings](topic:settings)).
- `--json` prints `hikasa.picture_compare.v1` ([JSON output](topic:json-output)): both sides (`spec`, `size`,
  `mtime`, `format`, `decoder`, `width`, `height`, `color`, `bit_depth`, `has_alpha`, `frames`, the EXIF
  `orientation` applied, the `tone_map` of an HDR picture, `converted`), the `options`, the `geometry` (`area`, the
  side Auto Scale `scaled` and its size), the `verdict`, the `stats` (`pixels`, `same`, `similar`, `different`,
  `left_only`, `right_only`, the percentages, `max_distance`, `mean_distance`, `bounds`), the `probes` and
  `diff_output` (the image written, or `null`). The JSON Schema is `docs/schemas/picture_compare.v1.json`.
- Warnings: `color_profile_ignored`, `aspect_ratio_differs` (Auto Scale resampled a picture whose proportions differ
  from the other's by more than 1 %), and with `--verbose` `frames_ignored` (an animation or a multi-page file) and
  `orientation_applied` (an EXIF orientation).

## Reports

`--report summary|side-by-side` (`--layout`) writes a report: *summary* (the default layout; text or HTML) gives both
pictures' file information, the options, the statistics and the verdict; *side-by-side* (HTML only: add `--html` or
`--html-mono`) embeds the left, right and difference images, scaled down to at most 1,024 pixels on the long side
unless the word `full-size`. The report words `mode-tolerance`, `mode-range`, `mode-blend` and `checkerboard` shape
its difference image, `ignore-unimportant` the verdict; `--title`, `--options`, `--output-options`, `--output-to
<file>|clipboard|printer|browser` (`--output <file>`), `--dry-run` as for every report ([hikasa report](topic:report)).
A script writes the same reports with `picture-report` ([hikasa script](topic:script)); a Picture Compare session runs
with `hikasa session run` ([hikasa session](topic:session)).

## Exit codes

1 identical bytes; 2 different bytes but every pixel the same (a PNG saved again as BMP); 12 similar pixels only (2
with `--ignore-unimportant`); 13 different pixels; 100 an error. Refused before anything is read (exit 100):
`--metadata` (picture metadata comparison is not available in this version, `unsupported`), `--diff-output -` with
`--json`, `--diff-output -` with a report on standard output, `--checkerboard` or `--diff-pane` without
`--diff-output`, `--blend` outside `--mode blend`, `--report side-by-side` without `--html`, a bad word, an unknown
format or one that is not of the picture kind. Also 100: a picture that does not decode (`invalid_data`), is too
large (`too_large`) or is not supported (`unsupported`), a file that cannot be read, a Ctrl-C.

## In the window

`--gui` opens the comparison in the window's Picture Compare view with the same words (`--wait` waits and exits with
its code, `--no-launch` does not start the window); the words are checked here first. The report flags,
`--diff-output` and `--probe` do not apply with it (exit 100). Until the window has the view it answers
`unsupported_view` (exit 100).

`hikasa quick` and the rules-based content test of [hikasa folder compare](topic:folder-compare) compare pictures by
their pixels too, with the tolerance, Auto Scale, tone map and size limit of the Picture Compare defaults session.

## Examples

```
hikasa picture compare old.png new.png
hikasa picture compare a.jpg b.jpg --tolerance 8 --iu
hikasa picture compare scan1.tif scan2.tif --auto-scale --right-orientation cw --probe 100,200
hikasa picture compare a.png b.png --diff-output diff.png --mode range --checkerboard
hikasa picture compare a.exr b.exr --tone-map reinhard --json
hikasa picture compare a.png b.png --report side-by-side --html --output-to browser
```

See [hikasa hex compare](topic:hex-compare) and [Contents](topic:index).
