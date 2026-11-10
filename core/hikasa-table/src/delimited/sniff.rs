//! Sniffing (contract §2.3, T-18): delimiter, qualifier, or fixed widths.

use crate::format::{DelimitedOptions, FixedOptions, TableKind};
use crate::regional::{DateOrder, Regional};

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

/// True when `s` is one date-time value in any date order (`2024-01-02 12:00:00`, `01/02/2024 1:05 PM`, `12:30`)
/// without a strong delimiter.
fn is_one_datetime(s: &str) -> bool {
    let s = s.trim();
    // A comma or another strong delimiter in the record is a field separator, not a fraction of seconds.
    if s.contains([',', ';', '\t', '|']) {
        return false;
    }
    [DateOrder::Ymd, DateOrder::Dmy, DateOrder::Mdy]
        .into_iter()
        .any(|date_order| {
            Regional {
                date_order,
                ..Regional::default()
            }
            .parse_datetime(s)
            .is_some()
        })
}

/// True when every `:` of the record outside qualifiers separates the parts of a time (a digit before it, exactly
/// two digits after it).
fn colons_are_times(record: &str) -> bool {
    let b = record.as_bytes();
    let mut quoted = false;
    let mut any = false;
    for (i, &c) in b.iter().enumerate() {
        if c == b'"' {
            quoted = !quoted;
        }
        if c != b':' || quoted {
            continue;
        }
        any = true;
        let before = i > 0 && b[i - 1].is_ascii_digit();
        let two = b.get(i + 1).is_some_and(u8::is_ascii_digit)
            && b.get(i + 2).is_some_and(u8::is_ascii_digit)
            && !b.get(i + 3).is_some_and(u8::is_ascii_digit);
        if !(before && two) {
            return false;
        }
    }
    any
}

/// Sniffs the type of a text table (T-18, F7): a candidate must occur in the first record; `:` is no candidate
/// when it separates the parts of times; records that are each one date-time value (the first one aside, a
/// header) are one column.
pub(crate) fn sniff(text: &str) -> TableKind {
    let records = sample_records(text);
    if records.is_empty() {
        return TableKind::Delimited(DelimitedOptions::with(','));
    }
    let single = TableKind::Delimited(DelimitedOptions {
        delimiters: vec!['\0'],
        qualifier: None,
        consecutive_as_one: false,
        trim_whitespace: false,
    });
    let body = if records.len() > 1 {
        &records[1..]
    } else {
        &records[..]
    };
    if body.iter().filter(|r| is_one_datetime(r)).count() * 5 >= body.len() * 4 {
        return single;
    }
    let need = if records.len() == 1 {
        1
    } else {
        (records.len() * 4).div_ceil(5)
    };
    let with_colon: Vec<&&str> = records.iter().filter(|r| r.contains(':')).collect();
    let colon_times = !with_colon.is_empty()
        && with_colon.iter().filter(|r| colons_are_times(r)).count() * 5 >= with_colon.len() * 4;
    let mut best: Option<(usize, char)> = None;
    for d in CANDIDATES {
        if d == ':' && colon_times {
            continue;
        }
        let counts: Vec<usize> = records.iter().map(|r| count_outside(r, d)).collect();
        if counts[0] == 0 {
            continue;
        }
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
    single
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
        // Ties go to the earlier candidate.
        assert_eq!(delim(sniff("a,b;c\n1,2;3\n")), (',', Some('"')));
        // A candidate must occur in the first record; `:` between the parts of a time is none.
        assert_eq!(delim(sniff("time\n12:00:00\n12:01:00\n")), ('\0', None));
        assert_eq!(delim(sniff("a:b:c\n1:2:3\n4:5:6\n")), (':', Some('"')));
        assert_eq!(delim(sniff("key:value\n7:120\n8:130\n")), (':', Some('"')));
        // A title line without the delimiter: one column, not the comma of the records below it.
        assert_eq!(
            delim(sniff("title\na,b\nc,d\ne,f\ng,h\ni,j\n")),
            ('\0', None)
        );
        match sniff("id   name    qty\n1    apple   10\n22   pear    3\n") {
            TableKind::Fixed(f) => assert_eq!(f.widths, vec![5, 8]),
            other => panic!("{other:?}"),
        }
    }
}
