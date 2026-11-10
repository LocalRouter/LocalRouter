//! Bounded, local transformations through quoted strings and HTML entities.
//! Edits are mapped back to source spans; unrelated bytes are never normalized.
use lr_types::AppResult;
use std::collections::{BTreeMap, HashMap};

#[derive(Clone, Debug)]
pub(crate) struct Edit {
    pub start: usize,
    pub end: usize,
    pub value: String,
}

pub(crate) fn rewrite(
    text: &str,
    direct: &mut dyn FnMut(&str) -> AppResult<Vec<Edit>>,
) -> AppResult<String> {
    rewrite_mode(text, direct)
}
/// Walk multiline request values in overlapping source windows. Commit only
/// at source boundaries outside quoted strings and outside any matched secret;
/// a credential crossing a cut is carried into the next window in full.
pub(crate) fn rewrite_mask(
    text: &str,
    longest_secret: usize,
    direct: &mut dyn FnMut(&str) -> AppResult<Vec<Edit>>,
) -> AppResult<(String, bool)> {
    const STEP: usize = 64 * 1024;
    let lookahead = longest_secret
        .saturating_mul(4096)
        .clamp(STEP, crate::placeholders::MAX_TAG_BYTES);
    let mut at = 0;
    let mut output = String::new();
    let mut limited = false;
    while text.len() - at > STEP + lookahead {
        let mut end = at + STEP + lookahead;
        while !text.is_char_boundary(end) {
            end -= 1;
        }
        let window = &text[at..end];
        let mut boundaries = Vec::new();
        let mut recognizer = crate::placeholders::TextStream::default();
        let mut consumed = 0;
        for line in window.split_inclusive('\n') {
            if consumed + line.len() > STEP {
                break;
            }
            let _ = recognizer.push(line, false, |s| Ok(s.into()))?;
            consumed += line.len();
            if line.ends_with('\n') && recognizer.at_boundary() {
                boundaries.push(consumed);
            }
        }
        let Some(mut cut) = boundaries.last().copied() else {
            break;
        };
        let mut budget = window.len().saturating_mul(24).min(16 * 1024 * 1024);
        let mut truncated = false;
        let all = edits(
            window,
            direct,
            0,
            &mut budget,
            &mut truncated,
            &mut HashMap::new(),
        )?;
        // Never apply an edit to only part of a source credential.
        while let Some(edit) = all.iter().find(|e| e.start < cut && e.end > cut) {
            let Some(previous) = boundaries.iter().rev().copied().find(|b| *b <= edit.start) else {
                cut = 0;
                break;
            };
            cut = previous;
        }
        if cut == 0 {
            break;
        }
        let selected: Vec<_> = all.into_iter().filter(|edit| edit.end <= cut).collect();
        output.push_str(&apply(&window[..cut], &selected));
        limited |= truncated;
        at += cut;
    }
    let (tail, truncated) = rewrite_mask_window(&text[at..], direct)?;
    output.push_str(&tail);
    limited |= truncated;
    if output.len() > crate::placeholders::MAX_TAG_BYTES {
        return Ok((text.into(), true));
    }
    Ok((output, limited))
}

fn rewrite_mask_window(
    text: &str,
    direct: &mut dyn FnMut(&str) -> AppResult<Vec<Edit>>,
) -> AppResult<(String, bool)> {
    let mut budget = text.len().saturating_mul(24).min(16 * 1024 * 1024);
    let mut truncated = false;
    let edits = edits(
        text,
        direct,
        0,
        &mut budget,
        &mut truncated,
        &mut HashMap::new(),
    )?;
    let size = edits.iter().fold(text.len(), |n, e| {
        n.saturating_sub(e.end - e.start)
            .saturating_add(e.value.len())
    });
    if size > crate::placeholders::MAX_TAG_BYTES {
        return Ok((text.into(), true));
    }
    Ok((apply(text, &edits), truncated))
}
fn rewrite_mode(
    text: &str,
    direct: &mut dyn FnMut(&str) -> AppResult<Vec<Edit>>,
) -> AppResult<String> {
    let mut budget = text.len().saturating_mul(24).min(16 * 1024 * 1024);
    let mut truncated = false;
    let edits = edits(
        text,
        direct,
        0,
        &mut budget,
        &mut truncated,
        &mut HashMap::new(),
    )?;
    let size = edits.iter().fold(text.len(), |n, e| {
        n.saturating_sub(e.end - e.start)
            .saturating_add(e.value.len())
    });
    if truncated || size > crate::placeholders::MAX_TAG_BYTES {
        return Ok(text.into());
    }
    Ok(apply(text, &edits))
}

fn apply(text: &str, edits: &[Edit]) -> String {
    let mut out = String::new();
    let mut pos = 0;
    for edit in edits {
        out.push_str(&text[pos..edit.start]);
        out.push_str(&edit.value);
        pos = edit.end;
    }
    out.push_str(&text[pos..]);
    out
}
fn overlay(edits: &mut Vec<Edit>, edit: Edit) {
    // Source edits are sorted and disjoint. Replace only wholly enclosed edits;
    // binary searches avoid repeatedly scanning/sorting a large source file.
    let first = edits.partition_point(|e| e.end <= edit.start);
    let last = edits.partition_point(|e| e.start < edit.end);
    if first < last && (edits[first].start < edit.start || edits[last - 1].end > edit.end) {
        return;
    }
    edits.splice(first..last, std::iter::once(edit));
}

