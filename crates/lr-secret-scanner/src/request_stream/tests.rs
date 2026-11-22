use super::*;
use serde_json::{json, Value};

/// Deterministic generator (xorshift64*), so failures reproduce from the seed.
struct Rng(u64);

impl Rng {
    fn new(seed: u64) -> Self {
        Self(seed.wrapping_mul(0x9E37_79B9_7F4A_7C15) | 1)
    }
    fn next(&mut self) -> u64 {
        self.0 ^= self.0 >> 12;
        self.0 ^= self.0 << 25;
        self.0 ^= self.0 >> 27;
        self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
    }
    fn below(&mut self, bound: usize) -> usize {
        (self.next() % bound as u64) as usize
    }
    fn chance(&mut self, percent: usize) -> bool {
        self.below(100) < percent
    }
    fn pick<'a>(&mut self, items: &[&'a str]) -> &'a str {
        items[self.below(items.len())]
    }
}

fn whitespace(rng: &mut Rng, out: &mut String) {
    while rng.chance(25) {
        out.push_str(rng.pick(&[" ", "\n", "\t", "\r", "  "]));
    }
}

/// String body in assorted spellings: raw multi-byte text and every escape
/// form, including surrogate pairs in either hex case.
fn string_body(rng: &mut Rng, out: &mut String) {
    for _ in 0..rng.below(12) {
        out.push_str(rng.pick(&[
            "a",
            "Zq",
            " ",
            "é",
            "日本",
            "🔑",
            "\u{7f}",
            "\\n",
            "\\\"",
            "\\\\",
            "\\/",
            "\\b",
            "\\f",
            "\\r",
            "\\t",
            "\\u00e9",
            "\\u0000",
            "\\u001F",
            "\\uD83D\\uDD11",
            "\\ud83d\\udd11",
            "{",
            "]",
            ",",
            ":",
        ]));
    }
}

fn value(rng: &mut Rng, depth: usize, out: &mut String) {
    whitespace(rng, out);
    let kind = if depth >= 5 {
        rng.below(3)
    } else {
        rng.below(5)
    };
    match kind {
        0 => out.push_str(rng.pick(&[
            "0",
            "-0",
            "1",
            "-12",
            "3.25",
            "1e5",
            "1E+5",
            "2.5e-3",
            "123456789012",
            "-0.0",
            "9007199254740993",
        ])),
        1 => out.push_str(rng.pick(&["true", "false", "null"])),
        2 => {
            out.push('"');
            string_body(rng, out);
            out.push('"');
        }
        3 => {
            out.push('{');
            let members = rng.below(4);
            for member in 0..members {
                whitespace(rng, out);
                // The index keeps decoded keys unique within the object.
                out.push_str(&format!("\"{member}"));
                string_body(rng, out);
                out.push('"');
                whitespace(rng, out);
                out.push(':');
                value(rng, depth + 1, out);
                if member + 1 < members {
                    out.push(',');
                }
            }
            whitespace(rng, out);
            out.push('}');
        }
        _ => {
            out.push('[');
            let elements = rng.below(4);
            for element in 0..elements {
                value(rng, depth + 1, out);
                if element + 1 < elements {
                    out.push(',');
                }
            }
            whitespace(rng, out);
            out.push(']');
        }
    }
    whitespace(rng, out);
}

fn document(seed: u64) -> String {
    let mut rng = Rng::new(seed);
    let mut out = String::new();
    value(&mut rng, 0, &mut out);
    out
}

#[derive(Clone, Debug, PartialEq)]
enum Ev {
    ObjectStart,
    ObjectEnd(u64),
    ArrayStart,
    ArrayEnd(u64),
    Key(String, bool),
    StringStart,
    Chunk(String),
    StringEnd,
    Scalar(String, bool),
    Opaque(bool),
}

#[derive(Clone, Debug, PartialEq)]
struct Rec {
    path: String,
    span: Span,
    ev: Ev,
}

fn record(records: &mut Vec<Rec>, path: &Path, span: Span, event: Event<'_>) {
    let ev = match event {
        Event::ObjectStart => Ev::ObjectStart,
        Event::ObjectEnd { len } => Ev::ObjectEnd(len),
        Event::ArrayStart => Ev::ArrayStart,
        Event::ArrayEnd { len } => Ev::ArrayEnd(len),
        Event::Key { text, truncated } => Ev::Key(text.into(), truncated),
        Event::StringStart => Ev::StringStart,
        Event::StringChunk { text } => {
            // One chunk is verbatim text or a single escape sequence.
            assert!(!text.is_empty() && text.len() as u64 <= span.len());
            Ev::Chunk(text.into())
        }
        Event::StringEnd => Ev::StringEnd,
        Event::Scalar { raw, truncated } => Ev::Scalar(raw.into(), truncated),
        Event::Opaque { last } => Ev::Opaque(last),
    };
    assert_eq!(path.pointer(), path.to_string());
    assert_eq!(path.segments().count(), path.depth());
    records.push(Rec {
        path: path.pointer(),
        span,
        ev,
    });
}

fn lex_with<'a>(
    limits: LexerLimits,
    chunks: impl IntoIterator<Item = &'a [u8]>,
) -> (Vec<Rec>, Result<(), LexError>) {
    let mut lexer = Lexer::with_limits(limits);
    let mut records = Vec::new();
    let mut fed = 0;
    for chunk in chunks {
        fed += chunk.len() as u64;
        if let Err(error) = lexer.push(chunk, |p, s, e| record(&mut records, p, s, e)) {
            assert_eq!(lexer.error(), Some(error));
            assert_eq!(lexer.push(b"1", |_, _, _| panic!("spent")), Err(error));
            assert_eq!(lexer.finish(|_, _, _| panic!("spent")), Err(error));
            return (records, Err(error));
        }
        assert_eq!(lexer.bytes_fed(), fed);
        assert!(lexer.settled() <= fed);
        // Nothing delivered reaches past the settled offset.
        assert!(records.last().is_none_or(|r| r.span.end <= lexer.settled()));
    }
    let result = lexer.finish(|p, s, e| record(&mut records, p, s, e));
    assert_eq!(result.is_ok(), lexer.is_complete());
    (records, result)
}

fn lex<'a>(chunks: impl IntoIterator<Item = &'a [u8]>) -> (Vec<Rec>, Result<(), LexError>) {
    lex_with(LexerLimits::default(), chunks)
}

/// Join the pieces whose cut points depend on chunking.
fn merged(records: Vec<Rec>) -> Vec<Rec> {
    let mut out: Vec<Rec> = Vec::new();
    for rec in records {
        if let Some(last) = out.last_mut() {
            let joins = match (&mut last.ev, &rec.ev) {
                (Ev::Chunk(text), Ev::Chunk(more)) => {
                    text.push_str(more);
                    true
                }
                (Ev::Opaque(last @ false), Ev::Opaque(next)) => {
                    *last = *next;
                    true
                }
                _ => false,
            };
            if joins {
                assert_eq!(last.span.end, rec.span.start, "pieces are contiguous");
                assert_eq!(last.path, rec.path);
                last.span.end = rec.span.end;
                continue;
            }
        }
        out.push(rec);
    }
    out
}

