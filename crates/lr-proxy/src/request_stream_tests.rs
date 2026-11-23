//! Unit tests of the request pipeline against a mock session.

use super::*;
use serde_json::json;
use std::collections::VecDeque;

/// A session that detects `token=<value>` assignments, registers the value,
/// and masks every registered value in every string (as the vault does).
#[derive(Default)]
struct Mock {
    known: Vec<String>,
    inspected: usize,
    diagnostics: Vec<String>,
    finished: bool,
    /// Drop the top-level `extensions` member.
    drop_extensions: bool,
}

fn strings(value: &Value, out: &mut Vec<String>) {
    match value {
        Value::String(s) => out.push(s.clone()),
        Value::Array(items) => items.iter().for_each(|v| strings(v, out)),
        Value::Object(map) => map.values().for_each(|v| strings(v, out)),
        _ => {}
    }
}

fn mask(value: &mut Value, known: &[String]) {
    match value {
        Value::String(s) => {
            for secret in known {
                *s = s.replace(secret.as_str(), "[MASKED]");
            }
        }
        Value::Array(items) => items.iter_mut().for_each(|v| mask(v, known)),
        Value::Object(map) => map.values_mut().for_each(|v| mask(v, known)),
        _ => {}
    }
}

impl StreamSession for Mock {
    fn inspect(&mut self, _: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        self.inspected += 1;
        let mut texts = Vec::new();
        strings(value, &mut texts);
        let mut added = Vec::new();
        for text in texts {
            for (i, _) in text.match_indices("token=") {
                let secret: String = text[i + 6..]
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if secret.len() >= 8 && !self.known.contains(&secret) {
                    self.known.push(secret.clone());
                    added.push(secret);
                }
            }
        }
        Ok(added)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        for unit in units {
            if self.drop_extensions && unit.scope.key() == "extensions" {
                unit.drop = true;
            }
            if !matches!(&unit.scope, UnitScope::RootMember { key } if key == "model") {
                mask(&mut unit.value, &self.known);
            }
        }
        Ok(())
    }
    fn diagnostic(&mut self, message: &str) {
        self.diagnostics.push(message.to_owned());
    }
    fn finish(&mut self, _: &RequestMetadata) {
        self.finished = true;
    }
}

struct Run {
    out: Vec<u8>,
    opened_at: Option<u64>,
    result: Result<Option<StreamSummary>, StreamAbort>,
}

fn run(body: &[u8], lag: usize, chunk: usize, session: &mut Mock) -> Run {
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut out = Vec::new();
    let mut opened_at = None;
    let mut fed = 0u64;
    let mut result = Ok(None);
    for piece in body.chunks(chunk.max(1)) {
        fed += piece.len() as u64;
        let pushed = if opened_at.is_some() {
            pipeline.push(piece, Some(&mut *session), &mut out)
        } else {
            pipeline.push(piece, None, &mut out)
        };
        if let Err(abort) = pushed {
            result = Err(abort);
            break;
        }
        if opened_at.is_none() && pipeline.ready_to_open() {
            pipeline.open();
            opened_at = Some(fed);
            if let Err(abort) = pipeline.push(&[], Some(&mut *session), &mut out) {
                result = Err(abort);
                break;
            }
        }
    }
    if result.is_ok() && opened_at.is_some() {
        result = pipeline.finish(session, &mut out).map(Some);
    }
    Run {
        out,
        opened_at,
        result,
    }
}

fn conversation(messages: usize, filler: usize) -> Value {
    json!({
        "model": "gpt-test",
        "instructions": "Be brief.",
        "input": (0..messages)
            .map(|i| json!({"type":"message","role":"user","content":[{"type":"input_text","text":format!("message {i} {}", "x".repeat(filler))}]}))
            .collect::<Vec<_>>(),
        "tools": [{"type":"function","name":"f","parameters":{"type":"object"}}],
        "stream": true,
    })
}

/// Compact JSON with `model` first, whatever map order `serde_json` uses in
/// this build (key order is preserved only when a dependency enables it).
fn encode(body: &Value) -> Vec<u8> {
    let map = body.as_object().unwrap();
    let mut keys: Vec<&String> = map.keys().collect();
    keys.sort_by_key(|k| *k != "model");
    let members: Vec<String> = keys
        .iter()
        .map(|k| format!("{}:{}", serde_json::to_string(k).unwrap(), map[*k]))
        .collect();
    format!("{{{}}}", members.join(",")).into_bytes()
}