struct View {
    text: String,
    spans: Vec<(u32, u32, u32, u32)>,
    escaped: BTreeMap<char, String>,
}
impl View {
    fn new() -> Self {
        Self {
            text: String::new(),
            spans: Vec::new(),
            escaped: BTreeMap::new(),
        }
    }
    fn push(&mut self, ch: char, start: usize, end: usize, raw: &str) {
        let at = self.text.len();
        self.text.push(ch);
        self.spans
            .push((at as u32, self.text.len() as u32, start as u32, end as u32));
        if raw != ch.to_string() {
            self.escaped.entry(ch).or_insert_with(|| raw.into());
        }
    }
    fn range(&self, start: usize, end: usize) -> Option<(usize, usize)> {
        Some((
            self.spans[self
                .spans
                .binary_search_by_key(&(start as u32), |s| s.0)
                .ok()?]
            .2 as usize,
            self.spans[self
                .spans
                .binary_search_by_key(&(end as u32), |s| s.1)
                .ok()?]
            .3 as usize,
        ))
    }
}
pub(crate) fn apostrophe_starts_quote(word: &str) -> bool {
    // An apostrophe inside a word is punctuation. Language literal prefixes
    // still introduce quoted strings (r'...', b'...', L'...', and similar).
    word.is_empty()
        || matches!(
            word.to_ascii_lowercase().as_str(),
            "r" | "b" | "f" | "u" | "l" | "u8" | "br" | "rb" | "fr" | "rf"
        )
}
fn quote_can_start(text: &str, at: usize, quote: char) -> bool {
    quote != '\''
        || apostrophe_starts_quote(
            text[..at]
                .rsplit(|ch: char| !(ch.is_alphanumeric() || ch == '_'))
                .next()
                .unwrap_or_default(),
        )
}

