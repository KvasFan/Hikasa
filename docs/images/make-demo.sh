#!/usr/bin/env bash
# The demo data behind the screenshots of the README: two versions of "quaywatch", a tide gauge logger made up for
# this page, and a few sessions over them.
#
#   make-demo.sh tree <dir>   writes <dir>/left and <dir>/right
#   make-demo.sh sessions     creates the sessions (run it where the tree is /demo, see below)
#   make-demo.sh shoot <name> saves the display as <name>.png, without metadata, at full size
#
# How the shots were made, all with a throwaway home, never on a real display:
#
#   env -u WAYLAND_DISPLAY xvfb-run -a -s "-screen 0 1600x1000x24" bash   # with HOME and XDG_* in a scratch folder
#   bwrap --ro-bind /usr /usr --ro-bind /etc /etc --proc /proc --dev /dev --tmpfs /tmp --tmpfs /var \
#         --symlink usr/bin /bin --symlink usr/lib /lib --symlink usr/lib /lib64 --ro-bind /tmp/.X11-unix /tmp/.X11-unix \
#         --bind <dir> /demo --bind <scratch>/home /home/demo --setenv HOME /home/demo --chdir /demo \
#         --setenv WINIT_X11_SCALE_FACTOR 1.25 hikasa-gui        # so that the paths read /demo/left and /demo/right
#
# with a window of 1280 x 800 logical pixels ([window] in state.toml, started once before so that the first-run
# notices are gone), `hikasa gui session <name>`, `hikasa gui open /demo/left /demo/right --expandall`, xdotool for
# the menus and dialogs, and `hikasa settings set options.appearance.theme_mode dark` for the dark view. The settings
# package of the Import wizard comes from `hikasa settings export -o /demo/harbour.hikasa-pkg`.
set -eu

tree() {
    local out=$1 l=$1/left r=$1/right
    rm -rf "$out"
    mkdir -p "$l/src" "$l/tests/fixtures" "$l/config" "$l/scripts" "$l/docs"

    cat >"$l/Cargo.toml" <<'E'
[package]
name = "quaywatch"
version = "1.4.2"
edition = "2021"

[dependencies]
chrono = "0.4"
serde = { version = "1", features = ["derive"] }
toml = "0.8"
E
    cat >"$l/README.md" <<'E'
# quaywatch

Reads tide gauges over a serial line and writes one log file per station.

    quaywatch --config config/default.ini

Readings older than the retention period are dropped at start.
E
    cat >"$l/config/default.ini" <<'E'
[station]
name=North Pier
port=/dev/ttyS0
baud=9600

[log]
folder=/srv/quaywatch
retention_days=30
E
    cat >"$l/scripts/deploy.sh" <<'E'
#!/bin/sh
set -e
cargo build --release
install -m 755 target/release/quaywatch /srv/quaywatch/bin/
systemctl restart quaywatch
E
    cat >"$l/src/main.rs" <<'E'
mod config;
mod legacy_csv;
mod report;
mod sensor;

fn main() {
    let config = config::load("config/default.ini").expect("config");
    let mut gauge = sensor::Gauge::open(&config.port, config.baud).expect("serial port");
    loop {
        match gauge.read() {
            Ok(reading) => report::append(&config.folder, &reading).expect("log"),
            Err(error) => eprintln!("gauge: {error}"),
        }
    }
}
E
    cat >"$l/src/config.rs" <<'E'
use std::path::PathBuf;

pub struct Config {
    pub port: String,
    pub baud: u32,
    pub folder: PathBuf,
    pub retention_days: u32,
}

pub fn load(path: &str) -> Result<Config, String> {
    let text = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
    parse(&text)
}
E
    cat >"$l/src/report.rs" <<'E'
use crate::sensor::Reading;
use std::{fs::OpenOptions, io::Write, path::Path};

pub fn append(folder: &Path, reading: &Reading) -> std::io::Result<()> {
    let name = reading.time.format("%Y-%m-%d.log").to_string();
    let mut file = OpenOptions::new().create(true).append(true).open(folder.join(name))?;
    writeln!(file, "{} {:.3}", reading.time, reading.level_m)
}
E
    cat >"$l/src/legacy_csv.rs" <<'E'
//! The CSV layout of the first gauges. Kept until the last station moves on.
use crate::sensor::Reading;

pub fn line(reading: &Reading) -> String {
    format!("{},{:.2}", reading.time.timestamp(), reading.level_m)
}
E
    cat >"$l/src/sensor.rs" <<'E'
//! The tide gauge sends one line per sample: `$TIDE,<raw>,<temp>*<sum>`.

use chrono::{DateTime, Utc};
use std::io::{self, BufRead, BufReader};
use std::time::Duration;

/// Raw counts per metre of the pressure sensor.
const COUNTS_PER_METRE: f64 = 1024.0;
/// How long a read waits for the next line.
const READ_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Debug, Clone)]
pub struct Reading {
    pub time: DateTime<Utc>,
    pub level_m: f64,
    pub water_temp_c: f32,
}

