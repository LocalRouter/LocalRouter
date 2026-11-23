use super::*;
use axum::{body::Body, extract::FromRequest, http::HeaderValue};

fn headers(content_type: &str) -> HeaderMap {
    let mut headers = HeaderMap::new();
    headers.insert(
        header::CONTENT_TYPE,
        HeaderValue::from_str(content_type).unwrap(),
    );
    headers
}

#[test]
fn content_routes_are_classified_by_path_method_and_media_type() {
    let json = headers("application/json");
    let form = headers("multipart/form-data; boundary=x");
    for path in [
        "/embeddings",
        "/moderations",
        "/rerank",
        "/search",
        "/ocr",
        "/images/generations",
        "/audio/speech",
        "/videos",
        "/messages/count_tokens",
    ] {
        assert_eq!(
            classify(&Method::POST, path, &json),
            Some(ContentRoute::Json),
            "{path}"
        );
    }
    for path in [
        "/audio/transcriptions",
        "/audio/translations",
        "/images/edits",
    ] {
        assert_eq!(
            classify(&Method::POST, path, &form),
            Some(ContentRoute::Form),
            "{path}"
        );
    }
    assert_eq!(
        classify(&Method::POST, "/videos", &form),
        Some(ContentRoute::Form)
    );
    assert_eq!(
        classify(&Method::POST, "/videos", &headers(" Multipart/Form-Data")),
        Some(ContentRoute::Form)
    );
    // Inference, control, local storage and non-POST routes are not content
    // routes.
    for (method, path) in [
        (Method::POST, "/chat/completions"),
        (Method::POST, "/files"),
        (Method::POST, "/batches"),
        (Method::POST, "/realtime/client_secrets"),
        (Method::POST, "/videos/v1/cancel"),
        (Method::GET, "/videos"),
        (Method::GET, "/embeddings"),
    ] {
        assert_eq!(classify(&method, path, &json), None, "{method} {path}");
    }
}

fn contents(path: &str, body: Value) -> Vec<String> {
    projection(path, &body)["messages"]
        .as_array()
        .unwrap()
        .iter()
        .map(|message| {
            assert_eq!(message["role"], "user");
            message["content"].as_str().unwrap().to_owned()
        })
        .collect()
}

#[test]
fn projections_hold_the_user_text_of_each_route() {
    assert_eq!(
        contents("/embeddings", json!({"model": "m", "input": "one"})),
        ["one"]
    );
    assert_eq!(
        contents(
            "/embeddings",
            json!({"model": "m", "input": ["one", "", [1, 2], "two"], "user": "u"})
        ),
        ["one", "two"],
        "token ids and the user id are not text"
    );
    assert_eq!(
        contents(
            "/moderations",
            json!({"input": [
                {"type": "text", "text": "words"},
                {"type": "image_url", "image_url": {"url": "data:image/png;base64,AAAA"}}
            ]})
        ),
        ["words"]
    );
    assert_eq!(
        contents(
            "/rerank",
            json!({"model": "m", "query": "q", "documents": ["d1", {"text": "d2"}], "top_n": 1})
        ),
        ["q", "d1", "d2"]
    );
    assert_eq!(
        contents("/search", json!({"query": ["q1", "q2"], "country": "US"})),
        ["q1", "q2"]
    );
    assert_eq!(
        contents(
            "/ocr",
            json!({"model": "m", "document": {"document_url": "https://example.com/d.pdf"},
                   "pages": "0", "filename": "notes.pdf"})
        ),
        ["notes.pdf"]
    );
    assert_eq!(
        contents(
            "/images/generations",
            json!({"model": "m", "prompt": "a cat", "size": "1024x1024"})
        ),
        ["a cat"]
    );
    assert_eq!(
        contents(
            "/videos",
            json!({"model": "p/m", "prompt": "waves", "seconds": 4})
        ),
        ["waves"]
    );
    assert_eq!(
        contents(
            "/audio/speech",
            json!({"model": "m", "input": "say", "instructions": "softly", "voice": "alloy"})
        ),
        ["say", "softly"]
    );
    let count = json!({"model": "m", "messages": [{"role": "user", "content": "hi"}]});
    assert!(matches!(
        projection("/messages/count_tokens", &count),
        Cow::Borrowed(value) if std::ptr::eq(value, &count)
    ));
}

const BOUNDARY: &str = "form-boundary";

fn form(fields: &[(&str, &[u8])]) -> axum::extract::Request {
    let mut body = Vec::new();
    for (name, value) in fields {
        body.extend_from_slice(
            format!("--{BOUNDARY}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n")
                .as_bytes(),
        );
        body.extend_from_slice(value);
        body.extend_from_slice(b"\r\n");
    }
    body.extend_from_slice(format!("--{BOUNDARY}--\r\n").as_bytes());
    axum::extract::Request::builder()
        .method("POST")
        .header(
            "content-type",
            format!("multipart/form-data; boundary={BOUNDARY}"),
        )
        .body(Body::from(body))
        .unwrap()
}

#[tokio::test]
async fn form_fields_are_read_up_to_their_limit_only() {
    let large = vec![b'x'; FORM_TEXT_LIMIT + 1];
    let mut multipart = axum::extract::Multipart::from_request(
        form(&[
            ("prompt", "within".as_bytes()),
            ("file", &[1, 2, 3, 4, 5]),
            ("prompt", &large),
            ("prompt", &[0xff, 0xfe]),
        ]),
        &(),
    )
    .await
    .unwrap();
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(form_text(field).await.unwrap(), "within");
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(
        form_bytes(field, 4).await.unwrap_err(),
        "exceeds 4 bytes",
        "a file over its limit is refused"
    );
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(form_text(field).await.unwrap_err(), "exceeds 128 KiB");
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(form_text(field).await.unwrap_err(), "is not valid UTF-8");
    assert!(multipart.next_field().await.unwrap().is_none());

    let mut multipart = axum::extract::Multipart::from_request(form(&[("file", &[7; 1000])]), &())
        .await
        .unwrap();
    let field = multipart.next_field().await.unwrap().unwrap();
    assert_eq!(form_bytes(field, 1000).await.unwrap(), vec![7; 1000]);
}
