//! Position-exact, byte-exact patch appliers for the tests (P-RPT-1 and the patch application tests): each hunk
//! is applied exactly at the position its header gives, its pre-image must equal the bytes there (terminators
//! included; `\ No newline at end of file` removes the newline of the line before it), and anything malformed is an
//! error. Unlike a fuzzy applier (diffy places a zero-context hunk by searching, and can pick the wrong one of
//! two duplicate lines), a wrong position or a wrong line can never pass.

/// An applier: the left file and the patch in, the patched file (or why the patch does not apply) out.
pub type Applier = fn(&[u8], &[u8]) -> Result<Vec<u8>, String>;

/// The lines of `bytes`, each with its `\n` (the last one may lack it). A lone CR does not end a line here (the
/// tests normalise CR-only files first).
pub fn split(bytes: &[u8]) -> Vec<Vec<u8>> {
    bytes
        .split_inclusive(|&b| b == b'\n')
        .map(<[u8]>::to_vec)
        .collect()
}

/// The patch body: the lines after the `# ` header comment lines.
fn body(patch: &[u8]) -> Vec<Vec<u8>> {
    split(patch)
        .into_iter()
        .skip_while(|l| l.first() == Some(&b'#'))
        .collect()
}

/// `n` or `a,b` as numbers.
fn numbers(s: &str) -> Result<(u64, Option<u64>), String> {
    let num = |t: &str| {
        t.parse::<u64>()
            .map_err(|e| format!("bad number {t:?}: {e}"))
    };
    match s.split_once(',') {
        Some((a, b)) => Ok((num(a)?, Some(num(b)?))),
        None => Ok((num(s)?, None)),
    }
}

/// One patch line's text after its `marker_len`-byte marker; a following `\ No newline at end of file` drops the
/// final `\n`.
fn take(lines: &[Vec<u8>], i: &mut usize, marker_len: usize) -> Result<Vec<u8>, String> {
    let line = lines.get(*i).ok_or("unexpected end of patch")?;
    let mut text = line
        .get(marker_len..)
        .ok_or_else(|| format!("short line {:?}", String::from_utf8_lossy(line)))?
        .to_vec();
    if text.last() != Some(&b'\n') {
        return Err(format!(
            "patch line without newline: {:?}",
            String::from_utf8_lossy(line)
        ));
    }
    *i += 1;
    if lines
        .get(*i)
        .is_some_and(|l| l.starts_with(b"\\ No newline at end of file"))
    {
        text.pop();
        *i += 1;
    }
    Ok(text)
}

/// Replaces `old.len()` lines of `image` at `at` (0-based) by `new`, checking the pre-image.
fn splice(
    image: &mut Vec<Vec<u8>>,
    at: usize,
    old: &[Vec<u8>],
    new: Vec<Vec<u8>>,
) -> Result<(), String> {
    let found = image.get(at..at + old.len()).ok_or_else(|| {
        format!(
            "hunk at line {} beyond the end ({} lines)",
            at + 1,
            image.len()
        )
    })?;
    if found != old {
        return Err(format!(
            "pre-image mismatch at line {}: want {:?}, found {:?}",
            at + 1,
            old.iter()
                .map(|l| String::from_utf8_lossy(l))
                .collect::<Vec<_>>(),
            found
                .iter()
                .map(|l| String::from_utf8_lossy(l))
                .collect::<Vec<_>>()
        ));
    }
    image.splice(at..at + old.len(), new);
    Ok(())
}

/// Applies a unified diff (plain or annotated) to `left`.
pub fn unified(left: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    let lines = body(patch);
    let mut image = split(left);
    if lines.is_empty() {
        return Ok(left.to_vec());
    }
    if !(lines[0].starts_with(b"--- ") && lines.get(1).is_some_and(|l| l.starts_with(b"+++ "))) {
        return Err("missing file headers".into());
    }
    let mut i = 2;
    // Lines added or removed by the hunks before the current one.
    let mut delta: i64 = 0;
    while i < lines.len() {
        let header = String::from_utf8_lossy(&lines[i]).into_owned();
        i += 1;
        let ranges = header
            .strip_prefix("@@ -")
            .and_then(|h| h.split_once(" @@"))
            .map(|(r, _)| r)
            .ok_or_else(|| format!("bad hunk header {header:?}"))?;
        let (old_r, new_r) = ranges.split_once(" +").ok_or("bad hunk ranges")?;
        let (old_start, old_len) = numbers(old_r)?;
        let (new_start, new_len) = numbers(new_r)?;
        let (old_len, new_len) = (old_len.unwrap_or(1), new_len.unwrap_or(1));
        let (mut old, mut new) = (Vec::new(), Vec::new());
        while i < lines.len() && !lines[i].starts_with(b"@@ ") {
            match lines[i].first() {
                Some(b' ') => {
                    let t = take(&lines, &mut i, 1)?;
                    old.push(t.clone());
                    new.push(t);
                }
                Some(b'-') => old.push(take(&lines, &mut i, 1)?),
                Some(b'+') => new.push(take(&lines, &mut i, 1)?),
                _ => {
                    return Err(format!(
                        "bad hunk line {:?}",
                        String::from_utf8_lossy(&lines[i])
                    ));
                }
            }
        }
        if old.len() as u64 != old_len || new.len() as u64 != new_len {
            return Err(format!(
                "hunk {header:?} has {} old and {} new lines",
                old.len(),
                new.len()
            ));
        }
        // An empty range names the line before it.
        let old_at = if old_len == 0 {
            old_start
        } else {
            old_start - 1
        };
        let new_at = if new_len == 0 {
            new_start
        } else {
            new_start - 1
        };
        let at = (old_at as i64 + delta) as usize;
        if at as u64 != new_at {
            return Err(format!(
                "hunk {header:?}: new start disagrees with the old one"
            ));
        }
        delta += new.len() as i64 - old.len() as i64;
        splice(&mut image, at, &old, new)?;
    }
    Ok(image.concat())
}