pub struct Gauge {
    port: BufReader<Box<dyn serialport::SerialPort>>,
    line: String,
}

impl Gauge {
    /// Opens the serial port of the gauge.
    pub fn open(path: &str, baud: u32) -> io::Result<Gauge> {
        let port = serialport::new(path, baud).timeout(READ_TIMEOUT).open()?;
        let port = BufReader::new(port);
        Ok(Gauge { port, line: String::new() })
    }

    /// Reads the next sample. A line that does not parse is an error.
    pub fn read(&mut self) -> io::Result<Reading> {
        self.line.clear();
        self.port.read_line(&mut self.line)?;
        parse(self.line.trim_end())
            .ok_or_else(|| io::ErrorKind::InvalidData.into())
    }
}

fn parse(line: &str) -> Option<Reading> {
    let body = line.strip_prefix("$TIDE,")?;
    let (fields, _checksum) = body.split_once('*')?;
    let (raw, temperature) = fields.split_once(',')?;
    let raw: f64 = raw.parse().ok()?;
    let level_m = raw / COUNTS_PER_METRE;
    let water_temp_c = temperature.parse().ok()?;
    Some(Reading { time: Utc::now(), level_m, water_temp_c })
}

#[cfg(test)]
mod tests {
    use super::parse;

    #[test]
    fn reads_a_sample() {
        let reading = parse("$TIDE,2048,11.5*4F").unwrap();
        assert_eq!(reading.level_m, 2.0);
    }
}
E
    cat >"$l/tests/gauge.rs" <<'E'
use quaywatch::sensor::parse;

#[test]
fn rejects_a_short_line() {
    assert!(parse("$TIDE,2048").is_none());
}
E
    printf 'time,level_m\n1700000000,2.31\n1700000600,2.35\n' >"$l/tests/fixtures/north_pier.csv"
    printf '# Setup\n\nConnect the gauge to the first serial port and start the service.\n' >"$l/docs/setup.md"
    printf '# Wiring\n\nThe gauge sends RS-232 at the speed set in the configuration.\n' >"$l/docs/wiring.md"
    printf '# Protocol\n\nOne line per sample, ended by a checksum.\n' >"$l/docs/protocol.md"
    printf '/target\n*.log\n' >"$l/.gitignore"
    printf 'MIT\n' >"$l/LICENSE"
    find "$l" -exec touch -d '2026-03-02 09:30' {} +

    cp -a "$l" "$r"
    sed -i 's/version = "1.4.2"/version = "1.5.0"/; /^toml/a serialport = "4"' "$r/Cargo.toml"
    sed -i 's/Readings older than the retention period are dropped at start./Readings older than the retention period are dropped every night.\nA level above `alarm_level_m` sends a notice to the harbour office./' "$r/README.md"
    sed -i 's/retention_days=30/retention_days=90\nalarm_level_m=3.20/; s/baud=9600/baud=19200/' "$r/config/default.ini"
    sed -i 's/^systemctl restart quaywatch/systemctl daemon-reload\nsystemctl restart quaywatch.service/' "$r/scripts/deploy.sh"
    sed -i 's/^mod legacy_csv;/mod alerts;/; s/report::append(&config.folder, &reading).expect("log"),/{\n                report::append(\&config.folder, \&reading).expect("log");\n                alerts::check(\&config, \&reading);\n            }/' "$r/src/main.rs"
    sed -i 's/    pub retention_days: u32,/    pub retention_days: u32,\n    pub alarm_level_m: f64,/' "$r/src/config.rs"
    rm "$r/src/legacy_csv.rs"
    cat >"$r/src/alerts.rs" <<'E'