fn random_cuts(rng: &mut Rng, input: &[u8]) -> Vec<Vec<u8>> {
    let mut chunks = Vec::new();
    let mut at = 0;
    while at < input.len() {
        let len = 1 + rng.below(9).min(input.len() - at - 1);
        chunks.push(input[at..at + len].to_vec());
        at += len;
    }
    chunks
}

/// Every chunking under test, the unfragmented one first.
fn chunkings(seed: u64, input: &[u8]) -> Vec<Vec<Vec<u8>>> {
    let mut all = vec![vec![input.to_vec()]];
    for size in [1, 2, 3, 5, 7, 64] {
        all.push(input.chunks(size).map(<[u8]>::to_vec).collect());
    }
    let mut rng = Rng::new(seed ^ 0xC0FFEE);
    for _ in 0..3 {
        all.push(random_cuts(&mut rng, input));
    }
    // Empty chunks are legal anywhere.
    let mut sparse = vec![Vec::new()];
    for chunk in input.chunks(4) {
        sparse.push(chunk.to_vec());
        sparse.push(Vec::new());
    }
    all.push(sparse);
    all
}

fn lex_chunked(chunks: &[Vec<u8>]) -> (Vec<Rec>, Result<(), LexError>) {
    lex(chunks.iter().map(Vec::as_slice))
}

/// Rebuild the document from events alone.
fn rebuild(records: &[Rec]) -> Value {
    let mut stack: Vec<(Value, Option<String>)> = Vec::new();
    let mut root = None;
    let mut text = String::new();
    fn place(stack: &mut [(Value, Option<String>)], root: &mut Option<Value>, value: Value) {
        match stack.last_mut() {
            None => *root = Some(value),
            Some((Value::Array(items), _)) => items.push(value),
            Some((Value::Object(map), key)) => {
                map.insert(key.take().expect("key before value"), value);
            }
            Some(_) => unreachable!(),
        }
    }
    for rec in records {
        match &rec.ev {
            Ev::ObjectStart => stack.push((json!({}), None)),
            Ev::ArrayStart => stack.push((json!([]), None)),
            Ev::ObjectEnd(len) | Ev::ArrayEnd(len) => {
                let (value, _) = stack.pop().unwrap();
                let actual = match &value {
                    Value::Array(items) => items.len(),
                    Value::Object(map) => map.len(),
                    _ => unreachable!(),
                };
                assert_eq!(*len, actual as u64);
                place(&mut stack, &mut root, value);
            }
            Ev::Key(key, truncated) => {
                assert!(!truncated);
                stack.last_mut().unwrap().1 = Some(key.clone());
            }
            Ev::StringStart => text.clear(),
            Ev::Chunk(chunk) => text.push_str(chunk),
            Ev::StringEnd => place(&mut stack, &mut root, Value::String(text.clone())),
            Ev::Scalar(raw, truncated) => {
                assert!(!truncated);
                place(&mut stack, &mut root, serde_json::from_str(raw).unwrap());
            }
            Ev::Opaque(_) => panic!("unexpected opaque subtree"),
        }
    }
    assert!(stack.is_empty());
    root.expect("root value")
}

/// Spans are ordered, in bounds, and separated only by whitespace and
/// punctuation; copying gaps and spans in order reproduces the input.
fn assert_spans_reassemble(input: &[u8], records: &[Rec]) {
    let mut output = Vec::new();
    let mut at = 0usize;
    for rec in records {
        let (start, end) = (rec.span.start as usize, rec.span.end as usize);
        assert!(at <= start && start < end && end <= input.len(), "{rec:?}");
        let gap = &input[at..start];
        assert!(
            gap.iter().all(|b| b" \t\r\n,:".contains(b)),
            "gap {gap:?} before {rec:?}"
        );
        output.extend_from_slice(gap);
        let raw = &input[start..end];
        match &rec.ev {
            Ev::ObjectStart => assert_eq!(raw, b"{"),
            Ev::ArrayStart => assert_eq!(raw, b"["),
            Ev::ObjectEnd(_) => assert_eq!(raw, b"}"),
            Ev::ArrayEnd(_) => assert_eq!(raw, b"]"),
            Ev::StringStart | Ev::StringEnd => assert_eq!(raw, b"\""),
            Ev::Key(key, false) => {
                assert_eq!(&serde_json::from_slice::<String>(raw).unwrap(), key)
            }
            Ev::Chunk(text) => {
                let quoted = [b"\"", raw, b"\""].concat();
                assert_eq!(&serde_json::from_slice::<String>(&quoted).unwrap(), text);
            }
            Ev::Scalar(text, false) => assert_eq!(raw, text.as_bytes()),
            Ev::Key(_, true) | Ev::Scalar(_, true) | Ev::Opaque(_) => {}
        }
        output.extend_from_slice(raw);
        at = end;
    }
    assert!(input[at..].iter().all(u8::is_ascii_whitespace));
    output.extend_from_slice(&input[at..]);
    assert_eq!(output, input);
}

/// Each value's path resolves, in the parsed document, to that value.
fn assert_paths_resolve(document: &Value, records: &[Rec]) {
    let mut text = String::new();
    for rec in records {
        let target = document
            .pointer(&rec.path)
            .unwrap_or_else(|| panic!("path {} does not resolve", rec.path));
        match &rec.ev {
            Ev::ObjectStart | Ev::ObjectEnd(_) => assert!(target.is_object()),
            Ev::ArrayStart | Ev::ArrayEnd(_) => assert!(target.is_array()),
            Ev::StringStart => text.clear(),
            Ev::Chunk(chunk) => text.push_str(chunk),
            Ev::StringEnd => assert_eq!(target.as_str(), Some(text.as_str())),
            Ev::Scalar(raw, _) => assert_eq!(target, &serde_json::from_str::<Value>(raw).unwrap()),
            Ev::Key(..) | Ev::Opaque(_) => {}
        }
    }
}

#[test]
fn random_documents_lex_identically_under_every_chunking() {
    for seed in 0..400 {
        let text = document(seed);
        let input = text.as_bytes();
        let expected: Value = serde_json::from_str(&text)
            .unwrap_or_else(|error| panic!("seed {seed} generated invalid JSON: {error}\n{text}"));
        let mut reference = None;
        for chunks in chunkings(seed, input) {
            let (records, result) = lex_chunked(&chunks);
            assert_eq!(result, Ok(()), "seed {seed}: {text}");
            // Spans from a fragmented run index the original bytes.
            assert_spans_reassemble(input, &records);
            let records = merged(records);
            assert_spans_reassemble(input, &records);
            assert_eq!(rebuild(&records), expected, "seed {seed}: {text}");
            assert_paths_resolve(&expected, &records);
            match &reference {
                None => reference = Some(records),
                Some(reference) => assert_eq!(&records, reference, "seed {seed}: {text}"),
            }
        }
    }
}