#[test]
fn unchanged_compact_bodies_are_forwarded_byte_for_byte_at_every_chunking() {
    let body = encode(&conversation(6, 40));
    for chunk in [1, 2, 3, 7, 64, 1000, body.len()] {
        let mut session = Mock::default();
        let r = run(&body, 64, chunk, &mut session);
        assert!(r.opened_at.is_some(), "chunk {chunk}");
        assert_eq!(r.result.as_ref().map(|_| ()), Ok(()), "chunk {chunk}");
        assert_eq!(r.out, body, "chunk {chunk}");
        assert!(session.finished);
        // Every unit was inspected once: 4 members + 6 elements.
        assert_eq!(session.inspected, 4 + 6, "chunk {chunk}");
    }
}

#[test]
fn whitespace_between_members_is_normalized_but_values_keep_their_spelling() {
    let body = "{ \"model\" : \"m\",\n        \"input\" : [ {\"role\":\"user\",\"content\":\"caf\\u00e9 \\\"q\\\" 1.50e3\"} , 7 ],\n        \"n\": 1.0 }";
    let mut session = Mock::default();
    let r = run(body.as_bytes(), 8, 5, &mut session);
    assert!(r.opened_at.is_some());
    r.result.unwrap();
    assert_eq!(
        String::from_utf8(r.out).unwrap(),
        "{\"model\":\"m\",\"input\":[{\"role\":\"user\",\"content\":\"caf\\u00e9 \\\"q\\\" 1.50e3\"},7],\"n\":1.0}"
    );
}

#[test]
fn bodies_within_the_lag_never_open() {
    let body = encode(&conversation(4, 10));
    let mut session = Mock::default();
    let r = run(&body, body.len(), 1, &mut session);
    assert_eq!(r.opened_at, None);
    assert!(r.out.is_empty());
    assert_eq!(session.inspected, 0);
}

#[test]
fn a_secret_detected_within_the_lag_masks_its_earlier_occurrences() {
    let secret = "Zq81RkLmPw0Xy7Tb";
    let mut body = conversation(30, 200);
    // A bare occurrence first; it only becomes detectable later.
    body["input"][2]["content"][0]["text"] = json!(format!("the value {secret} appears"));
    body["input"][20]["content"][0]["text"] = json!(format!("token={secret}"));
    let bytes = encode(&body);
    let distance = bytes.len() / 2;

    // Lag above the distance: nothing unmasked leaves.
    let mut session = Mock::default();
    let r = run(&bytes, distance + 4096, 97, &mut session);
    assert!(r.opened_at.is_some());
    r.result.unwrap();
    let text = String::from_utf8(r.out.clone()).unwrap();
    assert!(!text.contains(secret), "{text}");
    let forwarded: Value = serde_json::from_slice(&r.out).unwrap();
    let mut expected = body.clone();
    mask(&mut expected, &[secret.to_owned()]);
    expected["model"] = body["model"].clone();
    assert_eq!(forwarded, expected);
    assert!(session.diagnostics.is_empty());

    // Lag below it: the earlier occurrence was already released, which is
    // reported; the detected one is masked.
    let mut session = Mock::default();
    let r = run(&bytes, 512, 97, &mut session);
    r.result.unwrap();
    let text = String::from_utf8(r.out).unwrap();
    assert_eq!(text.matches(secret).count(), 1);
    assert!(text.contains("token=[MASKED]"));
    assert_eq!(session.diagnostics, vec![EARLIER_OCCURRENCE.to_owned()]);
}

#[test]
fn a_secret_split_across_chunks_and_units_is_masked() {
    let secret = "Ab12Cd34Ef56Gh78";
    let mut body = conversation(10, 50);
    body["input"][5]["content"][0]["text"] = json!(format!("token={secret} and again {secret}"));
    body["input"][6]["content"][0]["text"] = json!(format!("repeat {secret}"));
    let bytes = encode(&body);
    for chunk in [1, 5, 13, 4096] {
        let mut session = Mock::default();
        let r = run(&bytes, 256, chunk, &mut session);
        r.result.unwrap();
        let text = String::from_utf8(r.out).unwrap();
        assert!(!text.contains(secret), "chunk {chunk}");
        assert_eq!(text.matches("[MASKED]").count(), 3, "chunk {chunk}");
    }
}