fn edits(
    text: &str,
    direct: &mut dyn FnMut(&str) -> AppResult<Vec<Edit>>,
    depth: usize,
    budget: &mut usize,
    truncated: &mut bool,
    cache: &mut HashMap<String, Vec<Edit>>,
) -> AppResult<Vec<Edit>> {
    if let Some(edits) = cache.get(text) {
        return Ok(edits.clone());
    }
    if depth > 12 || text.len() > *budget {
        *truncated = true;
        return Ok(Vec::new());
    }
    *budget -= text.len();
    let mut out = direct(text)?;
    if out.len() > 8192 {
        *truncated = true;
        out.truncate(8192);
    }
    if text.contains('&') {
        let view = html_view(text);
        if view.text != text {
            for edit in edits(&view.text, direct, depth + 1, budget, truncated, cache)? {
                if let Some((start, end)) = view.range(edit.start, edit.end) {
                    let value = html_encode(&edit.value, &view.escaped);
                    overlay(&mut out, Edit { start, end, value });
                }
            }
        }
    }
    if text.contains('\\') {
        let view = backslash_view(text);
        if view.text != text {
            for edit in edits(&view.text, direct, depth + 1, budget, truncated, cache)? {
                if let Some((start, end)) = view.range(edit.start, edit.end) {
                    let value = quote_encode(&edit.value, '"', &view.escaped);
                    overlay(&mut out, Edit { start, end, value });
                }
            }
        }
    }
    if text.contains('%') {
        let view = percent_view(text);
        if view.text != text {
            for edit in edits(&view.text, direct, depth + 1, budget, truncated, cache)? {
                if let Some((start, end)) = view.range(edit.start, edit.end) {
                    let value = percent_encode(&edit.value, &view.escaped);
                    overlay(&mut out, Edit { start, end, value });
                }
            }
        }
    }
    let mut pos = 0;
    while pos < text.len() {
        // Markdown fences are containers, not JavaScript template strings.
        // Their inner JSON/code has its own quoting; applying template escapes
        // to the whole fence would corrupt characters such as `$` in JSON.
        if text[pos..].starts_with("```") {
            if let Some(line) = text[pos..].find('\n') {
                let start = pos + line + 1;
                let end = text[start..].find("```").map_or(text.len(), |n| start + n);
                for mut edit in edits(
                    &text[start..end],
                    direct,
                    depth + 1,
                    budget,
                    truncated,
                    cache,
                )? {
                    edit.start += start;
                    edit.end += start;
                    overlay(&mut out, edit);
                }
                pos = if end < text.len() { end + 3 } else { end };
                continue;
            }
        }
        // Attribute quotes belong to the marker, not to its containing language.
        if text.as_bytes()[pos] == b'<' {
            if let Some((end, _)) = crate::placeholders::candidate(text, pos) {
                pos = end;
                continue;
            }
        }
        let ch = text[pos..].chars().next().unwrap();
        if matches!(ch, '\'' | '"' | '`') && quote_can_start(text, pos, ch) {
            if let Some((end, view)) = quoted_view(text, pos, ch) {
                let inner = edits(&view.text, direct, depth + 1, budget, truncated, cache)?;
                for edit in inner {
                    if let Some((start, finish)) = view.range(edit.start, edit.end) {
                        let value = quote_encode(&edit.value, ch, &view.escaped);
                        overlay(
                            &mut out,
                            Edit {
                                start,
                                end: finish,
                                value,
                            },
                        );
                    }
                }
                pos = end;
                continue;
            }
        }
        pos += ch.len_utf8();
    }
    let size = out.iter().fold(text.len(), |n, e| {
        n.saturating_sub(e.end - e.start)
            .saturating_add(e.value.len())
    });
    if size > crate::placeholders::MAX_TAG_BYTES || out.len() > 8192 {
        *truncated = true;
        out.clear();
    }
    if !*truncated {
        cache.insert(text.to_owned(), out.clone());
    }
    Ok(out)
}
fn entity(raw: &str) -> Option<char> {
    let value = raw.strip_prefix('&')?.strip_suffix(';')?;
    match value {
        "lt" => Some('<'),
        "gt" => Some('>'),
        "amp" => Some('&'),
        "quot" => Some('"'),
        "apos" => Some('\''),
        "hyphen" => Some('-'),
        _ => {
            let n = if let Some(hex) = value
                .strip_prefix("#x")
                .or_else(|| value.strip_prefix("#X"))
            {
                u32::from_str_radix(hex, 16).ok()?
            } else {
                value.strip_prefix('#')?.parse::<u32>().ok()?
            };
            char::from_u32(n).filter(|ch| *ch != '\0')
        }
    }
}
fn html_view(text: &str) -> View {
    let mut view = View::new();
    let mut pos = 0;
    while pos < text.len() {
        if text.as_bytes()[pos] == b'&' {
            let end = text.as_bytes()[pos..(pos + 32).min(text.len())]
                .iter()
                .position(|b| *b == b';')
                .map(|n| pos + n + 1);
            if let Some(end) = end.filter(|end| *end - pos <= 32) {
                if let Some(ch) = entity(&text[pos..end]) {
                    view.push(ch, pos, end, &text[pos..end]);
                    pos = end;
                    continue;
                }
            }
        }
        let ch = text[pos..].chars().next().unwrap();
        view.push(
            ch,
            pos,
            pos + ch.len_utf8(),
            &text[pos..pos + ch.len_utf8()],
        );
        pos += ch.len_utf8();
    }
    view
}
fn html_encode(text: &str, styles: &BTreeMap<char, String>) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if let Some(style) = styles.get(&ch) {
            out.push_str(style);
        } else {
            match ch {
                '&' => out.push_str("&amp;"),
                '<' => out.push_str("&lt;"),
                '>' => out.push_str("&gt;"),
                '"' => out.push_str("&quot;"),
                '\'' => out.push_str("&apos;"),
                _ => out.push(ch),
            }
        }
    }
    out
}
fn literal_marker_header(text: &str) -> bool {
    ["<localrouter-redacted", "<localrouter-secret"]
        .iter()
        .any(|name| {
            text.get(..name.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
                && text
                    .as_bytes()
                    .get(name.len())
                    .is_none_or(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
        })
}
fn unfinished_marker(text: &str) -> bool {
    if !literal_marker_header(text) {
        return false;
    }
    let mut i = ["<localrouter-redacted", "<localrouter-secret"]
        .iter()
        .find(|name| {
            text.get(..name.len())
                .is_some_and(|s| s.eq_ignore_ascii_case(name))
        })
        .map_or(0, |name| name.len());
    let bytes = text.as_bytes();
    loop {
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&ch) = bytes.get(i) else {
            return true;
        };
        if ch == b'>' {
            return false;
        }
        if ch == b'/' {
            return i + 1 == bytes.len();
        }
        let start = i;
        while bytes
            .get(i)
            .is_some_and(|b| b.is_ascii_alphanumeric() || matches!(b, b'_' | b'-' | b':'))
        {
            i += 1;
        }
        if i == start {
            return false;
        }
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&ch) = bytes.get(i) else {
            return true;
        };
        if ch != b'=' {
            return false;
        }
        i += 1;
        while bytes.get(i).is_some_and(u8::is_ascii_whitespace) {
            i += 1;
        }
        let Some(&quote) = bytes.get(i) else {
            return true;
        };
        if matches!(quote, b'\'' | b'"') {
            i += 1;
            while let Some(&ch) = bytes.get(i) {
                if ch == quote {
                    break;
                }
                if ch == b'<' {
                    return false;
                }
                i += 1;
            }
            if i == bytes.len() {
                return true;
            }
            i += 1;
        } else {
            let start = i;
            while bytes.get(i).is_some_and(u8::is_ascii_alphanumeric) {
                i += 1;
            }
            if i == start {
                return false;
            }
        }
        if bytes
            .get(i)
            .is_some_and(|b| !b.is_ascii_whitespace() && !matches!(b, b'/' | b'>'))
        {
            return false;
        }
    }
}

fn unclosed_marker(text: &str) -> bool {
    let mut source = text.to_owned();
    for _ in 0..12 {
        let decoded = decode_layer(&source);
        if decoded == source {
            break;
        }
        source = decoded;
    }
    source
        .match_indices('<')
        .any(|(at, _)| unfinished_marker(&source[at..]))
}