/// A context-diff range: `(first line or insertion point, line count)`, 0-based start of the lines it covers.
fn context_range(s: &str) -> Result<(usize, usize), String> {
    Ok(match numbers(s)? {
        (0, None) => (0, 0),
        (l, None) => (l as usize - 1, 1),
        // `l+1,l` is the empty range after line `l`.
        (f, Some(l)) if l + 1 == f => (l as usize, 0),
        (f, Some(l)) if l >= f && f >= 1 => (f as usize - 1, (l - f + 1) as usize),
        other => return Err(format!("bad context range {other:?}")),
    })
}

/// Applies a context diff to `left`: the pre-image is the old part, or, when it is omitted, the new part's
/// context lines; the post-image the new part, or the old part's context lines.
pub fn context(left: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    let lines = body(patch);
    let mut image = split(left);
    if lines.is_empty() {
        return Ok(left.to_vec());
    }
    if !(lines[0].starts_with(b"*** ") && lines.get(1).is_some_and(|l| l.starts_with(b"--- "))) {
        return Err("missing file headers".into());
    }
    let mut i = 2;
    let mut delta: i64 = 0;
    let part = |line: &[u8], open: &str, close: &str| -> Option<String> {
        let s = String::from_utf8_lossy(line);
        s.trim_end_matches('\n')
            .strip_prefix(open)
            .and_then(|r| r.strip_suffix(close))
            .map(str::to_owned)
    };
    while i < lines.len() {
        if lines[i] != b"***************\n" {
            return Err(format!(
                "bad separator {:?}",
                String::from_utf8_lossy(&lines[i])
            ));
        }
        i += 1;
        let old_r = part(&lines[i], "*** ", " ****").ok_or("bad old range")?;
        let (old_at, old_len) = context_range(&old_r)?;
        i += 1;
        let mut old: Vec<(u8, Vec<u8>)> = Vec::new();
        while i < lines.len() && part(&lines[i], "--- ", " ----").is_none() {
            let m = lines[i][0];
            old.push((m, take(&lines, &mut i, 2)?));
        }
        let new_r = part(lines.get(i).ok_or("missing new part")?, "--- ", " ----")
            .ok_or("bad new range")?;
        let (new_at, new_len) = context_range(&new_r)?;
        i += 1;
        let mut new: Vec<(u8, Vec<u8>)> = Vec::new();
        while i < lines.len() && lines[i] != b"***************\n" {
            let m = lines[i][0];
            new.push((m, take(&lines, &mut i, 2)?));
        }
        let context = |p: &[(u8, Vec<u8>)]| -> Vec<Vec<u8>> {
            p.iter()
                .filter(|(m, _)| *m == b' ')
                .map(|(_, l)| l.clone())
                .collect()
        };
        for (m, _) in &old {
            if !matches!(m, b' ' | b'-' | b'!') {
                return Err(format!("bad old marker {:?}", *m as char));
            }
        }
        for (m, _) in &new {
            if !matches!(m, b' ' | b'+' | b'!') {
                return Err(format!("bad new marker {:?}", *m as char));
            }
        }
        let pre = if old.is_empty() {
            context(&new)
        } else {
            old.iter().map(|(_, l)| l.clone()).collect()
        };
        let post = if new.is_empty() {
            context(&old)
        } else {
            new.iter().map(|(_, l)| l.clone()).collect()
        };
        if pre.len() != old_len || post.len() != new_len {
            return Err(format!(
                "hunk *** {old_r} / --- {new_r}: {} old and {} new lines",
                pre.len(),
                post.len()
            ));
        }
        let at = (old_at as i64 + delta) as usize;
        if at != new_at {
            return Err(format!(
                "hunk *** {old_r} / --- {new_r}: positions disagree"
            ));
        }
        delta += post.len() as i64 - pre.len() as i64;
        splice(&mut image, at, &pre, post)?;
    }
    Ok(image.concat())
}

