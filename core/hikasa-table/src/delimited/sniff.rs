//! Sniffing (contract §2.3, T-18): delimiter, qualifier, or fixed widths.

use crate::format::{DelimitedOptions, FixedOptions, TableKind};

/// Candidate delimiters in tie order.
const CANDIDATES: [char; 6] = [',', ';', '\t', '|', ':', ' '];
/// Records sampled.
const SAMPLE: usize = 64;

/// Splits the first records quote-aware (qualifier `"`), returning each record's text.
fn sample_records(text: &str) -> Vec<&str> {
    let mut out = Vec::new();
    let bytes = text.as_bytes();
    let mut start = 0;
    let mut quoted = false;
    let mut i = 0;
    while i < bytes.len() && out.len() < SAMPLE {
        match bytes[i] {
            b'"' => quoted = !quoted,
            b'\n' | b'\r' if !quoted => {
                out.push(&text[start..i]);
                if bytes[i] == b'\r' && bytes.get(i + 1) == Some(&b'\n') {
                    i += 1;
                }
                start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if out.len() < SAMPLE && start < bytes.len() {
        out.push(&text[start..]);
    }
    out.retain(|r| !r.trim().is_empty());
    out
}

/// Occurrences of `d` outside `"` qualifiers.
fn count_outside(record: &str, d: char) -> usize {
    let mut quoted = false;
    let mut n = 0;
    for c in record.chars() {
        if c == '"' {
            quoted = !quoted;
        } else if c == d && !quoted {
            n += 1;
        }
    }
    n
}

/// Sniffs the type of a text table.
pub(crate) fn sniff(text: &str) -> TableKind {
    let records = sample_records(text);
    if records.is_empty() {
        return TableKind::Delimited(DelimitedOptions::with(','));
    }
    let need = if records.len() == 1 {
        1
    } else {
        (records.len() * 4).div_ceil(5)
    };
    let mut best: Option<(usize, char)> = None;
    for d in CANDIDATES {
        let counts: Vec<usize> = records.iter().map(|r| count_outside(r, d)).collect();
        // Modal non-zero count.
        let mut sorted: Vec<usize> = counts.iter().copied().filter(|&c| c > 0).collect();
        if sorted.is_empty() {
            continue;
        }
        sorted.sort_unstable();
        let mut modal = (0usize, 0usize);
        let mut i = 0;
        while i < sorted.len() {
            let j = sorted[i..].iter().take_while(|&&x| x == sorted[i]).count();
            if j > modal.1 {
                modal = (sorted[i], j);
            }
            i += j;
        }
        let score = counts.iter().filter(|&&c| c == modal.0).count();
        // Space is a weak candidate: aligned text has runs of spaces, which fixed widths handle better.
        if d == ' ' && records.len() > 1 && modal.0 > 0 && fixed_widths(&records).is_some() {
            continue;
        }
        if score >= need && best.is_none_or(|(s, _)| score > s) {
            best = Some((score, d));
        }
    }
    if let Some((_, d)) = best {
        let mut opts = DelimitedOptions::with(d);
        let fields: Vec<&str> = records
            .iter()
            .flat_map(|r| r.split(d))
            .map(str::trim_start)
            .collect();
        let single = fields.iter().filter(|f| f.starts_with('\'')).count();
        let double = fields.iter().filter(|f| f.starts_with('"')).count();
        if double == 0 && single * 2 >= fields.len() && single > 0 {
            opts.qualifier = Some('\'');
        }
        if d == ' ' {
            opts.consecutive_as_one = true;
        }
        return TableKind::Delimited(opts);
    }
    if records.len() >= 2
        && let Some(widths) = fixed_widths(&records)
    {
        return TableKind::Fixed(FixedOptions { widths });
    }
    TableKind::Delimited(DelimitedOptions {
        delimiters: vec!['\0'],
        qualifier: None,
        consecutive_as_one: false,
        trim_whitespace: false,
    })
}

/// Column boundaries where every non-empty sample line is blank and some line has a non-blank just before.
fn fixed_widths(records: &[&str]) -> Option<Vec<u32>> {
    let lines: Vec<Vec<char>> = records.iter().map(|r| r.chars().collect()).collect();
    let width = lines.iter().map(Vec::len).max()?;
    let blank_at = |p: usize| lines.iter().all(|l| l.get(p).is_none_or(|c| *c == ' '));
    let mut bounds = Vec::new();
    let mut p = 1;
    while p < width {
        if blank_at(p) && !blank_at(p - 1) {
            // Skip the run of blank columns; the boundary is its end.
            let mut q = p;
            while q < width && blank_at(q) {
                q += 1;
            }
            if q < width {
                bounds.push(q);
            }
            p = q;
        } else {
            p += 1;
        }
    }
    if bounds.is_empty() {
        return None;
    }
    let mut widths = Vec::with_capacity(bounds.len());
    let mut prev = 0;
    for b in bounds {
        widths.push((b - prev) as u32);
        prev = b;
    }
    Some(widths)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn delim(kind: TableKind) -> (char, Option<char>) {
        match kind {
            TableKind::Delimited(o) => (o.delimiters[0], o.qualifier),
            other => panic!("{other:?}"),
        }
    }

    #[test]
    fn sniffs_delimiters_and_fixed_widths() {
        assert_eq!(delim(sniff("a,b,c\n1,2,3\n")), (',', Some('"')));
        assert_eq!(delim(sniff("a;b;c\n1,5;2,5;3\n")), (';', Some('"')));
        assert_eq!(delim(sniff("a\tb\n1\t2\n")), ('\t', Some('"')));
        assert_eq!(delim(sniff("a|b\n1|2\n3|4\n")), ('|', Some('"')));
        assert_eq!(delim(sniff("\"x,y\";b\n\"1,2\";3\n")), (';', Some('"')));
        assert_eq!(delim(sniff("'a';'b'\n'1';'2'\n")), (';', Some('\'')));
        assert_eq!(delim(sniff("one\ntwo\n")), ('\0', None));
        match sniff("id   name    qty\n1    apple   10\n22   pear    3\n") {
            TableKind::Fixed(f) => assert_eq!(f.widths, vec![5, 8]),
            other => panic!("{other:?}"),
        }
    }
}