fn quoted_view(text: &str, start: usize, quote: char) -> Option<(usize, View)> {
    let mut view = View::new();
    let mut pos = start + 1;
    while pos < text.len() {
        let begin = pos;
        let ch = text[pos..].chars().next()?;
        pos += ch.len_utf8();
        if ch == quote {
            return Some((pos, view));
        }
        if ch == '<' {
            if let Some((end, _)) = crate::placeholders::candidate(text, begin) {
                for (offset, c) in text[begin..end].char_indices() {
                    let at = begin + offset;
                    view.push(c, at, at + c.len_utf8(), &text[at..at + c.len_utf8()]);
                }
                pos = end;
                continue;
            }
            if unfinished_marker(&text[begin..]) {
                return None;
            }
        }
        if ch != '\\' {
            view.push(ch, begin, pos, &text[begin..pos]);
            continue;
        }
        let esc = text[pos..].chars().next()?;
        pos += esc.len_utf8();
        let decoded = match esc {
            '\\' => '\\',
            '\'' => '\'',
            '"' => '"',
            '`' => '`',
            '/' => '/',
            '$' if quote == '`' => '$',
            'n' => '\n',
            'r' => '\r',
            't' => '\t',
            'b' => '\u{8}',
            'f' => '\u{c}',
            'u' | 'x' => {
                let n = if esc == 'x' { 2 } else { 4 };
                let digits = text.get(pos..pos + n)?;
                let mut code = u32::from_str_radix(digits, 16).ok()?;
                pos += n;
                if (0xd800..=0xdbff).contains(&code) {
                    if text.get(pos..pos + 2)? != "\\u" {
                        return None;
                    }
                    let low = u32::from_str_radix(text.get(pos + 2..pos + 6)?, 16).ok()?;
                    if !(0xdc00..=0xdfff).contains(&low) {
                        return None;
                    }
                    code = 0x10000 + ((code - 0xd800) << 10) + (low - 0xdc00);
                    pos += 6;
                }
                char::from_u32(code)?
            }
            _ => return None,
        };
        view.push(decoded, begin, pos, &text[begin..pos]);
    }
    None
}
fn quote_encode(text: &str, quote: char, styles: &BTreeMap<char, String>) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if let Some(style) = styles.get(&ch) {
            out.push_str(style);
            continue;
        }
        match ch {
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            '\u{8}' => out.push_str("\\b"),
            '\u{c}' => out.push_str("\\f"),
            c if c == quote => {
                out.push('\\');
                out.push(c);
            }
            '$' if quote == '`' => out.push_str("\\$"),
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => {
                out.push_str(&format!("\\u{:04x}", c as u32))
            }
            _ => out.push(ch),
        }
    }
    out
}

// Marker recognition needs decoded characters, not edit/source-span trees.
// Keep this prefilter linear so plain source code is not recursively explored.
fn decode_layer(source: &str) -> String {
    let mut decoded = String::with_capacity(source.len());
    let mut at = 0;
    while at < source.len() {
        let tail = &source[at..];
        if tail.starts_with('&') {
            if let Some(end) = tail.as_bytes().iter().take(32).position(|b| *b == b';') {
                if let Some(ch) = entity(&tail[..end + 1]) {
                    decoded.push(ch);
                    at += end + 1;
                    continue;
                }
            }
        }
        if tail.starts_with('%') {
            if let Some(ch) = tail
                .get(1..3)
                .and_then(|s| u8::from_str_radix(s, 16).ok())
                .filter(u8::is_ascii)
            {
                decoded.push(ch as char);
                at += 3;
                continue;
            }
        }
        if tail.starts_with('\\') {
            let kind = tail.as_bytes().get(1).copied();
            let length = match kind {
                Some(b'u') => 4,
                Some(b'x') => 2,
                _ => 0,
            };
            if length > 0 {
                if let Some(ch) = tail
                    .get(2..2 + length)
                    .and_then(|s| u32::from_str_radix(s, 16).ok())
                    .and_then(char::from_u32)
                {
                    decoded.push(ch);
                    at += length + 2;
                    continue;
                }
            } else if let Some(ch) = match kind {
                Some(b'\\') => Some('\\'),
                Some(b'"') => Some('"'),
                Some(b'\'') => Some('\''),
                Some(b'/') => Some('/'),
                Some(b'`') => Some('`'),
                _ => None,
            } {
                decoded.push(ch);
                at += 2;
                continue;
            }
        }
        let ch = tail.chars().next().expect("remaining character");
        decoded.push(ch);
        at += ch.len_utf8();
    }
    decoded
}
#[derive(Clone, Copy)]
pub(crate) enum Prefix {
    Ordinary,
    Partial,
    Marker,
    Quoted,
}
fn incomplete_escape(source: &str) -> bool {
    source == "\\"
        || source == "%"
        || (source.len() == 2
            && source.starts_with('%')
            && source.as_bytes()[1].is_ascii_hexdigit())
        || ["&lt;", "&amp;", "&quot;", "&apos;"]
            .iter()
            .any(|s| s.starts_with(source))
        || (source.starts_with("&#") && source.len() < 32 && !source.contains(';'))
        || source
            .strip_prefix("\\u")
            .is_some_and(|s| s.len() < 4 && s.bytes().all(|b| b.is_ascii_hexdigit()))
        || source
            .strip_prefix("\\x")
            .is_some_and(|s| s.len() < 2 && s.bytes().all(|b| b.is_ascii_hexdigit()))
}
pub(crate) fn marker_prefix(text: &str) -> Prefix {
    let mut source = text.to_owned();
    for _ in 0..=12 {
        if source.starts_with(['\'', '"', '`']) {
            return Prefix::Quoted;
        }
        let lower = source.to_ascii_lowercase();
        for name in ["<localrouter-redacted", "<localrouter-secret"] {
            if name.starts_with(&lower) {
                return Prefix::Partial;
            }
            if lower.starts_with(name)
                && lower
                    .as_bytes()
                    .get(name.len())
                    .is_some_and(|b| b.is_ascii_whitespace() || matches!(b, b'/' | b'>'))
            {
                return Prefix::Marker;
            }
        }
        let decoded = decode_layer(&source);
        if decoded == source {
            if let Some(at) = source.rfind(['&', '%', '\\']) {
                let before = source[..at].to_ascii_lowercase();
                if ["<localrouter-redacted", "<localrouter-secret"]
                    .iter()
                    .any(|name| name.starts_with(&before))
                    && incomplete_escape(&source[at..])
                {
                    return Prefix::Partial;
                }
            }
            return if incomplete_escape(&source) {
                Prefix::Partial
            } else {
                Prefix::Ordinary
            };
        }
        source = decoded;
    }
    Prefix::Ordinary
}
fn decoded_any(text: &str, predicate: impl Fn(&str) -> bool) -> bool {
    let mut source = text.to_owned();
    for _ in 0..=12 {
        if predicate(&source) {
            return true;
        }
        let decoded = decode_layer(&source);
        if decoded == source {
            return false;
        }
        source = decoded;
    }
    false
}
pub(crate) fn has_marker_name(text: &str) -> bool {
    decoded_any(text, |s| {
        let lower = s.to_ascii_lowercase();
        lower.contains("localrouter-redacted") || lower.contains("localrouter-secret")
    })
}
pub(crate) fn has_marker(text: &str) -> bool {
    decoded_any(text, |s| {
        s.match_indices('<').any(|(at, _)| {
            crate::placeholders::candidate(s, at)
                .is_some_and(|(_, id)| id.len() == 32 && id.bytes().all(|b| b.is_ascii_hexdigit()))
        })
    })
}