#[test]
fn mutated_documents_are_accepted_exactly_when_serde_accepts_them() {
    let mut accepted = 0;
    let mut rejected = 0;
    for seed in 0..3000u64 {
        let mut rng = Rng::new(seed ^ 0xBAD5EED);
        let mut input = document(seed % 500).into_bytes();
        for _ in 0..1 + rng.below(2) {
            if input.is_empty() {
                break;
            }
            let at = rng.below(input.len());
            let byte = if rng.chance(70) {
                b"{}[],:\"\\u0123456789abcdefnrtl-+.eE \n"[rng.below(36)]
            } else {
                rng.below(256) as u8
            };
            match rng.below(3) {
                0 => input[at] = byte,
                1 => input.insert(at, byte),
                _ => {
                    input.remove(at);
                }
            }
        }
        let serde = serde_json::from_slice::<Value>(&input);
        let mut outcome = None;
        for chunks in [
            vec![input.clone()],
            input.chunks(1).map(<[u8]>::to_vec).collect(),
            random_cuts(&mut rng, &input),
        ] {
            let (records, result) = lex_chunked(&chunks);
            let shown = String::from_utf8_lossy(&input);
            // The same verdict, at the same offset, for every chunking.
            assert_eq!(
                *outcome.get_or_insert(result),
                result,
                "seed {seed}: {shown}"
            );
            match (result, &serde) {
                (Ok(()), Ok(expected)) => {
                    let records = merged(records);
                    assert_spans_reassemble(&input, &records);
                    // Mutation may produce duplicate keys; the last one wins
                    // in both parsers.
                    assert_eq!(&rebuild(&records), expected, "seed {seed}: {shown}");
                }
                (Err(_), Err(_)) => {}
                // The lexer does not evaluate number magnitudes.
                (Ok(()), Err(error)) if error.to_string().contains("out of range") => {}
                (lexed, serde) => panic!("seed {seed}: lexer {lexed:?}, serde {serde:?}: {shown}"),
            }
            if let Err(LexError::InvalidUtf8 { offset }) = result {
                let error = std::str::from_utf8(&input).unwrap_err();
                assert_eq!(error.valid_up_to() as u64, offset, "seed {seed}");
            }
            if let (Err(error), Err(utf8)) = (result, std::str::from_utf8(&input)) {
                assert!(error.offset() <= utf8.valid_up_to() as u64, "seed {seed}");
            }
        }
        match outcome {
            Some(Ok(())) => accepted += 1,
            _ => rejected += 1,
        }
    }
    // The corpus exercises both verdicts.
    assert!(accepted > 100 && rejected > 1000, "{accepted} / {rejected}");
}

fn events(text: &str) -> Vec<(String, Ev)> {
    let (records, result) = lex([text.as_bytes()]);
    assert_eq!(result, Ok(()), "{text}");
    merged(records)
        .into_iter()
        .map(|rec| (rec.path, rec.ev))
        .collect()
}

#[test]
fn paths_follow_keys_and_indexes_whatever_the_member_order() {
    let chunk = |text: &str| Ev::Chunk(text.into());
    let key = |text: &str| Ev::Key(text.into(), false);
    // `role` after `content`, `signature` after `thinking`, `model` last.
    let text = r#"{"messages":[{"content":[{"thinking":"t","signature":"s"}],"role":"user"}],"model":"m"}"#;
    let expected = vec![
        ("".into(), Ev::ObjectStart),
        ("/messages".into(), key("messages")),
        ("/messages".into(), Ev::ArrayStart),
        ("/messages/0".into(), Ev::ObjectStart),
        ("/messages/0/content".into(), key("content")),
        ("/messages/0/content".into(), Ev::ArrayStart),
        ("/messages/0/content/0".into(), Ev::ObjectStart),
        ("/messages/0/content/0/thinking".into(), key("thinking")),
        ("/messages/0/content/0/thinking".into(), Ev::StringStart),
        ("/messages/0/content/0/thinking".into(), chunk("t")),
        ("/messages/0/content/0/thinking".into(), Ev::StringEnd),
        ("/messages/0/content/0/signature".into(), key("signature")),
        ("/messages/0/content/0/signature".into(), Ev::StringStart),
        ("/messages/0/content/0/signature".into(), chunk("s")),
        ("/messages/0/content/0/signature".into(), Ev::StringEnd),
        ("/messages/0/content/0".into(), Ev::ObjectEnd(2)),
        ("/messages/0/content".into(), Ev::ArrayEnd(1)),
        ("/messages/0/role".into(), key("role")),
        ("/messages/0/role".into(), Ev::StringStart),
        ("/messages/0/role".into(), chunk("user")),
        ("/messages/0/role".into(), Ev::StringEnd),
        ("/messages/0".into(), Ev::ObjectEnd(2)),
        ("/messages".into(), Ev::ArrayEnd(1)),
        ("/model".into(), key("model")),
        ("/model".into(), Ev::StringStart),
        ("/model".into(), chunk("m")),
        ("/model".into(), Ev::StringEnd),
        ("".into(), Ev::ObjectEnd(2)),
    ];
    assert_eq!(events(text), expected);
    for chunks in chunkings(1, text.as_bytes()) {
        let (records, result) = lex_chunked(&chunks);
        assert_eq!(result, Ok(()));
        let actual: Vec<_> = merged(records)
            .into_iter()
            .map(|rec| (rec.path, rec.ev))
            .collect();
        assert_eq!(actual, expected);
    }
}

#[test]
fn path_patterns_and_pointer_escaping() {
    let text = r#"{"input":[0,{"content":[{"a/b~c":"x"}]}]}"#;
    let mut lexer = Lexer::new();
    let mut seen = Vec::new();
    lexer
        .push(text.as_bytes(), |path, _, event| {
            if matches!(event, Event::StringChunk { .. }) {
                assert_eq!(path.pointer(), "/input/1/content/0/a~1b~0c");
                assert_eq!(path.depth(), 5);
                assert!(!path.is_root());
                assert_eq!(path.last(), Some(Segment::Key("a/b~c")));
                assert_eq!(path.segment(1), Some(Segment::Index(1)));
                assert_eq!(path.segment(5), None);
                let pattern = [
                    Step::Key("input"),
                    Step::AnyIndex,
                    Step::Key("content"),
                    Step::Index(0),
                    Step::AnyKey,
                ];
                assert!(path.matches(&pattern));
                assert!(path.starts_with(&pattern[..2]));
                assert!(!path.matches(&pattern[..4]));
                assert!(!path.starts_with(&[Step::Key("messages")]));
                assert!(!path.starts_with(&[Step::AnyIndex]));
                assert!(!path.starts_with(&[Step::Key("input"), Step::AnyKey]));
                assert!(!path.starts_with(&[Step::Key("input"), Step::Index(0)]));
                seen.push(path.pointer());
            }
        })
        .unwrap();
    lexer.finish(|_, _, _| {}).unwrap();
    assert_eq!(seen.len(), 1);
    assert!(lexer.path().is_root() && lexer.path().last().is_none());
}