#[test]
fn nothing_is_released_while_an_incomplete_unit_is_within_reach() {
    let mut body = conversation(4, 10);
    // One element far larger than the lag.
    body["input"][2]["content"][0]["text"] = json!("y".repeat(200_000));
    let bytes = encode(&body);
    let big_end = bytes.windows(10).rposition(|w| w == b"yyyyyyyyyy").unwrap();
    let mut pipeline = RequestPipeline::new(1024, MAX_HOLD);
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut opened = false;
    for (i, piece) in bytes.chunks(1000).enumerate() {
        let fed = (i + 1) * 1000;
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
        if fed < big_end {
            // The element before the big one must wait for it: its end plus
            // the lag lies inside the unfinished element.
            let text = String::from_utf8_lossy(&out);
            assert!(!text.contains("message 1 "), "released at {fed}");
        }
    }
    pipeline.finish(&mut session, &mut out).unwrap();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), body);
    assert!(pipeline.max_held() >= 200_000);
}

#[test]
fn the_tail_stays_the_lag_behind_the_head() {
    let body = encode(&conversation(400, 100));
    let lag = 8 * 1024;
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut opened = false;
    let mut fed = 0usize;
    for piece in body.chunks(333) {
        fed += piece.len();
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
        // Released bytes are an unchanged prefix of the input, at least the
        // lag behind what arrived.
        assert_eq!(&body[..out.len()], &out[..]);
        assert!(out.len() + lag <= fed || out.is_empty());
    }
    assert!(!out.is_empty());
    assert!(pipeline.max_held() < lag + 2 * 1024 + 333);
    pipeline.finish(&mut session, &mut out).unwrap();
    assert_eq!(out, body);
}