fn backslash_view(text: &str) -> View {
    let mut view = View::new();
    let mut pos = 0;
    while pos < text.len() {
        let start = pos;
        if text.as_bytes()[pos] == b'\\' {
            // Reuse the quoted decoder for exactly one escape, preserving raw
            // quotes elsewhere in a freeform or multiply escaped fragment.
            for length in [12, 6, 4, 2] {
                if let Some(raw) = text.get(pos..pos + length) {
                    let wrapped = format!("\"{raw}\"");
                    if let Some((end, decoded)) = quoted_view(&wrapped, 0, '"') {
                        if end == wrapped.len() && decoded.text.chars().count() == 1 {
                            let ch = decoded.text.chars().next().unwrap();
                            pos += length;
                            view.push(ch, start, pos, raw);
                            break;
                        }
                    }
                }
            }
            if pos != start {
                continue;
            }
        }
        let ch = text[pos..].chars().next().unwrap();
        pos += ch.len_utf8();
        view.push(ch, start, pos, &text[start..pos]);
    }
    view
}

pub(crate) fn complete_marker(text: &str) -> bool {
    decoded_any(text, |s| crate::placeholders::candidate(s, 0).is_some())
}

pub(crate) fn complete_nonreference_markup(text: &str) -> bool {
    decoded_any(text, |s| {
        if !s.starts_with('<')
            || !s.ends_with('>')
            || s.matches('<').count() != 1
            || crate::placeholders::candidate(s, 0).is_some()
        {
            return false;
        }
        let mut quote = None;
        for ch in s.chars() {
            if let Some(q) = quote {
                if ch == q {
                    quote = None;
                }
            } else if matches!(ch, '\'' | '"') {
                quote = Some(ch);
            }
        }
        quote.is_none()
    })
}

pub(crate) fn complete_quoted(text: &str) -> bool {
    if unclosed_marker(text) {
        return false;
    }
    let mut source = text.to_owned();
    for _ in 0..=12 {
        if let Some(quote) = source
            .chars()
            .next()
            .filter(|ch| matches!(ch, '\'' | '"' | '`'))
        {
            return quoted_view(&source, 0, quote).is_some_and(|(end, _)| end == source.len());
        }
        let decoded = decode_layer(&source);
        if decoded == source {
            return false;
        }
        source = decoded;
    }
    false
}

