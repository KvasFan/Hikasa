# Metadata Views: Media Compare, Version Compare, Registry Compare

These three views share a generic **key/value tree comparison** widget
(`hikasa-metadata`): two trees of groups and items aligned by key, with colouring
(red difference, purple orphan, blue unimportant), display filters (Show All /
Differences / Same / None), Next/Previous Difference Item (Ctrl+N/P), Find (Ctrl+F),
Copy (selection as text), Select All, a Text Details panel (full value of the current
item), side-by-side or over-under layout, display font commands, File Info headers,
and *Item Is Important* toggling per item (persisted in the session's Importance tab).

## 1. Media Compare

- **MD-001** (P1): Compares tags of audio/video containers: MP3 (ID3v1/ID3v2), MP4/
  M4A/M4B/M4V (ilst atoms), FLAC (Vorbis comments + pictures), WAV (RIFF INFO), plus
  APE/OGG/Opus where `lofty` supports them. Masks: `*.flac;*.m4a;*.m4b;*.m4p;*.m4v;
  *.mp3;*.mp4;*.wav`.
- **MD-002** (P1): Groups: **Metadata** (every tag: title, artist, album, album artist,
  composer, genre, year/dates, track/disc numbers and totals, comment, lyrics, BPM,
  ISRC, copyright, encoder, URLs, podcast fields, TV fields, pictures with type/mime/
  size/hash, …) and **Audio** (format, duration, bitrate, sample rate, channels, data
  hash). Session rule: which groups/tags are important (default: all metadata
  important; audio data important).
- **MD-003** (P2): Play / Play Both (preview audio through the system player).
- **MD-004** (P1): Report: side-by-side or summary.

## 2. Version Compare

- **VR-001** (P1): Compares version information of executables/libraries. Windows PE:
  fixed `VS_FIXEDFILEINFO` (file/product version, flags, OS, type, subtype, date,
  signature, structure version), string tables per translation (Company Name, File
  Description, File Version, Internal Name, Legal Copyright/Trademarks, Original
  Filename, Product Name/Version, Private/Special Build, Comments, Language), and **PE
  header** fields (checksum, date, linker version, OS/image version, processor,
  subsystem/UI, DLL flags such as NX compatible, dynamic base, high-entropy VA, CFG,
  AppContainer, Terminal-server aware; characteristics such as executable, DLL,
  large-address-aware, debug stripped…). ELF and Mach-O (P2): architecture, type,
  interpreter, SONAME, NEEDED, build-id, rpath, notes.
- **VR-002** (P1): Masks: `*.exe;*.dll;*.ocx;*.sys;*.drv;*.scr;*.bpl;*.so;*.dylib`.
  Also feeds the folder view **Version** column and the "Compare versions" criterion.
- **VR-003** (P1): Report: side-by-side or summary; `ignore-unimportant`.

## 3. Registry Compare

- **RG-001** (P1): Compares Windows registry **export files** (`.reg`, both REGEDIT4
  ANSI and Windows Registry Editor Version 5.00 UTF-16) as trees of keys and typed values
  (REG_SZ, EXPAND_SZ, MULTI_SZ, DWORD, QWORD, BINARY, NONE). Keys sorted and aligned by
  name; values coloured by character differences; Hex Details panel for binary values.
- **RG-002** (P2, Windows only): Live registries (local `reg:\\HKEY_LOCAL_MACHINE\…`,
  remote `reg:\\Machine\HKEY_USERS\…`), editing (New Key, New Value (Ins), Modify,
  Rename (F2), Delete, Copy to Right/Left, Export key / Export All to `.reg`), Up One
  Level (Left/Right/Both), Set as Base Keys / Set Both / Set on Other Side, Reload /
  Recompare, Expand/Collapse All.
- **RG-003** (P1): Report: side-by-side or summary.
