//! YAML scalar value resolution — span → logical string.
//!
//! A `cst:Scalar` node carries only a byte-span; the *logical* value
//! (block-scalar bodies de-indented, quotes stripped, escapes
//! applied) is what every downstream consumer wants. The seeder uses
//! this to populate the `text` attribute once, at seed-time, so TGG
//! rules can bind scalar content directly instead of re-deriving it
//! per access.

/// Resolve the logical value of a scalar from its source byte-span.
///
/// Returns `None` only when the span is out of bounds or not on a
/// `char` boundary — a malformed graph.
#[must_use]
pub fn resolve(source: &str, start: usize, end: usize) -> Option<String> {
    let raw_text = source.get(start..end)?;
    if let Some(rest) = raw_text.strip_prefix('|') {
        return Some(parse_block_scalar(rest, false));
    }
    if let Some(rest) = raw_text.strip_prefix('>') {
        return Some(parse_block_scalar(rest, true));
    }
    let raw = raw_text.trim();
    if raw.len() >= 2 {
        let bytes = raw.as_bytes();
        let (first, last) = (bytes[0], bytes[bytes.len() - 1]);
        if first == b'"' && last == b'"' {
            return Some(unescape_double_quoted(&raw[1..raw.len() - 1]));
        }
        if first == b'\'' && last == b'\'' {
            return Some(raw[1..raw.len() - 1].replace("''", "'"));
        }
    }
    Some(raw.to_string())
}

fn parse_block_scalar(body: &str, fold: bool) -> String {
    let mut rest = body;
    while let Some(s) = rest.strip_prefix('-').or_else(|| rest.strip_prefix('+')) {
        rest = s;
    }
    let rest = rest
        .strip_prefix('\n')
        .or_else(|| rest.strip_prefix("\r\n"))
        .unwrap_or(rest);
    let indent = rest
        .lines()
        .find(|l| !l.trim().is_empty())
        .map_or(0, |l| l.chars().take_while(|c| *c == ' ').count());
    let mut out = String::new();
    let mut prev_blank = false;
    let mut first = true;
    for line in rest.lines() {
        let stripped = if line.len() >= indent {
            &line[indent..]
        } else {
            line.trim_start()
        };
        if fold {
            if stripped.is_empty() {
                if !first {
                    out.push('\n');
                }
                prev_blank = true;
            } else {
                if !first && !prev_blank {
                    out.push(' ');
                }
                out.push_str(stripped);
                prev_blank = false;
            }
        } else {
            if !first {
                out.push('\n');
            }
            out.push_str(stripped);
        }
        first = false;
    }
    while out.ends_with('\n') {
        out.pop();
    }
    out
}

fn unescape_double_quoted(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars();
    while let Some(c) = chars.next() {
        if c == '\\' {
            match chars.next() {
                Some('"') => out.push('"'),
                Some('n') => out.push('\n'),
                Some('t') => out.push('\t'),
                Some('r') => out.push('\r'),
                Some('0') => out.push('\0'),
                Some('\\') | None => out.push('\\'),
                Some(other) => {
                    out.push('\\');
                    out.push(other);
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}