#[test]
fn bodies_whose_treatment_is_not_known_early_are_never_opened() {
    let long = "z".repeat(2000);
    let cases: Vec<(&str, Vec<u8>)> = vec![
        (
            "model not a string",
            serde_json::to_vec(&json!({"model":7,"input":[long, long]})).unwrap(),
        ),
        (
            "not an object",
            serde_json::to_vec(&json!([{"model":"m"}, long, long])).unwrap(),
        ),
        (
            "duplicate key",
            format!(r#"{{"model":"m","model":"n","input":["{long}","{long}"]}}"#).into_bytes(),
        ),
        (
            "whole-document member",
            serde_json::to_vec(&json!({"model":"m","commands":{"q":1},"input":[long, long]}))
                .unwrap(),
        ),
        (
            "invalid JSON",
            format!(r#"{{"model":"m" "input":["{long}","{long}"]}}"#).into_bytes(),
        ),
    ];
    for (name, body) in cases {
        let mut session = Mock::default();
        let r = run(&body, 100, 50, &mut session);
        assert_eq!(r.opened_at, None, "{name}");
        assert!(r.out.is_empty(), "{name}");
    }
}

#[test]
fn a_late_model_opens_the_stream_only_once_it_arrived() {
    let long = "z".repeat(2000);
    let body = format!(r#"{{"input":["{long}","{long}"],"model":"m"}}"#);
    let mut session = Mock::default();
    let r = run(body.as_bytes(), 100, 50, &mut session);
    let model_end = body.find(r#""model":"m""#).unwrap() + 11;
    assert!(r.opened_at.unwrap() >= model_end as u64);
    r.result.unwrap();
    assert_eq!(
        serde_json::from_slice::<Value>(&r.out).unwrap(),
        serde_json::from_str::<Value>(&body).unwrap()
    );
}

#[test]
fn problems_after_release_abort_the_stream() {
    let long = "z".repeat(2000);
    for (name, body) in [
        (
            "duplicate key",
            format!(r#"{{"model":"m","input":["{long}","{long}"],"model":"other"}}"#),
        ),
        (
            "whole-document member",
            format!(r#"{{"model":"m","input":["{long}","{long}"],"signature":"s"}}"#),
        ),
        (
            "invalid JSON",
            format!(r#"{{"model":"m","input":["{long}","{long}"] x"#),
        ),
        (
            "truncated",
            format!(r#"{{"model":"m","input":["{long}","{long}"]"#),
        ),
    ] {
        let mut session = Mock::default();
        let r = run(body.as_bytes(), 100, 50, &mut session);
        assert!(r.opened_at.is_some(), "{name}");
        let abort = r.result.unwrap_err();
        assert_eq!(abort.status, 400, "{name}");
        assert!(!session.finished, "{name}");
    }
}

#[test]
fn dropped_members_keep_the_document_valid() {
    for position in ["first", "middle", "last"] {
        let long = "w".repeat(3000);
        let body = match position {
            "first" => format!(r#"{{"extensions":{{"a":1}},"model":"m","input":["{long}"]}}"#),
            "middle" => {
                format!(r#"{{"model":"m","extensions":{{"a":1}},"input":["{long}"],"n":1}}"#)
            }
            _ => format!(r#"{{"model":"m","input":["{long}"],"extensions":{{"a":1}}}}"#),
        };
        let mut session = Mock {
            drop_extensions: true,
            ..Default::default()
        };
        let r = run(body.as_bytes(), 100, 64, &mut session);
        r.result.unwrap();
        let forwarded: Value = serde_json::from_slice(&r.out).unwrap();
        assert!(forwarded.get("extensions").is_none(), "{position}");
        assert_eq!(forwarded["input"][0].as_str().unwrap().len(), 3000);
    }
}

#[test]
fn the_hold_cap_aborts_an_open_stream_and_gives_up_before_one() {
    let long = "v".repeat(10_000);
    let body = format!(r#"{{"model":"m","input":["{long}","{long}","{long}"]}}"#);
    let mut pipeline = RequestPipeline::new(100, 5_000);
    let mut out = Vec::new();
    // Not yet open: the pipeline gives up and the buffered path decides.
    pipeline.push(body.as_bytes(), None, &mut out).unwrap();
    assert!(pipeline.unstreamable().is_some());
    assert!(!pipeline.ready_to_open());

    let mut pipeline = RequestPipeline::new(100, 15_000);
    let mut session = Mock::default();
    pipeline
        .push(&body.as_bytes()[..400], None, &mut out)
        .unwrap();
    assert!(pipeline.ready_to_open());
    pipeline.open();
    let abort = pipeline
        .push(&body.as_bytes()[400..], Some(&mut session), &mut out)
        .unwrap_err();
    assert_eq!(abort.status, 413);
}

#[test]
fn correlation_parts_match_the_whole_body() {
    let mut body = conversation(12, 30);
    body["input"][3] = json!({"type":"function_call","name":"f","arguments":"{}","call_id":"c"});
    body["input"][4] = json!({"role":"assistant","content":"done"});
    body["extensions"] = json!({"localrouter.session":{"id":"conv-1","reset":false}});
    body["previous_response_id"] = json!("resp_1");
    let bytes = encode(&body);
    let mut session = Mock::default();
    let r = run(&bytes, 128, 41, &mut session);
    let summary = r.result.unwrap().unwrap();
    let whole = lr_monitor::CorrelationParts::from_body(&body);
    assert_eq!(format!("{:?}", summary.correlation), format!("{whole:?}"));

    // `messages` decides over `input`, even when it is not an array.
    let body = json!({"model":"m","messages":"text","input":["a".repeat(500)]});
    let bytes = encode(&body);
    let mut session = Mock::default();
    let r = run(&bytes, 64, 17, &mut session);
    let summary = r.result.unwrap().unwrap();
    let whole = lr_monitor::CorrelationParts::from_body(&body);
    assert_eq!(format!("{:?}", summary.correlation), format!("{whole:?}"));
}

#[test]
fn fingerprints_recognize_released_tokens_only() {
    let mut prints = Fingerprints::new();
    prints.add(br#"{"text":"the key Qw3rTy9Zx1 is here","n":"short"}"#);
    assert!(prints.contains("Qw3rTy9Zx1"));
    assert!(prints.contains("prefix \"Qw3rTy9Zx1\""));
    assert!(!prints.contains("Zx1Qw3rTy9"));
    assert!(!prints.contains("short"));
}

/// The memory bound of plan §6: a body far above the lag is forwarded while
/// holding at most the lag plus one unit and one read.
#[test]
fn memory_stays_bounded_by_the_lag_for_a_large_body() {
    let lag = 16 * 1024 * 1024;
    let element = 64 * 1024;
    let total = std::env::var("LR_STREAM_BOUND_MIB")
        .ok()
        .and_then(|v| v.parse::<usize>().ok())
        .unwrap_or(500)
        * 1024
        * 1024;
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut forwarded = 0usize;
    let mut fed = 0usize;
    let mut push = |pipeline: &mut RequestPipeline, data: &[u8], session: &mut Mock, open: bool| {
        if open {
            pipeline.push(data, Some(session), &mut out).unwrap();
        } else {
            pipeline.push(data, None, &mut out).unwrap();
        }
        forwarded += out.len();
        out.clear();
    };
    let mut open = false;
    let item = format!(
        r#"{{"role":"user","content":"{}"}}"#,
        "m".repeat(element - 32)
    );
    push(
        &mut pipeline,
        br#"{"model":"m","input":["#,
        &mut session,
        open,
    );
    let mut first = true;
    while fed < total {
        let mut chunk = Vec::with_capacity(STEP + element);
        while chunk.len() < STEP {
            if !first {
                chunk.push(b',');
            }
            first = false;
            chunk.extend_from_slice(item.as_bytes());
        }
        fed += chunk.len();
        push(&mut pipeline, &chunk, &mut session, open);
        if !open && pipeline.ready_to_open() {
            pipeline.open();
            open = true;
            push(&mut pipeline, &[], &mut session, open);
        }
    }
    push(&mut pipeline, b"]}", &mut session, open);
    assert!(open);
    assert!(forwarded > 0);
    let max = pipeline.max_held();
    assert!(
        max <= lag + element + STEP + element,
        "held {max} bytes for a {fed}-byte body"
    );
    eprintln!("memory bound: {fed} bytes forwarded with at most {max} bytes held (lag {lag})");
}

/// A session that asks about every `ask=<value>` it detects; the answer
/// (a bool) registers the value for masking (true) or allows it (false).
#[derive(Default)]
struct Asking {
    inner: Mock,
    queued: VecDeque<String>,
    asking: Option<String>,
    asked: Vec<String>,
}

impl StreamSession for Asking {
    fn inspect(&mut self, scope: &UnitScope, value: &Value) -> Result<Vec<String>, StreamAbort> {
        let mut texts = Vec::new();
        strings(value, &mut texts);
        for text in texts {
            for (i, _) in text.match_indices("ask=") {
                let value: String = text[i + 4..]
                    .chars()
                    .take_while(char::is_ascii_alphanumeric)
                    .collect();
                if !self.asked.contains(&value) && !self.queued.contains(&value) {
                    self.queued.push_back(value);
                }
            }
        }
        self.inner.inspect(scope, value)
    }
    fn mask(&mut self, units: &mut [TailUnit]) -> Result<(), StreamAbort> {
        self.inner.mask(units)
    }
    fn diagnostic(&mut self, message: &str) {
        self.inner.diagnostic(message);
    }
    fn finish(&mut self, metadata: &RequestMetadata) {
        self.inner.finish(metadata);
    }
    fn pending(&self) -> bool {
        self.asking.is_some() || !self.queued.is_empty()
    }
    fn question(&mut self) -> Option<Question> {
        if self.asking.is_some() {
            return None;
        }
        let value = self.queued.pop_front()?;
        self.asked.push(value.clone());
        self.asking = Some(value);
        Some(Question(Box::pin(async { Answer(Box::new(true)) })))
    }
    fn answer(&mut self, answer: Answer) -> Result<Vec<String>, StreamAbort> {
        let value = self.asking.take().expect("a question was open");
        if value == "deny" {
            return Err(StreamAbort::new(403, "denied"));
        }
        if *answer.0.downcast::<bool>().unwrap() {
            self.inner.known.push(value.clone());
            return Ok(vec![value]);
        }
        Ok(Vec::new())
    }
}

/// Answer every open question with `true`, releasing after each answer.
fn answer_all(
    pipeline: &mut RequestPipeline,
    session: &mut Asking,
    out: &mut Vec<u8>,
    raised: &mut Vec<usize>,
) -> Result<(), StreamAbort> {
    while let Some(question) = session.question() {
        assert!(pipeline.gated());
        raised.push(out.len());
        let answer = futures::executor::block_on(question.0);
        let registered = session.answer(answer)?;
        pipeline.registered(&registered, session);
        pipeline.push(&[], Some(session), out)?;
    }
    Ok(())
}

/// Feed `body` in `chunk`s, answering every question as soon as it is
/// raised; returns the output and the output length at each question.
fn run_asking(
    body: &[u8],
    lag: usize,
    chunk: usize,
    session: &mut Asking,
) -> Result<(Vec<u8>, Vec<usize>), StreamAbort> {
    let mut pipeline = RequestPipeline::new(lag, MAX_HOLD);
    let mut out = Vec::new();
    let mut raised = Vec::new();
    let mut opened = false;
    for piece in body.chunks(chunk.max(1)) {
        if opened {
            pipeline.push(piece, Some(&mut *session), &mut out)?;
        } else {
            pipeline.push(piece, None, &mut out)?;
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut *session), &mut out)?;
            }
        }
        if opened {
            answer_all(&mut pipeline, session, &mut out, &mut raised)?;
        }
    }
    assert!(opened);
    pipeline.push(&[], Some(&mut *session), &mut out)?;
    answer_all(&mut pipeline, session, &mut out, &mut raised)?;
    assert!(!pipeline.gated());
    pipeline.finish(session, &mut out)?;
    Ok((out, raised))
}

#[test]
fn a_question_stops_the_release_at_its_unit_until_it_is_answered() {
    let mut body = conversation(60, 200);
    // A bare occurrence first, then the finding that asks about it.
    body["input"][10]["content"][0]["text"] = json!("bare Qq11Ww22Ee33Rr44 here");
    body["input"][12]["content"][0]["text"] = json!("ask=Qq11Ww22Ee33Rr44");
    body["input"][40]["content"][0]["text"] = json!("ask=Tt55Yy66Uu77Ii88 and Qq11Ww22Ee33Rr44");
    let bytes = encode(&body);
    let unit_12 = serde_json::to_string(&body["input"][12]).unwrap();
    let at = bytes
        .windows(unit_12.len())
        .position(|w| w == unit_12.as_bytes())
        .unwrap();
    for chunk in [1, 97, 4096] {
        let mut session = Asking::default();
        let (out, raised) = run_asking(&bytes, 4096, chunk, &mut session).unwrap();
        let text = String::from_utf8(out.clone()).unwrap();
        assert!(!text.contains("Qq11Ww22Ee33Rr44"), "chunk {chunk}");
        assert!(!text.contains("Tt55Yy66Uu77Ii88"), "chunk {chunk}");
        assert_eq!(session.asked, vec!["Qq11Ww22Ee33Rr44", "Tt55Yy66Uu77Ii88"]);
        assert_eq!(raised.len(), 2);
        // When the first question was raised, nothing from unit 12 on had
        // been released.
        assert!(raised[0] <= at, "chunk {chunk}: {} > {at}", raised[0]);
        let mut expected = body.clone();
        mask(
            &mut expected,
            &["Qq11Ww22Ee33Rr44".into(), "Tt55Yy66Uu77Ii88".into()],
        );
        assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), expected);
    }
}

#[test]
fn a_denial_ends_the_stream_before_the_denied_unit() {
    let mut body = conversation(60, 200);
    body["input"][30]["content"][0]["text"] = json!("ask=deny secret-after-this");
    let bytes = encode(&body);
    let mut session = Asking::default();
    let mut pipeline = RequestPipeline::new(2048, MAX_HOLD);
    let mut out = Vec::new();
    let mut denied = None;
    let mut opened = false;
    for piece in bytes.chunks(500) {
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
        if let Some(question) = session.question() {
            assert!(pipeline.gated());
            let answer = futures::executor::block_on(question.0);
            if let Err(abort) = session.answer(answer) {
                denied = Some(abort);
                break;
            }
        }
    }
    assert_eq!(denied.unwrap().status, 403);
    let text = String::from_utf8_lossy(&out);
    assert!(!text.is_empty(), "earlier content was released");
    assert!(!text.contains("secret-after-this"));
    assert!(!text.contains("message 30 "));
    assert!(!session.inner.finished);
}

#[test]
fn strict_mode_releases_nothing_before_the_end() {
    let body = encode(&conversation(200, 300));
    let mut pipeline = RequestPipeline::new(1024, MAX_HOLD);
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut opened = false;
    for piece in body.chunks(777) {
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.hold_until_end();
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
        assert!(out.is_empty(), "released before the end");
    }
    // Every complete unit was inspected while held (the last member,
    // `stream`, completes with the closing brace).
    assert!(session.inspected >= 3 + 200, "{}", session.inspected);
    pipeline.finish(&mut session, &mut out).unwrap();
    assert_eq!(out, body);
}

#[test]
fn inspect_only_observes_every_unit_and_emits_nothing() {
    let mut body = conversation(50, 100);
    body["input"][7]["content"][0]["text"] = json!("token=Zz99Xx88Cc77Vv66");
    body["extensions"] = json!({"localrouter.session":{"id":"c1"}});
    let bytes = encode(&body);
    let mut pipeline = RequestPipeline::new(512, MAX_HOLD);
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut opened = false;
    for piece in bytes.chunks(301) {
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.inspect_only();
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
        assert!(out.is_empty());
    }
    let summary = pipeline.finish(&mut session, &mut out).unwrap();
    assert!(out.is_empty());
    assert_eq!(session.known, vec!["Zz99Xx88Cc77Vv66".to_owned()]);
    let whole = lr_monitor::CorrelationParts::from_body(&body);
    assert_eq!(format!("{:?}", summary.correlation), format!("{whole:?}"));
}

#[test]
fn a_repeated_nested_key_makes_the_body_unstreamable_and_aborts_an_open_stream() {
    let long = "q".repeat(3000);
    let dup = format!(
        r#"{{"model":"m","metadata":{{"user_id":"a","user_id":"b"}},"input":["{long}","{long}"]}}"#
    );
    let mut pipeline = RequestPipeline::new(100, MAX_HOLD);
    let mut out = Vec::new();
    pipeline.push(dup.as_bytes(), None, &mut out).unwrap();
    assert!(pipeline.unstreamable().is_some());
    assert!(!pipeline.ready_to_open());

    let late = format!(
        r#"{{"model":"m","input":["{long}","{long}"],"metadata":{{"user_id":"a","user_id":"b"}}}}"#
    );
    let mut session = Mock::default();
    let r = run(late.as_bytes(), 100, 64, &mut session);
    assert!(r.opened_at.is_some());
    assert_eq!(r.result.unwrap_err().status, 400);
    assert!(!session.finished);
}

#[test]
fn a_spilling_hold_forwards_the_same_bytes() {
    let dir = std::env::temp_dir().join(format!("lr-pipeline-spill-{}", uuid::Uuid::new_v4()));
    let config = std::sync::Arc::new(crate::spill::SpillConfig::with_limits(
        dir.clone(),
        crate::spill::CHUNK,
        1 << 30,
        1 << 30,
    ));
    let mut body = conversation(300, 900);
    body["input"][250]["content"][0]["text"] = json!("token=Pp00Oo99Ii88Uu77");
    body["input"][3]["content"][0]["text"] = json!("early Pp00Oo99Ii88Uu77");
    let bytes = encode(&body);
    let lag = bytes.len() - 1000;
    let mut pipeline =
        RequestPipeline::with_hold(lag, crate::spill::SpillBuffer::new(config.clone()));
    let mut session = Mock::default();
    let mut out = Vec::new();
    let mut opened = false;
    for piece in bytes.chunks(4096) {
        if opened {
            pipeline.push(piece, Some(&mut session), &mut out).unwrap();
        } else {
            pipeline.push(piece, None, &mut out).unwrap();
            if pipeline.ready_to_open() {
                pipeline.open();
                opened = true;
                pipeline.push(&[], Some(&mut session), &mut out).unwrap();
            }
        }
    }
    assert!(pipeline.spilled_bytes() > 0, "the hold went to disk");
    pipeline.finish(&mut session, &mut out).unwrap();
    let mut expected = body.clone();
    mask(&mut expected, &["Pp00Oo99Ii88Uu77".into()]);
    expected["model"] = body["model"].clone();
    assert_eq!(serde_json::from_slice::<Value>(&out).unwrap(), expected);
    drop(pipeline);
    assert_eq!(config.in_flight(), 0);
    let _ = std::fs::remove_dir_all(dir);
}

/// A session that judges the whole body gets its output held, whatever
/// framing the transport chose; others keep theirs.
#[test]
fn verdict_sessions_hold_their_output() {
    use crate::stream_driver::{verdict_output, Output};
    assert_eq!(verdict_output(Output::Chunked, true), Output::Sized);
    assert_eq!(verdict_output(Output::Sized, true), Output::Sized);
    for length in [None, Some(7)] {
        for hold in [false, true] {
            let tee = Output::Tee { length, hold };
            assert_eq!(
                verdict_output(tee, true),
                Output::Tee { length, hold: true }
            );
            assert_eq!(verdict_output(tee, false), tee);
        }
    }
    assert_eq!(verdict_output(Output::Chunked, false), Output::Chunked);
}