#[test]
fn escapes_and_surrogate_pairs_decode_across_every_split() {
    let text = r#"["\"\\\/\b\f\n\r\t","\u00e9\u65E5","\ud83d\udd11\uD83D\uDD11","é🔑"]"#;
    let expected = vec![
        ("".into(), Ev::ArrayStart),
        ("/0".into(), Ev::StringStart),
        ("/0".into(), Ev::Chunk("\"\\/\u{8}\u{c}\n\r\t".into())),
        ("/0".into(), Ev::StringEnd),
        ("/1".into(), Ev::StringStart),
        ("/1".into(), Ev::Chunk("é日".into())),
        ("/1".into(), Ev::StringEnd),
        ("/2".into(), Ev::StringStart),
        ("/2".into(), Ev::Chunk("🔑🔑".into())),
        ("/2".into(), Ev::StringEnd),
        ("/3".into(), Ev::StringStart),
        ("/3".into(), Ev::Chunk("é🔑".into())),
        ("/3".into(), Ev::StringEnd),
        ("".into(), Ev::ArrayEnd(4)),
    ];
    assert_eq!(events(text), expected);
    let input = text.as_bytes();
    // Two chunks cut at every byte, including inside escapes, between the
    // halves of a pair and inside raw multi-byte characters.
    for cut in 0..=input.len() {
        let (records, result) = lex([&input[..cut], &input[cut..]]);
        assert_eq!(result, Ok(()));
        assert_spans_reassemble(input, &records);
        let actual: Vec<_> = merged(records)
            .into_iter()
            .map(|r| (r.path, r.ev))
            .collect();
        assert_eq!(actual, expected, "cut at {cut}");
    }
    // Unmerged: a surrogate pair is one chunk spanning its twelve raw bytes,
    // and a raw character split across chunks is delivered whole.
    let pair = br#""\ud83d\udd11""#;
    let (records, _) = lex(pair.chunks(1));
    assert_eq!(
        records[1],
        Rec {
            path: "".into(),
            span: Span { start: 1, end: 13 },
            ev: Ev::Chunk("🔑".into())
        }
    );
    assert_eq!(records.len(), 3);
    let raw = "\"🔑\"".as_bytes();
    let (records, _) = lex(raw.chunks(1));
    assert_eq!(
        records[1],
        Rec {
            path: "".into(),
            span: Span { start: 1, end: 5 },
            ev: Ev::Chunk("🔑".into())
        }
    );
}

