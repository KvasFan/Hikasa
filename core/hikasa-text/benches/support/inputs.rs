//! Deterministic benchmark inputs (§6.1: seeded xorshift64*, never committed), shared by the benches and by
//! the gate and memory tests (`#[path]` include): two C-like sources of about 48 bytes per line, the right one
//! a copy of the left one with about 1 % of the lines changed, in clusters.
#![allow(dead_code)]

/// Seeded xorshift64*.
pub struct XorShift(pub u64);

impl XorShift {
    pub fn next(&mut self) -> u64 {
        let mut x = self.0;
        x ^= x >> 12;
        x ^= x << 25;
        x ^= x >> 27;
        self.0 = x;
        x.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }

    pub fn below(&mut self, n: u64) -> u64 {
        self.next() % n.max(1)
    }
}

/// Identifiers used by the generated statements.
const NAMES: [&str; 16] = [
    "count", "value", "index", "buffer", "length", "result", "offset", "status", "handle",
    "cursor", "total", "limit", "flags", "state", "token", "node",
];

/// One C-like line (≈ 48 bytes on average): statements, calls, declarations, comments, braces, blanks.
pub fn c_line(rng: &mut XorShift, k: u64) -> String {
    let a = NAMES[rng.below(16) as usize];
    let b = NAMES[rng.below(16) as usize];
    let n = rng.below(100_000);
    match rng.below(20) {
        0 => String::new(),
        1 => "    }".to_owned(),
        2 => "    {".to_owned(),
        3 => format!("static int {a}_{k}(struct {b} *p, int n) /* {n} */"),
        4 => format!("    /* update the {a} of the {b} ({n}) */"),
        5 => format!("    if ({a}_{k} > {n}) {{ return {b}; }}"),
        6 => format!("    for (int i = 0; i < {a}; i++) {b}[i] = {n};"),
        7 => format!("    {a} = {b} + 0x{n:X}; // next step {k}"),
        8 => format!("    printf(\"%d {a}\\n\", {b}_{k});"),
        _ => format!("    int {a}_{k} = compute_next_{b}({n}, {a});"),
    }
}

/// `lines` lines on each side (≈ 48 bytes each, LF endings): the right side copies the left one with change
/// clusters of 1–10 lines starting at about 1 line in 400 (edits, insertions and deletions in equal shares),
/// so about 1 % of the lines change.
pub fn c_pair(lines: usize, seed: u64) -> (Vec<u8>, Vec<u8>) {
    let mut rng = XorShift(seed);
    let mut left = String::with_capacity(lines * 50);
    let mut right = String::with_capacity(lines * 50);
    let mut k = 0usize;
    while k < lines {
        let line = c_line(&mut rng, k as u64);
        left.push_str(&line);
        left.push('\n');
        if rng.below(400) == 0 {
            let cluster = 1 + rng.below(10) as usize;
            match rng.below(3) {
                0 => {
                    // Edit this line and the next ones.
                    right.push_str(&line.replace(' ', "  "));
                    right.push('\n');
                    for j in 1..cluster {
                        let next = c_line(&mut rng, (k + j) as u64);
                        left.push_str(&next);
                        left.push('\n');
                        right.push_str(&format!("{next} /* edited */\n"));
                    }
                    k += cluster;
                    continue;
                }
                1 => {
                    // Insert lines on the right.
                    right.push_str(&line);
                    right.push('\n');
                    for j in 0..cluster {
                        right.push_str(&c_line(&mut rng, (lines + k + j) as u64));
                        right.push('\n');
                    }
                }
                _ => {
                    // Delete this line on the right.
                }
            }
        } else {
            right.push_str(&line);
            right.push('\n');
        }
        k += 1;
    }
    (left.into_bytes(), right.into_bytes())
}

/// Number of lines of `bytes` (LF only).
pub fn line_count(bytes: &[u8]) -> usize {
    bytes.iter().filter(|&&b| b == b'\n').count()
}