/// Applies a normal diff (`NaR`, `N,MdR`, `N,McR,S` commands) to `left`.
pub fn normal(left: &[u8], patch: &[u8]) -> Result<Vec<u8>, String> {
    let lines = body(patch);
    let mut image = split(left);
    let mut i = 0usize;
    let mut delta: i64 = 0;
    while i < lines.len() {
        let cmd = String::from_utf8_lossy(&lines[i]).trim_end().to_owned();
        i += 1;
        let pos = cmd
            .find(['a', 'd', 'c'])
            .ok_or_else(|| format!("bad command {cmd:?}"))?;
        let (l, op, r) = (
            numbers(&cmd[..pos])?,
            &cmd[pos..=pos],
            numbers(&cmd[pos + 1..])?,
        );
        let count = |(a, b): (u64, Option<u64>)| b.map_or(1, |b| (b - a + 1) as usize);
        let mut old = Vec::new();
        while i < lines.len() && lines[i].starts_with(b"< ") {
            old.push(take(&lines, &mut i, 2)?);
        }
        if op == "c" {
            if lines.get(i).map(Vec::as_slice) != Some(b"---\n".as_slice()) {
                return Err(format!("{cmd}: missing ---"));
            }
            i += 1;
        }
        let mut new = Vec::new();
        while i < lines.len() && lines[i].starts_with(b"> ") {
            new.push(take(&lines, &mut i, 2)?);
        }
        let (old_at, old_n) = match op {
            "a" => (l.0 as usize, 0),
            _ => (l.0 as usize - 1, count(l)),
        };
        let (new_at, new_n) = match op {
            "d" => (r.0 as usize, 0),
            _ => (r.0 as usize - 1, count(r)),
        };
        if old.len() != old_n || new.len() != new_n {
            return Err(format!(
                "{cmd}: {} old and {} new lines",
                old.len(),
                new.len()
            ));
        }
        let at = (old_at as i64 + delta) as usize;
        if at != new_at {
            return Err(format!("{cmd}: positions disagree"));
        }
        delta += new.len() as i64 - old.len() as i64;
        splice(&mut image, at, &old, new)?;
    }
    Ok(image.concat())
}

/// Every `\ No newline at end of file` follows the last line of its side: it ends its hunk, or (unified) it
/// follows the old side's last `-` line and only `+` lines follow it in the hunk, or (context) it ends the old
/// part, or (normal) it ends the `< ` lines of a change.
pub fn no_newline_markers_are_last(patch: &[u8]) -> Result<(), String> {
    let lines = body(patch);
    let unified = lines.first().is_some_and(|l| l.starts_with(b"--- "));
    let context = lines.first().is_some_and(|l| l.starts_with(b"*** "));
    let hunk_start = |l: &[u8]| {
        l.starts_with(b"@@ ")
            || l.starts_with(b"***************")
            || (!unified && !context && l.first().is_some_and(u8::is_ascii_digit))
    };
    for (k, line) in lines.iter().enumerate() {
        if !line.starts_with(b"\\ No newline at end of file") {
            continue;
        }
        let prev = k
            .checked_sub(1)
            .and_then(|p| lines.get(p))
            .ok_or("marker without a line")?;
        let rest: Vec<&Vec<u8>> = lines[k + 1..]
            .iter()
            .take_while(|l| !hunk_start(l))
            .collect();
        let ok = match rest.first() {
            None => true,
            Some(next) if unified => {
                prev.first() == Some(&b'-')
                    && rest
                        .iter()
                        .all(|l| l.first() == Some(&b'+') || l.starts_with(b"\\ "))
                    && next.first() == Some(&b'+')
            }
            Some(next) if context => next.starts_with(b"--- "),
            Some(next) => next.as_slice() == b"---\n",
        };
        if !ok {
            return Err(format!(
                "`\\ No newline at end of file` in the middle of a hunk, after {:?}",
                String::from_utf8_lossy(prev)
            ));
        }
    }
    Ok(())
}

/// `bytes` with every lone CR (not followed by LF) replaced by LF: what a patch writes for a CR-only file.
pub fn lone_cr_to_lf(bytes: &[u8]) -> Vec<u8> {
    bytes
        .iter()
        .enumerate()
        .map(|(k, &b)| {
            if b == b'\r' && bytes.get(k + 1) != Some(&b'\n') {
                b'\n'
            } else {
                b
            }
        })
        .collect()
}
