#!/usr/bin/env python3
"""Checks the ar layout of a .deb the way dpkg reads it: the magic, debian-binary first with "2.0", then the two tar
members, even padding, nothing after them. Run with `python3 -I`; dpkg-deb is not needed."""

import sys

data = open(sys.argv[1], "rb").read()
assert data[:8] == b"!<arch>\n", "magic"
offset, names = 8, []
while offset < len(data):
    header = data[offset : offset + 60]
    assert len(header) == 60 and header[58:60] == b"`\n", f"header at {offset}"
    name = header[:16].decode().rstrip()
    size = int(header[48:58])
    mode = header[40:48].decode().rstrip()
    assert mode == "100644", f"{name}: mode {mode}"
    if not names:
        assert data[offset + 60 : offset + 60 + size] == b"2.0\n", "debian-binary content"
    names.append(name)
    offset += 60 + size + size % 2
assert offset == len(data), "trailing bytes"
assert names == ["debian-binary", "control.tar.xz", "data.tar.xz"], names
print("deb layout ok:", " ".join(names))
