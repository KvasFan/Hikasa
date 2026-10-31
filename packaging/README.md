# Packaging

Local scripts that build every release package of Hikasa. There is no CI job behind them: a maintainer runs them by
hand and uploads the results ([Release steps](#release-steps)).

```sh
packaging/build-all.sh     # every package into target/dist/, with SHA256SUMS
packaging/smoke-test.sh    # tests them; nothing is installed on the system
```

The version comes from `[workspace.package]` in `Cargo.toml`. No script names it. Two files must name it by nature, and
the scripts cope when they lag: `aur/PKGBUILD` (`package-arch.sh` works on a copy with the right `pkgver`) and the
metainfo file (a version it does not list is added to the packaged copy).

| Variable | Meaning | Default |
|---|---|---|
| `HIKASA_WORK` | scratch folder: builds, downloads, wine prefixes, smoke tests | `/var/tmp/hikasa-work/release` |
| `HIKASA_DIST` | output folder | `target/dist` (git-ignored) |
| `CARGO_TARGET_DIR` | build root, with one folder per platform below it | `$HIKASA_WORK/target` |
| `HIKASA_JOBS` | cargo jobs | `6` |
| `HIKASA_GLIBC` | glibc the Linux binaries link against | `2.31` |

## Packages

| Script | Result | Needs |
|---|---|---|
| `package-targz.sh` | `hikasa-<version>-x86_64-linux.tar.gz` | zig |
| `package-deb.sh` | `hikasa_<version>-1_amd64.deb` | zig, `xz`; not `dpkg-deb` |
| `package-appimage.sh` | `Hikasa-<version>-x86_64.AppImage` | zig, two pinned downloads |
| `package-arch.sh [--tag]` | `hikasa-<version>-1-x86_64.pkg.tar.zst` | `makepkg`; `namcap` is optional |
| `package-windows-zip.sh [x64] [x86]` | `hikasa-<version>-windows-<arch>-portable.zip` | mingw-w64, `zip` |
| `package-windows-msi.sh [x64] [x86]` | `hikasa-<version>-windows-<arch>.msi` | mingw-w64, msitools (`wixl`, `msiinfo`, `msibuild`) |

Requirements in general: a Rust stable toolchain (`rust-version` in `Cargo.toml`), Python 3 and, for Windows,
`rustup target add x86_64-pc-windows-gnu i686-pc-windows-gnu`. Brand files and desktop metadata additionally use
`rsvg-convert`, `magick`, `desktop-file-validate`, `appstreamcli` and `update-mime-database`.

Every package carries `LICENSE-MIT`, `LICENSE-APACHE` and `THIRD_PARTY_LICENSES.txt`, which
`third-party-licenses.py` generates from `cargo metadata` and `cargo tree`: every crate compiled into the two
executables for the package's targets, with the licence files those crates ship, the fonts' licences among them. It
needs nothing but cargo and Python.

### Linux binaries (`build-linux.sh`)

`hikasa` and `hikasa-gui` for the `.tar.gz`, `.deb` and AppImage are the same files:

- **glibc floor.** A binary linked on a rolling distribution needs that distribution's glibc. `zig-cc.sh` links with
  `zig cc -target x86_64-linux-gnu.$HIKASA_GLIBC` instead, against the symbol versions of an older glibc; zig also brings
  the unwinder, so no `libgcc_s` is needed and panics in jobs are still caught. No container, no sysroot.
- **Baseline x86-64.** `RUSTFLAGS` is set with `-C target-cpu=x86-64`, which replaces whatever a cargo configuration says.
- `check-linux-binaries.sh` prints the floor from `objdump -T` and the libraries from `objdump -p`, and rejects a
  binary whose code is full of BMI2 or MOVBE instructions, the mark of a machine-wide `target-cpu`. Dependencies select
  AVX2 code at run time, so a few of them are normal.

The window libraries (EGL, X11, Wayland, xkbcommon, fontconfig) are not linked: winit and wgpu load them at run time.
The `.deb` and the PKGBUILD depend on them by name; the `.tar.gz` and the AppImage take them from the system.

### Arch (`aur/PKGBUILD`, `package-arch.sh`)

`aur/PKGBUILD` is the recipe for the AUR: it builds the tagged source. `package-arch.sh` runs the same recipe on a copy
whose source is a tarball of the working tree, or with `--tag` on the tag itself, and never installs anything. It gives
`makepkg` a configuration of its own, because `makepkg.conf` may carry `target-cpu=native` (CachyOS does, in
`/etc/makepkg.conf.d/rust.conf`). This package is linked against the system's glibc, as Arch packages are. `--nodeps`
is passed so that a `cargo` from rustup is accepted.

### AppImage

The AppImage starts the GUI. The command line is reachable in two ways:

```sh
./Hikasa-<version>-x86_64.AppImage --cli folder compare a b

ln -s Hikasa-<version>-x86_64.AppImage hikasa       # a link named hikasa is the command line
./hikasa version
```

An AppImage mounts itself with FUSE, so it needs `libfuse`/`fusermount`. Without FUSE (containers, some minimal
systems) extract it at each start:

```sh
./Hikasa-<version>-x86_64.AppImage --appimage-extract-and-run [--cli] [arguments]
```

Pins, in `package-appimage.sh`: [appimagetool](https://github.com/AppImage/appimagetool) 1.9.1 and the
[type 2 runtime](https://github.com/AppImage/type2-runtime) 20251108, each with its sha256. appimagetool runs extracted,
so building needs no FUSE.

### .deb

`package-deb.sh` writes the `ar` archive itself, members and headers as `dpkg-deb` does: `debian-binary`, then
`control.tar.xz` (`control`, `md5sums`), then `data.tar.xz`. `check-deb.py` reads the layout back. Debian's triggers
refresh the icon, desktop and MIME caches, so there are no maintainer scripts.

### Windows

`build-windows.sh` uses the mingw GNU targets, which need no proprietary SDK:

- `windows/hikasa.rc.in` becomes one resource object per executable (icon, version information), compiled by `windres`
  and passed with `-C link-arg=`, so the crates need no build script.
- `+crt-static` links libgcc and winpthreads into the executables. The script refuses an executable that needs a
  `lib*.dll` or has the wrong subsystem.
- `hikasa-gui.exe` is a windows-subsystem program in release builds and opens no console; `hikasa-gui --help` therefore
  prints nothing unless redirected, while `hikasa gui open --help` shows the same text.

The portable zip contains `hikasa-portable.toml`: its settings stay in the zip's folder (SES-050). Deleting the file
returns to `%APPDATA%\Hikasa`.

The installer is an MSI compiled by `wixl` from msitools (`windows/hikasa.wxs`, `package-windows-msi.sh`), which runs
natively: no wine, no download. On Arch: `pacman -S msitools`. It has no wizard pages (the choices are msiexec
properties), a Start Menu shortcut, the `.hikasa-pkg` association and the entry in Apps & Features that uninstalls it.
The package is dual-purpose: per user by default, in `%LOCALAPPDATA%\Programs\Hikasa` without elevation; per machine
with `ALLUSERS=1` from an elevated prompt. `ADDTOPATH=1` adds the PATH entry of that scope. Silent use:

```bat
msiexec /i hikasa-<version>-windows-x64.msi /qn ADDTOPATH=1
msiexec /i hikasa-<version>-windows-x64.msi /qn ALLUSERS=1 ADDTOPATH=1
msiexec /x hikasa-<version>-windows-x64.msi /qn
```

`wixl` 0.106 writes the PATH entry without the flag that removes it on uninstall, and a wrong code for the `HKMU` root,
so the script corrects the `Environment` and `Registry` tables with `msibuild` after compiling.

`smoke-test.sh` runs Windows programs under wine on Xvfb, with a private `WINEPREFIX` and `HOME` and with
`winemenubuilder` off, so wine writes neither menu entries nor file associations for the real user. wine's msi ignores
`MSIINSTALLPERUSER`: the per-user branch is tested there with `ALLUSERS=` and lands in Program Files. wine also cannot
compile the shaders of wgpu's DX12 backend, so the window under wine runs with `WGPU_BACKEND=gl`.

## Brand files

`assets/brand/logo-light.svg` and `logo-dark.svg` are written by hand. `generate-brand.sh` makes the rest from the light
logo, and the results stay in git because the build needs them: `hicolor/` (16 to 512 px), `hikasa.ico` and
`window-icon-64.rgba`, which `hikasa-gui` embeds. `generate-brand.sh --preview <file.png>` renders a sheet of sizes.

## Desktop metadata

```sh
desktop-file-validate packaging/hikasa.desktop
appstreamcli validate --no-net --pedantic packaging/io.github.kvasfan.hikasa.metainfo.xml
```

The file name `hikasa.desktop` matches the window's app id (`.with_app_id("hikasa")` in `gui/hikasa-gui/src/main.rs`).
`hikasa-mime.xml` is installed as `share/mime/packages/hikasa.xml` and declares `application/x-hikasa-pkg`.

## Release steps

By hand; nothing here pushes anything.

1. Set the version in `Cargo.toml` (`[workspace.package]` and the internal crates in `[workspace.dependencies]`), add it
   with its date to `<releases>` in `io.github.kvasfan.hikasa.metainfo.xml`, set `pkgver` in `aur/PKGBUILD`, and move the
   CHANGELOG entries.
2. Run the gates of `docs/17`, then `packaging/build-all.sh` and `packaging/smoke-test.sh`.
3. Commit, tag `v<version>`, push the tag.
4. Create the GitHub release and upload everything in `target/dist/`, `SHA256SUMS` included.
5. AUR: run `updpkgsums` in a copy of `aur/PKGBUILD`, check it with `packaging/package-arch.sh --tag`, then push the
   `PKGBUILD` and a fresh `.SRCINFO` (`makepkg --printsrcinfo`) to the AUR repository.