#[test]
fn settled_offset_waits_for_incomplete_tokens() {
    let mut lexer = Lexer::new();
    let push = |lexer: &mut Lexer, bytes: &[u8]| lexer.push(bytes, |_, _, _| {}).unwrap();
    push(&mut lexer, br#"{"ke"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (1, 4));
    push(&mut lexer, br#"\u00"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (1, 8));
    push(&mut lexer, br#"41y":"ab"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (16, 16));
    push(&mut lexer, br#"\ud83d\ud"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (16, 25));
    push(&mut lexer, br#"d11"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (28, 28));
    push(&mut lexer, &"é".as_bytes()[..1]);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (28, 29));
    push(&mut lexer, &"é".as_bytes()[1..]);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (30, 30));
    push(&mut lexer, br#"","n":12"#);
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (36, 38));
    assert!(!lexer.is_complete());
    push(&mut lexer, b"}");
    assert_eq!((lexer.settled(), lexer.bytes_fed()), (39, 39));
    assert!(lexer.is_complete());
    lexer.finish(|_, _, _| {}).unwrap();
}

fn verdict(input: &[u8]) -> Result<(), LexError> {
    let whole = lex([input]).1;
    let bytes = lex(input.chunks(1)).1;
    assert_eq!(whole, bytes, "{:?}", String::from_utf8_lossy(input));
    whole
}

#[test]
fn invalid_escapes_and_lone_surrogates_are_rejected() {
    for (text, offset) in [
        (r#""\x""#, 2),
        (r#""\u12G4""#, 5),
        (r#""\u12"#, 5),
        // Trailing surrogate without a leading one.
        (r#""\udd11""#, 6),
        // Leading surrogate followed by text, by another escape, by a
        // non-surrogate unit, by another leading surrogate.
        (r#""\ud83dx""#, 7),
        (r#""\ud83d\n""#, 8),
        (r#""\ud83d\u0041""#, 12),
        (r#""\ud83d\ud83d""#, 12),
        (r#""\ud83d""#, 7),
    ] {
        let result = verdict(text.as_bytes());
        if offset as usize == text.len() {
            assert_eq!(result, Err(LexError::Truncated { offset }), "{text}");
        } else {
            assert_eq!(result, Err(LexError::Syntax { offset }), "{text}");
        }
        assert!(serde_json::from_str::<Value>(text).is_err(), "{text}");
    }
}

#[test]
fn invalid_utf8_is_rejected_at_its_offset() {
    let wrap = |bytes: &[u8]| [b"{\"k\":\"ab".as_slice(), bytes, b"\"}"].concat();
    for bad in [
        &[0xFF][..],
        // Overlong encoding, encoded surrogate, code point above U+10FFFF.
        &[0xC0, 0x80],
        &[0xED, 0xA0, 0x80],
        &[0xF4, 0x90, 0x80, 0x80],
        // Continuation without a lead, lead followed by ASCII.
        &[0x80],
        &[0xE2, 0x28, 0xA1],
        &[0xF0, 0x9F, 0x94],
    ] {
        let input = wrap(bad);
        assert_eq!(
            verdict(&input),
            Err(LexError::InvalidUtf8 { offset: 8 }),
            "{bad:x?}"
        );
        assert_eq!(std::str::from_utf8(&input).unwrap_err().valid_up_to(), 8);
        // The valid prefix was still delivered.
        let (records, _) = lex(input.chunks(1));
        assert_eq!(merged(records).last().unwrap().ev, Ev::Chunk("ab".into()));
    }
    // Outside a string too, including in an opaque subtree.
    assert_eq!(
        verdict(b"[1,\xFF]"),
        Err(LexError::InvalidUtf8 { offset: 3 })
    );
    let limits = LexerLimits {
        max_depth: 0,
        ..Default::default()
    };
    assert_eq!(
        lex_with(limits, [&b"[1,\xFF]"[..]]).1,
        Err(LexError::InvalidUtf8 { offset: 3 })
    );
    // Input ending inside a character.
    let cut = "\"é🔑".as_bytes();
    for len in [2, 4, 5, 6] {
        assert_eq!(
            verdict(&cut[..len]),
            Err(LexError::InvalidUtf8 {
                offset: if len == 2 { 1 } else { 3 }
            })
        );
    }
    // A valid character outside a string is a syntax error, not a UTF-8 one.
    assert_eq!(
        verdict("[é]".as_bytes()),
        Err(LexError::Syntax { offset: 1 })
    );
}

#[test]
fn nesting_beyond_the_limit_is_reported_as_an_opaque_subtree() {
    // The default limit is the deepest nesting serde_json parses.
    let nested = |depth: usize| format!("{}1{}", "[".repeat(depth), "]".repeat(depth));
    assert!(serde_json::from_str::<Value>(&nested(127)).is_ok());
    assert!(serde_json::from_str::<Value>(&nested(128)).is_err());
    let (records, result) = lex(nested(127).as_bytes().chunks(3));
    assert_eq!(result, Ok(()));
    assert_eq!(
        rebuild(&merged(records)),
        serde_json::from_str::<Value>(&nested(127)).unwrap()
    );

    let deep = nested(300);
    for chunks in chunkings(7, deep.as_bytes()) {
        let (records, result) = lex_chunked(&chunks);
        assert_eq!(result, Ok(()));
        assert_spans_reassemble(deep.as_bytes(), &records);
        let records = merged(records);
        let opaque: Vec<_> = records
            .iter()
            .filter(|rec| matches!(rec.ev, Ev::Opaque(_)))
            .collect();
        assert_eq!(opaque.len(), 1);
        assert_eq!(opaque[0].ev, Ev::Opaque(true));
        assert_eq!(
            opaque[0].span,
            Span {
                start: 127,
                end: 300 + 1 + 173
            }
        );
        assert_eq!(opaque[0].path, "/0".repeat(127));
        // 127 starts, the subtree, 127 ends.
        assert_eq!(records.len(), 255);
    }

    // Lexing resumes after the subtree; brackets and quotes inside its
    // strings do not end it.
    let limits = LexerLimits {
        max_depth: 2,
        ..Default::default()
    };
    let text = r#"{"a":{"b":{"c":[1,"}]\"]"],"é":{}},"d":[]},"e":2}"#;
    let inner = r#"{"c":[1,"}]\"]"],"é":{}}"#;
    let start = text.find(inner).unwrap() as u64;
    for chunks in chunkings(9, text.as_bytes()) {
        let (records, result) = lex_with(limits, chunks.iter().map(Vec::as_slice));
        assert_eq!(result, Ok(()));
        assert_spans_reassemble(text.as_bytes(), &records);
        let actual: Vec<_> = merged(records)
            .into_iter()
            .filter(|rec| !matches!(rec.ev, Ev::Key(..)))
            .map(|rec| (rec.path, rec.span.start, rec.ev))
            .collect();
        let expected = vec![
            ("".to_string(), 0, Ev::ObjectStart),
            ("/a".into(), 5, Ev::ObjectStart),
            ("/a/b".into(), start, Ev::Opaque(true)),
            (
                "/a/d".into(),
                start + inner.len() as u64 + 5,
                Ev::Opaque(true),
            ),
            (
                "/a".into(),
                start + inner.len() as u64 + 7,
                Ev::ObjectEnd(2),
            ),
            (
                "/e".into(),
                text.len() as u64 - 2,
                Ev::Scalar("2".into(), false),
            ),
            ("".into(), text.len() as u64 - 1, Ev::ObjectEnd(2)),
        ];
        assert_eq!(actual, expected);
    }
    // A root container can itself be beyond the limit.
    let limits = LexerLimits {
        max_depth: 0,
        ..Default::default()
    };
    let (records, result) = lex_with(limits, [&b" [1,[2]] "[..]]);
    assert_eq!(result, Ok(()));
    assert_eq!(
        records,
        vec![Rec {
            path: "".into(),
            span: Span { start: 1, end: 8 },
            ev: Ev::Opaque(true)
        }]
    );
    // An unterminated opaque subtree is truncated input.
    assert_eq!(
        lex_with(limits, [&b"[1,[2]"[..]]).1,
        Err(LexError::Truncated { offset: 6 })
    );
}

#[test]
fn oversized_keys_and_scalars_are_bounded() {
    // 4095 ASCII bytes, then two-byte characters: the cut may not split one.
    let long = format!("{}{}", "k".repeat(4095), "é".repeat(600));
    let digits = "7".repeat(5000);
    let text = format!(r#"{{"{long}":{{"in\u0041":1}},"n":{digits},"z":null}}"#);
    for chunks in chunkings(3, text.as_bytes()) {
        let (records, result) = lex_chunked(&chunks);
        assert_eq!(result, Ok(()));
        assert_spans_reassemble(text.as_bytes(), &records);
        let records = merged(records);
        let Ev::Key(key, true) = &records[1].ev else {
            panic!("{:?}", records[1].ev)
        };
        assert_eq!(key, &"k".repeat(4095));
        assert_eq!(
            records[1].span,
            Span {
                start: 1,
                end: 1 + long.len() as u64 + 2
            }
        );
        // The path carries the shortened key; nested paths stay correct.
        assert_eq!(records[1].path, format!("/{key}"));
        assert_eq!(records[3].ev, Ev::Key("inA".into(), false));
        assert_eq!(records[3].path, format!("/{key}/inA"));
        let scalar = records
            .iter()
            .find(|rec| rec.path == "/n" && matches!(rec.ev, Ev::Scalar(..)))
            .unwrap();
        assert_eq!(scalar.ev, Ev::Scalar("7".repeat(4096), true));
        assert_eq!(scalar.span.len(), 5000);
        assert_eq!(
            records[records.len() - 2].ev,
            Ev::Scalar("null".into(), false)
        );
    }
    // A key of exactly the limit is not truncated.
    let exact = format!(r#"{{"{}":1}}"#, "k".repeat(4096));
    let (records, _) = lex([exact.as_bytes()]);
    assert_eq!(records[1].ev, Ev::Key("k".repeat(4096), false));
    // Tighter limits apply.
    let limits = LexerLimits {
        max_key_bytes: 2,
        max_scalar_bytes: 0,
        ..Default::default()
    };
    let (records, result) = lex_with(limits, [&br#"{"a\u00e9b":123}"#[..]]);
    assert_eq!(result, Ok(()));
    assert_eq!(records[1].ev, Ev::Key("a".into(), true));
    assert_eq!(records[2].ev, Ev::Scalar(String::new(), true));
}

#[test]
fn truncated_input_is_reported_as_truncated() {
    let text = r#" {"a":[1,-2.5e3,true,null,"x\n\ud83d\udd11é"],"b":{"c":"d"}} "#;
    let input = text.as_bytes();
    assert_eq!(verdict(input), Ok(()));
    let end = text.trim_end().len();
    for len in 0..end {
        let result = verdict(&input[..len]);
        if std::str::from_utf8(&input[..len]).is_err() {
            assert!(matches!(result, Err(LexError::InvalidUtf8 { .. })), "{len}");
        } else {
            assert_eq!(
                result,
                Err(LexError::Truncated { offset: len as u64 }),
                "{len}"
            );
        }
    }
    for text in [
        "", "  \n", "tru", "-", "1.", "2e", "\"abc", "[", "{\"a\"", "{\"a\":",
    ] {
        assert_eq!(
            verdict(text.as_bytes()),
            Err(LexError::Truncated {
                offset: text.len() as u64
            }),
            "{text:?}"
        );
    }
    // Root scalars end with the input.
    for text in ["0", "-12.5e+3", "true", " null ", "\"s\""] {
        assert_eq!(verdict(text.as_bytes()), Ok(()), "{text}");
        assert_eq!(
            rebuild(&merged(lex(text.as_bytes().chunks(1)).0)),
            serde_json::from_str::<Value>(text).unwrap()
        );
    }
}

#[test]
fn malformed_documents_are_rejected_at_the_offending_byte() {
    for (text, offset) in [
        (r#"{"a":1,}"#, 7),
        ("[1,]", 3),
        ("[1 2]", 3),
        (r#"{"a" 1}"#, 5),
        (r#"{"a":1 "b":2}"#, 7),
        ("{1:2}", 1),
        (r#"{"a":1]"#, 6),
        ("[1}", 2),
        ("]", 0),
        (",", 0),
        (":", 0),
        ("[:1]", 1),
        (r#"{"a"::1}"#, 5),
        (r#"{,"a":1}"#, 1),
        ("[,1]", 1),
        ("[01]", 2),
        ("[1.]", 3),
        ("[-]", 2),
        ("[+1]", 1),
        ("[.5]", 1),
        ("[1e]", 3),
        ("[1e+]", 4),
        ("[tru]", 4),
        ("[truee]", 5),
        ("[nul,1]", 4),
        ("[True]", 1),
        ("[1\"a\"]", 2),
        ("[\"a\"1]", 4),
        ("[\"a\tb\"]", 3),
        ("{} x", 3),
        ("1 2", 2),
        ("[] []", 3),
        ("\u{feff}{}", 0),
        ("'a'", 0),
        ("[NaN]", 1),
    ] {
        assert_eq!(
            verdict(text.as_bytes()),
            Err(LexError::Syntax { offset }),
            "{text:?}"
        );
        assert!(serde_json::from_str::<Value>(text).is_err(), "{text:?}");
    }
    let error = LexError::Syntax { offset: 4 };
    assert_eq!(error.offset(), 4);
    assert_eq!(error.to_string(), "invalid JSON at byte 4");
    assert_eq!(
        LexError::InvalidUtf8 { offset: 1 }.to_string(),
        "invalid UTF-8 at byte 1"
    );
    assert_eq!(
        LexError::Truncated { offset: 2 }.to_string(),
        "JSON document ends early at byte 2"
    );
}

/// The intended use: a head lexer annotates offsets, a tail lexer over the
/// same bytes rewrites one string and copies every other byte unchanged.
#[test]
fn a_tail_lexer_maps_edits_back_onto_the_original_bytes() {
    let text = "{ \"model\" : \"m\",\n \"input\":[{\"text\":\"keep \\u00e9\"},{\"text\":\"my \\\"secret\\\" 🔑\"}] ,\"n\":1.50}";
    let input = text.as_bytes();
    let target = [Step::Key("input"), Step::Index(1), Step::Key("text")];
    for size in [1, 3, 1000] {
        // Head: find the raw extent of the string content to replace.
        let mut head = Lexer::new();
        let mut extent: Option<Span> = None;
        for chunk in input.chunks(size) {
            head.push(chunk, |path, span, event| {
                if path.matches(&target) && matches!(event, Event::StringChunk { .. }) {
                    extent.get_or_insert(span).end = span.end;
                }
            })
            .unwrap();
        }
        head.finish(|_, _, _| {}).unwrap();
        let extent = extent.unwrap();

        // Tail: differently chunked, emits output from offsets alone.
        let mut tail = Lexer::new();
        let mut output = Vec::new();
        let mut copied = 0u64;
        for chunk in input.chunks(size + 2) {
            tail.push(chunk, |_, span, event| {
                if matches!(event, Event::StringChunk { .. }) && span.start == extent.start {
                    output.extend_from_slice(&input[copied as usize..span.start as usize]);
                    output.extend_from_slice(b"[REDACTED]");
                }
                if span.start >= extent.start && span.end <= extent.end {
                    copied = span.end;
                }
            })
            .unwrap();
            let settled = tail.settled().max(copied);
            output.extend_from_slice(&input[copied as usize..settled as usize]);
            copied = settled;
        }
        tail.finish(|_, _, _| {}).unwrap();
        output.extend_from_slice(&input[copied as usize..]);

        let expected = text.replace("my \\\"secret\\\" 🔑", "[REDACTED]");
        assert_eq!(String::from_utf8(output).unwrap(), expected, "size {size}");
    }
}

// ---------------------------------------------------------------------------
// MetadataSniffer
// ---------------------------------------------------------------------------

fn sniff_chunked(input: &[u8], size: usize) -> RequestMetadata {
    let mut lexer = Lexer::new();
    let mut sniffer = MetadataSniffer::new();
    for chunk in input.chunks(size) {
        lexer
            .push(chunk, |path, _, event| sniffer.observe(path, &event))
            .unwrap();
    }
    lexer
        .finish(|path, _, event| sniffer.observe(path, &event))
        .unwrap();
    sniffer.into_metadata()
}

fn sniffed(text: &str) -> RequestMetadata {
    let whole = sniff(text.as_bytes()).unwrap();
    for size in [1, 2, 5] {
        assert_eq!(sniff_chunked(text.as_bytes(), size), whole, "{text}");
    }
    whole
}

fn value_of<T: Clone>(value: T) -> Sniffed<T> {
    Sniffed::Value(value)
}

#[test]
fn sniffer_collects_every_field() {
    let body = json!({
        "model": "gpt-\"5\"\n",
        "stream": true,
        "type": "response.create",
        "previous_response_id": "resp_1",
        "client_metadata": {"session_id": "s", "nested": [1, {"a": null}], "q": "\"\\\u{1}é"},
        "metadata": {"other": {"user_id": "no"}, "user_id": "{\"session_id\":\"abc\"}", "session_id": "sid"},
        "extensions": {"localrouter": {"session": "x"}},
        "messages": [{"role": "user", "model": "nested", "tools": [1]}, {}, 3],
        "input": [{"type": "message"}],
        "tools": [{"name": "t"}, {"name": "u"}],
    });
    let meta = sniffed(&body.to_string());
    assert_eq!(meta.model, value_of("gpt-\"5\"\n".to_string()));
    assert_eq!(meta.stream, value_of(true));
    assert!(meta.is_stream());
    assert_eq!(meta.kind, value_of("response.create".to_string()));
    assert_eq!(meta.previous_response_id, value_of("resp_1".to_string()));
    assert_eq!(
        meta.client_metadata_json().unwrap(),
        body["client_metadata"]
    );
    // Compact text, escaped the way serde_json writes it.
    assert_eq!(
        meta.client_metadata.value().unwrap(),
        &serde_json::to_string(&body["client_metadata"]).unwrap()
    );
    assert_eq!(
        meta.metadata_user_id,
        value_of("{\"session_id\":\"abc\"}".to_string())
    );
    assert_eq!(meta.metadata_session_id, value_of("sid".to_string()));
    assert_eq!(meta.extensions_json().unwrap(), body["extensions"]);
    assert_eq!(meta.messages, Some(Shape::Array { len: 3 }));
    assert_eq!((meta.messages_len(), meta.input_len()), (3, 1));
    assert_eq!((meta.tools_len(), meta.has_tools()), (2, true));
    assert!(meta.complete && !meta.duplicate_keys);
    assert!(meta.model.is_settled() && !meta.model.is_overflow());
}

#[test]
fn sniffer_is_independent_of_member_order() {
    let members = [
        r#""model":"m""#,
        r#""stream":true"#,
        r#""messages":[{"role":"user","content":"hi"},{"content":[],"role":"assistant"}]"#,
        r#""tools":[{"name":"t"}]"#,
        r#""metadata":{"user_id":"u"}"#,
    ];
    let mut order: Vec<usize> = (0..members.len()).collect();
    let mut reference = None;
    let mut permutations = 0;
    // Heap's algorithm, iteratively.
    let mut counters = vec![0; order.len()];
    let mut index = 0;
    loop {
        let text = format!(
            "{{{}}}",
            order
                .iter()
                .map(|i| members[*i])
                .collect::<Vec<_>>()
                .join(",")
        );
        let meta = sniffed(&text);
        assert_eq!(meta.model, value_of("m".to_string()), "{text}");
        assert_eq!(
            *reference.get_or_insert_with(|| meta.clone()),
            meta,
            "{text}"
        );
        permutations += 1;
        while index < order.len() && counters[index] >= index {
            counters[index] = 0;
            index += 1;
        }
        if index == order.len() {
            break;
        }
        order.swap(if index % 2 == 0 { 0 } else { counters[index] }, index);
        counters[index] += 1;
        index = 0;
    }
    assert_eq!(permutations, 120);
}

#[test]
fn sniffer_reports_progress_while_the_body_streams() {
    let mut lexer = Lexer::new();
    let mut sniffer = MetadataSniffer::new();
    let mut feed = |sniffer: &mut MetadataSniffer, text: &str| {
        lexer
            .push(text.as_bytes(), |path, _, event| {
                sniffer.observe(path, &event)
            })
            .unwrap()
    };
    feed(&mut sniffer, r#"{"input":[{"model":"inner"},"#);
    assert_eq!(sniffer.metadata().model, Sniffed::Absent);
    assert_eq!(sniffer.metadata().input, None);
    assert!(!sniffer.metadata().model.is_settled());
    feed(&mut sniffer, r#"{}],"model""#);
    assert_eq!(sniffer.metadata().input, Some(Shape::Array { len: 2 }));
    assert_eq!(sniffer.metadata().model, Sniffed::Pending);
    feed(&mut sniffer, r#":"gpt"#);
    assert_eq!(sniffer.metadata().model, Sniffed::Pending);
    assert!(!sniffer.metadata().model.is_settled());
    feed(&mut sniffer, r#"-5","stream":false"#);
    assert_eq!(sniffer.metadata().model, value_of("gpt-5".to_string()));
    // A scalar is complete only at its delimiter.
    assert_eq!(sniffer.metadata().stream, Sniffed::Pending);
    assert!(!sniffer.metadata().complete);
    feed(&mut sniffer, "}");
    assert_eq!(sniffer.metadata().stream, value_of(false));
    assert!(sniffer.metadata().complete);
    assert_eq!(
        MetadataSniffer::default().metadata(),
        &RequestMetadata::default()
    );
}

#[test]
fn sniffer_reads_other_types_as_unset_and_last_duplicate_wins() {
    let meta = sniffed(
        r#"{"model":7,"stream":"true","type":null,"previous_response_id":{"a":"b"},
            "messages":"text","input":"text","tools":{"a":[1]},"metadata":["user_id"],
            "client_metadata":"plain","extensions":[1,"two",{"3":[]}]}"#,
    );
    assert_eq!(meta.model, Sniffed::Mismatch);
    assert_eq!(meta.stream, Sniffed::Mismatch);
    assert!(!meta.is_stream());
    assert_eq!(meta.kind, Sniffed::Mismatch);
    assert_eq!(meta.previous_response_id, Sniffed::Mismatch);
    assert_eq!(meta.messages, Some(Shape::String));
    assert_eq!((meta.messages_len(), meta.input_len()), (0, 1));
    assert_eq!(meta.tools, Some(Shape::Object));
    assert!(!meta.has_tools());
    assert_eq!(meta.metadata_user_id, Sniffed::Absent);
    assert_eq!(meta.client_metadata, value_of("\"plain\"".to_string()));
    assert_eq!(
        meta.extensions,
        value_of(r#"[1,"two",{"3":[]}]"#.to_string())
    );
    assert!(!meta.duplicate_keys);

    for (text, shape) in [
        (r#"{"tools":null}"#, Shape::Scalar),
        (r#"{"tools":[]}"#, Shape::Array { len: 0 }),
        (r#"{"tools":3}"#, Shape::Scalar),
    ] {
        let meta = sniffed(text);
        assert_eq!(meta.tools, Some(shape));
        assert!(!meta.has_tools());
    }
    for text in [
        r#"{"stream":1}"#,
        r#"{"stream":null}"#,
        r#"{"stream":[true]}"#,
        r#"{"stream":{}}"#,
    ] {
        assert_eq!(sniffed(text).stream, Sniffed::Mismatch, "{text}");
    }

    // Escaped key spellings name the same key; near misses do not.
    let meta = sniffed(r#"{"\u006dodel":"a","Model":"b","model ":"c","stream\u0000":true}"#);
    assert_eq!(meta.model, value_of("a".to_string()));
    assert_eq!(meta.stream, Sniffed::Absent);

    // Repeats: the last occurrence wins and the repeat is flagged.
    let meta = sniffed(
        r#"{"model":"first","messages":[1,2],"metadata":{"user_id":"u1","session_id":"s"},
            "tools":[1],"stream":true,"client_metadata":{"a":1},
            "model":"second","messages":[1],"metadata":{"user_id":"a","user_id":"u2"},
            "tools":"none","stream":0,"client_metadata":{"b":2}}"#,
    );
    assert_eq!(meta.model, value_of("second".to_string()));
    assert_eq!(meta.messages_len(), 1);
    assert_eq!(meta.metadata_user_id, value_of("u2".to_string()));
    assert_eq!(meta.metadata_session_id, Sniffed::Absent);
    assert!(!meta.has_tools());
    assert_eq!(meta.stream, Sniffed::Mismatch);
    assert_eq!(meta.client_metadata, value_of(r#"{"b":2}"#.to_string()));
    assert!(meta.duplicate_keys);
    for text in [
        r#"{"model":"a","model":1}"#,
        r#"{"input":[],"input":[]}"#,
        r#"{"metadata":1,"metadata":2}"#,
        r#"{"metadata":{"session_id":"a","session_id":"b"}}"#,
        r#"{"extensions":{},"extensions":{}}"#,
        r#"{"type":"a","type":"b"}"#,
    ] {
        assert!(sniffed(text).duplicate_keys, "{text}");
    }
    assert!(!sniffed(r#"{"a":{"model":1,"model":2},"x":1,"x":2}"#).duplicate_keys);

    // Roots that are not objects carry no fields.
    for text in [r#"[{"model":"m"}]"#, r#""model""#, "12", r#"[["model"]]"#] {
        let meta = sniffed(text);
        assert_eq!(
            meta,
            RequestMetadata {
                complete: true,
                ..Default::default()
            },
            "{text}"
        );
    }
}

#[test]
fn sniffer_bounds_every_capture() {
    let run = |limit: usize, text: &str| {
        let mut lexer = Lexer::new();
        let mut sniffer = MetadataSniffer::with_limit(limit);
        for chunk in text.as_bytes().chunks(3) {
            lexer
                .push(chunk, |path, _, event| sniffer.observe(path, &event))
                .unwrap();
        }
        lexer
            .finish(|path, _, event| sniffer.observe(path, &event))
            .unwrap();
        sniffer.into_metadata()
    };
    // Exactly at the bound is kept; one byte more is dropped and flagged, and
    // the fields after it are still collected.
    let meta = run(
        8,
        r#"{"model":"12345678","type":"123456789","stream":true}"#,
    );
    assert_eq!(meta.model, value_of("12345678".to_string()));
    assert_eq!(meta.kind, Sniffed::Overflow);
    assert!(meta.kind.is_overflow() && meta.kind.is_settled());
    assert_eq!(meta.stream, value_of(true));

    let meta = run(
        8,
        r#"{"client_metadata":{"a":12},"extensions":{"a":123},"metadata":{"user_id":"123456789","session_id":"s"},"model":"m"}"#,
    );
    assert_eq!(meta.client_metadata, value_of(r#"{"a":12}"#.to_string()));
    assert_eq!(meta.extensions, Sniffed::Overflow);
    assert_eq!(meta.metadata_user_id, Sniffed::Overflow);
    assert_eq!(meta.metadata_session_id, value_of("s".to_string()));
    assert_eq!(meta.model, value_of("m".to_string()));

    // Escaping counts against the bound.
    assert_eq!(
        run(8, r#"{"extensions":"\n\n\n"}"#).extensions,
        value_of(r#""\n\n\n""#.to_string())
    );
    assert_eq!(
        run(8, r#"{"extensions":"\n\n\n\n"}"#).extensions,
        Sniffed::Overflow
    );
    assert_eq!(
        run(8, r#"{"extensions":[[[[[[[[[]]]]]]]]]}"#).extensions,
        Sniffed::Overflow
    );

    // The default bound is 64 KiB per field.
    let big = "x".repeat(DEFAULT_FIELD_LIMIT);
    let meta = sniffed(&format!(
        r#"{{"model":"{big}","previous_response_id":"{big}y","extensions":{{"k":"{big}"}}}}"#
    ));
    assert_eq!(
        meta.model.value().map(String::len),
        Some(DEFAULT_FIELD_LIMIT)
    );
    assert_eq!(meta.previous_response_id, Sniffed::Overflow);
    assert_eq!(meta.extensions, Sniffed::Overflow);

    // Subtrees beyond the lexer's limits cannot be captured faithfully.
    let sniff_limited = |limits: LexerLimits, text: &str| {
        let mut lexer = Lexer::with_limits(limits);
        let mut sniffer = MetadataSniffer::new();
        lexer
            .push(text.as_bytes(), |path, _, event| {
                sniffer.observe(path, &event)
            })
            .unwrap();
        lexer
            .finish(|path, _, event| sniffer.observe(path, &event))
            .unwrap();
        sniffer.into_metadata()
    };
    let meta = sniff_limited(
        LexerLimits {
            max_depth: 2,
            ..Default::default()
        },
        r#"{"extensions":{"a":{"deep":1}},"messages":[[1],[2]],"model":"m"}"#,
    );
    assert_eq!(meta.extensions, Sniffed::Overflow);
    assert_eq!(meta.messages, Some(Shape::Array { len: 2 }));
    assert_eq!(meta.model, value_of("m".to_string()));
    let meta = sniff_limited(
        LexerLimits {
            max_key_bytes: 16,
            max_scalar_bytes: 4,
            ..Default::default()
        },
        r#"{"client_metadata":{"a_key_longer_than_sixteen":1},"extensions":{"n":123456},"previous_response_id":"p","model":"m"}"#,
    );
    assert_eq!(meta.client_metadata, Sniffed::Overflow);
    assert_eq!(meta.extensions, Sniffed::Overflow);
    // A key too long to compare is not one of the tracked keys.
    assert_eq!(meta.previous_response_id, Sniffed::Absent);
    assert_eq!(meta.model, value_of("m".to_string()));
    let meta = sniff_limited(
        LexerLimits {
            max_key_bytes: 5,
            ..Default::default()
        },
        r#"{"modelx":"n","metadata":{"user_idx":"u"}}"#,
    );
    assert_eq!(meta.model, Sniffed::Absent);
    let limits = LexerLimits {
        max_depth: 1,
        ..Default::default()
    };
    let mut lexer = Lexer::with_limits(limits);
    let mut sniffer = MetadataSniffer::new();
    lexer
        .push(
            br#"{"messages":[1,2],"model":{"a":1},"stream":[],"metadata":{"user_id":"u"}}"#,
            |path, _, event| sniffer.observe(path, &event),
        )
        .unwrap();
    assert_eq!(sniffer.metadata().messages, Some(Shape::Opaque));
    assert_eq!(sniffer.metadata().messages_len(), 0);
    assert_eq!(sniffer.metadata().model, Sniffed::Mismatch);
    assert_eq!(sniffer.metadata().stream, Sniffed::Mismatch);
    assert_eq!(sniffer.metadata().metadata_user_id, Sniffed::Absent);
}

#[test]
fn sniffer_captures_random_subtrees_exactly() {
    for seed in 0..300 {
        let mut rng = Rng::new(seed ^ 0x5EED);
        let (mut first, mut second) = (String::new(), String::new());
        value(&mut rng, 1, &mut first);
        value(&mut rng, 1, &mut second);
        let text =
            format!(r#"{{"a":[1],"client_metadata":{first},"b":{{}},"extensions":{second}}}"#);
        let expected: Value = serde_json::from_str(&text).unwrap();
        let meta = sniffed(&text);
        assert_eq!(
            meta.client_metadata_json().as_ref(),
            Some(&expected["client_metadata"]),
            "seed {seed}: {text}"
        );
        assert_eq!(
            meta.extensions_json().as_ref(),
            Some(&expected["extensions"]),
            "seed {seed}: {text}"
        );
    }
}