fn percent_view(text: &str) -> View {
    let mut view = View::new();
    let mut pos = 0;
    while pos < text.len() {
        let start = pos;
        if text.as_bytes()[pos] == b'%' {
            let mut bytes = Vec::new();
            let mut end = pos;
            while bytes.len() < 4 && text.as_bytes().get(end) == Some(&b'%') {
                let Some(hex) = text.get(end + 1..end + 3) else {
                    break;
                };
                let Ok(byte) = u8::from_str_radix(hex, 16) else {
                    break;
                };
                bytes.push(byte);
                end += 3;
                if let Ok(decoded) = std::str::from_utf8(&bytes) {
                    if let Some(ch) = decoded
                        .chars()
                        .next()
                        .filter(|_| decoded.chars().count() == 1)
                    {
                        view.push(ch, start, end, &text[start..end]);
                        pos = end;
                        break;
                    }
                }
            }
            if pos != start {
                continue;
            }
        }
        let ch = text[pos..].chars().next().unwrap();
        pos += ch.len_utf8();
        view.push(ch, start, pos, &text[start..pos]);
    }
    view
}
fn percent_encode(text: &str, styles: &BTreeMap<char, String>) -> String {
    let mut out = String::new();
    for ch in text.chars() {
        if let Some(style) = styles.get(&ch) {
            out.push_str(style);
        } else if ch.is_ascii_alphanumeric() || matches!(ch, '-' | '_' | '.' | '~') {
            out.push(ch);
        } else {
            let mut buf = [0; 4];
            for byte in ch.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{byte:02X}"));
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::placeholders::{restore_text, Vault, RETENTION_SECS};
    use lr_types::secret_context::SecretContext;
    use serde_json::json;
    use std::sync::Arc;
    const SECRET: &str = "synthetic-\"quoted\"-'single'-\\path\n<&>-🔐-`-${probe}";
    fn json_string(s: &str) -> String {
        serde_json::to_string(s).unwrap()
    }
    fn html(s: &str) -> String {
        html_encode(s, &BTreeMap::new())
    }
    fn code(s: &str) -> String {
        format!(
            "tools.exec_command({});",
            json!({"cmd":format!("expected = {}",json_string(s))})
        )
    }
    fn tag() -> &'static str {
        "<localrouter-redacted id='abc123' usage='Unmasked when returned verbatim'/>"
    }
    #[test]
    fn mixed_nested_encodings_preserve_each_layer_and_unrelated_bytes() {
        type Encoder = Box<dyn Fn(&str) -> String>;
        let variants: Vec<Encoder> = vec![
            Box::new(|s| s.into()),
            Box::new(json_string),
            Box::new(|s| json_string(&json_string(s))),
            Box::new(html),
            Box::new(|s| percent_encode(s, &BTreeMap::new())),
            Box::new(|s| html(&html(s))),
            Box::new(|s| json_string(&html(s))),
            Box::new(|s| html(&json_string(s))),
            Box::new(code),
            Box::new(|s| format!("```json\n{}\n```", json!({"content":s}))),
            Box::new(|s| code(&code(s))),
        ];
        for (i, encode) in variants.into_iter().enumerate() {
            let input = encode(tag());
            let result = restore_text(&input, true, |id| {
                Ok((id == "abc123").then(|| SECRET.into()))
            })
            .unwrap();
            assert_eq!(result, encode(SECRET), "encoding {i}");
        }
        for name in ["localrouter-redacted", "localrouter-secret"] {
            let input = tag().replace("localrouter-redacted", name);
            assert_eq!(
                restore_text(&input, true, |_| Ok(Some(SECRET.into()))).unwrap(),
                SECRET
            );
        }
    }
    #[test]
    fn numeric_entities_unicode_and_multiple_backslash_layers() {
        let encoded = tag()
            .replace('<', "&#x3c;")
            .replace('>', "&#62;")
            .replace('-', "&#45;")
            .replace('\'', "&#39;");
        let expected = SECRET
            .replace('<', "&#x3c;")
            .replace('>', "&#62;")
            .replace('-', "&#45;")
            .replace('\'', "&#39;")
            .replace('&', "&amp;")
            .replace('"', "&quot;");
        // Escape ampersands in original secret only, not in the generated entities.
        let expected = expected.replace("&amp;#", "&#");
        assert_eq!(
            restore_text(&encoded, true, |_| Ok(Some(SECRET.into()))).unwrap(),
            expected
        );
        for input in [
            r"\u003clocalrouter\u002dredacted id='abc123'/\u003e".to_owned(),
            r"\x3clocalrouter-secret id='abc123'/\x3e".into(),
        ] {
            let result = restore_text(&input, true, |_| Ok(Some("a<\"b\"-c".into()))).unwrap();
            assert!(!has_marker(&result));
            assert!(result.contains("a"));
            let double = input.replace('\\', "\\\\");
            assert!(!has_marker(
                &restore_text(&double, true, |_| Ok(Some("a<b".into()))).unwrap()
            ));
        }
    }
    #[test]
    fn known_originals_in_code_history_are_masked_and_round_trip() {
        let vault = Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "encoded-history",
        );
        vault.register(SECRET).unwrap();
        for input in [
            code(SECRET),
            code(&code(SECRET)),
            json_string(SECRET),
            html(SECRET),
        ] {
            let mut value = json!({"input":[{"type":"custom_tool_call","input":input}]});
            let original = value.clone();
            vault.protect(&mut value).unwrap();
            assert_ne!(value, original, "{input}");
            assert!(!value.to_string().contains("quoted"));
            vault.restore(&mut value, true).unwrap();
            assert_eq!(value, original);
        }
    }
    #[test]
    fn unresolved_malformed_and_examples_are_never_fatal_or_modified() {
        for input in [
            "<localrouter-secret",
            "<localrouter-redacted id='bad'/>",
            "'<localrouter-redacted' in text",
            "<localrouter-secret id='x' id='y'/>",
        ] {
            for encoded in [
                input.to_string(),
                json_string(input),
                html(input),
                code(input),
            ] {
                assert_eq!(restore_text(&encoded, true, |_| Ok(None)).unwrap(), encoded);
            }
        }
    }
    #[test]
    fn fragmented_escaped_markers_match_whole_response_at_every_split() {
        use crate::placeholders::TextStream;
        for input in [
            tag().to_string(),
            format!("```js\nconst value = `{}`;\n```", tag()),
            html(&format!("`{}`", tag())),
            percent_encode(&format!("`{}`", tag()), &BTreeMap::new()),
            format!("const value = `{}`;", tag()),
            format!("```json\n{}\n```", json!({"content":tag()})),
            format!("````json\n{}\n````", json!({"content":tag()})),
            format!(
                "check = '<localrouter-redacted '; value = {}",
                json_string(tag())
            ),
            format!(
                "I can't tell. I haven't checked. I didn't change this value:\n{}",
                tag()
            ),
            format!("'{}'", tag()),
            html(&format!("'{}'", tag())),
            html(tag()),
            html(&html(tag())),
            json_string(tag()),
            json_string(&json_string(tag())),
            html(&json_string(tag())),
            percent_encode(&json_string(tag()), &BTreeMap::new()),
            format!("cat <<'EOF'\ntoken = {}\nEOF", json_string(tag())),
            format!("&amp; token = {}", json_string(tag())),
            format!("%41 token = {}", json_string(tag())),
            format!(
                "example <localrouter-secret unfinished token = {}",
                json_string(tag())
            ),
            percent_encode(tag(), &BTreeMap::new()),
            r"\u003clocalrouter\u002dsecret id='abc123'/\u003e".into(),
        ] {
            let expected = restore_text(&input, true, |_| Ok(Some(SECRET.into()))).unwrap();
            for split in 0..=input.len() {
                if !input.is_char_boundary(split) {
                    continue;
                }
                let mut stream = TextStream::default();
                let mut out = String::new();
                for fragment in [&input[..split], &input[split..]] {
                    out.push_str(
                        &stream
                            .push(fragment, false, |s| {
                                restore_text(s, true, |_| Ok(Some(SECRET.into())))
                            })
                            .unwrap(),
                    );
                }
                out.push_str(
                    &stream
                        .push("", true, |s| {
                            restore_text(s, true, |_| Ok(Some(SECRET.into())))
                        })
                        .unwrap(),
                );
                assert_eq!(out, expected, "split {split}: {input}");
            }
        }
    }
    #[test]
    fn unsupported_schemas_and_unknown_references_are_metadata_only_diagnostics() {
        let vault = Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "diagnostic-policy",
        );
        let seen = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = seen.clone();
        vault.set_operation_observer(Arc::new(move |operation| {
            sink.lock().unwrap().push(operation)
        }));
        vault.register(SECRET).unwrap();
        let schema = json!({"tools":[{"type":"function","name":"test","strict":true,"parameters":{"$id":"https://example.invalid/schema","type":"object","properties":{"uuid":{"type":"string","format":"uuid"}}}}]});
        vault.set_request_contract(&schema).unwrap();
        vault
            .validate_tool(Some("test"), &json!({"uuid":"harmless"}))
            .unwrap();
        let missing = "<localrouter-secret id='00112233445566778899aabbccddeeff'/>";
        for input in [missing.to_string(), html(missing), code(missing)] {
            assert_eq!(vault.restore_text(&input, true).unwrap(), input);
        }
        assert!(seen
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.action == "restore_warning"));
        assert!(!format!("{:?}", seen.lock().unwrap()).contains(SECRET));
        let mut restorer = crate::wire::ResponseRestorer::new(Arc::new(vault));
        assert_eq!(
            restorer.message("{broken provider event").unwrap(),
            vec!["{broken provider event"]
        );
        assert_eq!(restorer.json(b"Provider busy").unwrap(), b"Provider busy");
    }
    #[test]
    fn punctuation_control_and_unicode_corpus_round_trips_through_code() {
        for n in 0..96 {
            let secret = format!(
                "synthetic-{n:03}-{}",
                [
                    "\"'\\\n\r\t<&>-$(){}[]`",
                    "é日本語🔐",
                    "\0\u{8}\u{c}",
                    "tail\\",
                    "plain"
                ][n % 5]
            );
            let vault = Vault::for_client(
                Arc::new(lr_api_keys::MockKeychain::new()),
                &format!("corpus-{n}"),
            );
            vault.register(&secret).unwrap();
            let original = code(&secret);
            let mut request = json!({"input":original});
            vault.protect(&mut request).unwrap();
            assert_ne!(request["input"], original);
            vault.restore(&mut request, true).unwrap();
            assert_eq!(request["input"], original, "case {n}");
            let args = request["input"]
                .as_str()
                .unwrap()
                .strip_prefix("tools.exec_command(")
                .unwrap()
                .strip_suffix(");")
                .unwrap();
            serde_json::from_str::<serde_json::Value>(args).unwrap();
        }
    }
    #[test]
    fn nesting_budget_is_nonfatal_for_masking_and_restoration() {
        let mut input = tag().to_owned();
        for _ in 0..20 {
            input = html(&input);
        }
        assert_eq!(
            restore_text(&input, true, |_| Ok(Some(SECRET.into()))).unwrap(),
            input
        );
        let mut deep = SECRET.to_string();
        for _ in 0..20 {
            deep = html(&deep);
        }
        let vault = Vault::for_client(Arc::new(lr_api_keys::MockKeychain::new()), "nesting-limit");
        vault.register(SECRET).unwrap();
        assert!(vault.protect(&mut json!({"input":deep})).is_ok());
    }
    #[test]
    fn credential_read_failure_does_not_abort_response_or_disclose_values() {
        struct Failed;
        impl lr_api_keys::KeychainStorage for Failed {
            fn store(&self, _: &str, _: &str, _: &str) -> lr_types::AppResult<()> {
                Err(lr_types::AppError::Internal("fixture unavailable".into()))
            }
            fn get(&self, _: &str, _: &str) -> lr_types::AppResult<Option<String>> {
                Err(lr_types::AppError::Internal("fixture unavailable".into()))
            }
            fn delete(&self, _: &str, _: &str) -> lr_types::AppResult<()> {
                Ok(())
            }
        }
        let vault = Vault::for_client(Arc::new(Failed), "unavailable-store");
        assert_eq!(vault.restore_text(tag(), true).unwrap(), tag());
        assert!(vault.register(SECRET).is_ok());
    }
    #[test]
    fn unsafe_key_collision_preserves_response_and_only_reports_diagnostic() {
        let vault = Arc::new(Vault::for_client(
            Arc::new(lr_api_keys::MockKeychain::new()),
            "collision-fallback",
        ));
        vault.register("existing-key").unwrap();
        let mut request = json!({"input":"existing-key"});
        vault.protect(&mut request).unwrap();
        let tag = request["input"].as_str().unwrap();
        let content = json!({tag:"first","existing-key":"second"}).to_string();
        assert_eq!(vault.restore_text(&content, true).unwrap(), content);
        let notices = Arc::new(std::sync::Mutex::new(Vec::new()));
        let sink = notices.clone();
        vault.set_operation_observer(Arc::new(move |e| sink.lock().unwrap().push(e)));
        let mut response = json!({"input":{tag:"first","existing-key":"second"}});
        let original = response.clone();
        vault.restore(&mut response, true).unwrap();
        assert_eq!(response, original);
        assert!(!notices
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.action == "restore"));
        assert!(notices
            .lock()
            .unwrap()
            .iter()
            .any(|e| e.action == "restore_warning"));
    }
    #[test]
    fn single_purge_preserves_other_inflight_references_and_prevents_resurrection() {
        let store = Arc::new(lr_api_keys::MockKeychain::new());
        let vault = Vault::for_client(store.clone(), "single-purge");
        vault.register("synthetic-first-secret").unwrap();
        vault.register("synthetic-second-secret").unwrap();
        let mut first = json!("synthetic-first-secret");
        let mut second = json!("synthetic-second-secret");
        vault.protect(&mut first).unwrap();
        vault.protect(&mut second).unwrap();
        let id = Vault::list_client(store.clone(), "single-purge")
            .unwrap()
            .into_iter()
            .find(|e| e.placeholder == first.as_str().unwrap())
            .unwrap()
            .reference_id;
        Vault::purge_client(store.clone(), "single-purge", Some(&id)).unwrap();
        assert_eq!(
            vault.restore_text(second.as_str().unwrap(), true).unwrap(),
            "synthetic-second-secret"
        );
        assert_eq!(
            vault.restore_text(first.as_str().unwrap(), true).unwrap(),
            first.as_str().unwrap()
        );
        assert!(vault.register("synthetic-first-secret").is_ok());
        assert!(!vault.knows("synthetic-first-secret").unwrap());
        let new = Vault::find_client(store, "single-purge", RETENTION_SECS)
            .unwrap()
            .unwrap();
        new.register("synthetic-first-secret").unwrap();
        let mut renewed = json!("synthetic-first-secret");
        new.protect(&mut renewed).unwrap();
        assert_ne!(renewed, first);
    }
}

#[cfg(test)]
mod prose_quote_tests {
    #[test]
    fn contractions_do_not_escape_plaintext_originals() {
        let tag = "<localrouter-secret id='abc123'/>";
        let secret = "original-'apostrophe'-\"quote\"-\\path\nline";
        let input = format!("I didn't change {tag}; here's the value.");
        let expected = input.replace(tag, secret);
        assert_eq!(
            crate::placeholders::restore_text(&input, false, |_| Ok(Some(secret.into()))).unwrap(),
            expected
        );
        for size in [1, 2, 7] {
            let mut stream = crate::placeholders::TextStream::default();
            let mut actual = String::new();
            for fragment in input.as_bytes().chunks(size) {
                actual.push_str(
                    &stream
                        .push(std::str::from_utf8(fragment).unwrap(), false, |s| {
                            crate::placeholders::restore_text(s, false, |_| Ok(Some(secret.into())))
                        })
                        .unwrap(),
                );
            }
            actual.push_str(
                &stream
                    .push("", true, |s| {
                        crate::placeholders::restore_text(s, false, |_| Ok(Some(secret.into())))
                    })
                    .unwrap(),
            );
            assert_eq!(actual, expected);
        }
    }
}