use crate::{config::Config, sensor::Reading};

pub fn check(config: &Config, reading: &Reading) {
    if reading.level_m > config.alarm_level_m {
        eprintln!("high water: {:.2} m", reading.level_m);
    }
}
E
    sed -i -e 's/^const COUNTS_PER_METRE: f64 = 1024.0;/const COUNTS_PER_METRE: f64 = 1000.0;/' \
        -e 's/from_secs(5)/from_secs(2)/' \
        -e '/^const READ_TIMEOUT/a /// Samples below this level are taken as a sensor fault.\nconst MIN_LEVEL_M: f64 = -1.5;' \
        -e 's/^    pub water_temp_c: f32,/    pub water_temp_c: Option<f32>,\n    pub battery_v: Option<f32>,/' \
        -e 's/A line that does not parse is an error./A bad line or level is an error./' \
        -e '/let (fields, _checksum) = body.split_once/c\    let (fields, checksum) = body.split_once('"'"'*'"'"')?;\n    if u8::from_str_radix(checksum, 16).ok()? != xor_of(fields) {\n        return None;\n    }' \
        -e '/let (raw, temperature) = fields.split_once/c\    let mut fields = fields.split('"'"','"'"');' \
        -e 's/let raw: f64 = raw.parse().ok()?;/let raw: f64 = fields.next()?.parse().ok()?;/' \
        -e '/let level_m = raw/a\    if level_m < MIN_LEVEL_M {\n        return None;\n    }' \
        -e 's/let water_temp_c = temperature.parse().ok()?;/let water_temp_c = fields.next().and_then(|t| t.parse().ok());\n    let battery_v = fields.next().and_then(|v| v.parse().ok());/' \
        -e 's/Some(Reading { time: Utc::now(), level_m, water_temp_c })/Some(Reading { time: Utc::now(), level_m, water_temp_c, battery_v })/' \
        -e '/^#\[cfg(test)\]/i fn xor_of(text: \&str) -> u8 {\n    text.bytes().fold(0, |sum, byte| sum ^ byte)\n}\n' \
        -e 's/"$TIDE,2048,11.5\*4F"/"$TIDE,2000,11.5,3.7*33"/' \
        "$r/src/sensor.rs"
    sed -i 's/fn rejects_a_short_line() {/fn rejects_a_bad_checksum() {/; s/parse("$TIDE,2048")/parse("$TIDE,2048,11.5*00")/' "$r/tests/gauge.rs"
    printf '1700001200,2.42\n' >>"$r/tests/fixtures/north_pier.csv"
    printf 'time,level_m\n1700000000,1.87\n' >"$r/tests/fixtures/south_quay.csv"
    find "$r" -newermt '2026-03-02 10:00' -exec touch -d '2026-05-18 16:45' {} +
}

sessions() {
    local q="Harbour tools/quaywatch" n="Field notes"
    hikasa session mkdir "Harbour tools"
    hikasa session mkdir "$q"
    hikasa session mkdir "$n"
    pair() { hikasa session new --type "$1" --left "/demo/left${2:+/$2}" --right "/demo/right${2:+/$2}" --description "$3" "$4"; }
    pair folder-compare "" "Release 1.4 against 1.5" "$q/Release review"
    pair text-compare src/sensor.rs "Checksum and battery fields" "$q/Gauge parser"
    pair text-compare src/main.rs "Alert hook in the main loop" "$q/Main loop"
    pair text-compare config/default.ini "Settings of the station" "$q/Station config"
    pair folder-compare tests "Fixtures of the two stations" "$q/Test fixtures"
    pair folder-compare docs "" "$n/Setup guide"
    pair text-compare README.md "" "$n/Readme wording"
}

shoot() {
    import -window root "$1.raw.png"
    magick "$1.raw.png" -strip -define png:compression-level=9 -define png:compression-filter=5 \
        -define png:compression-strategy=1 "$1.png"
    rm "$1.raw.png"
}

"$@"
