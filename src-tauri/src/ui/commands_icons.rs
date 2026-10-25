//! Best-effort, user-action-only favicon discovery. Never sends endpoint credentials.
use base64::Engine;
use reqwest::{Client, Url};
use std::{io::Cursor, time::Duration};

const MAX_BYTES: usize = 256 * 1024;

fn origin_url(endpoint_url: &str) -> Option<Url> {
    let url = Url::parse(endpoint_url).ok()?;
    if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() {
        return None;
    }
    Url::parse(&format!("{}/", url.origin().ascii_serialization())).ok()
}

async fn fetch(client: &Client, url: Url) -> Option<Vec<u8>> {
    fetch_document(client, url).await.map(|(_, bytes)| bytes)
}

async fn fetch_document(client: &Client, url: Url) -> Option<(Url, Vec<u8>)> {
    let origin = url.origin();
    let mut target = url;
    let mut response = None;
    for _ in 0..4 {
        let candidate = client.get(target.clone()).send().await.ok()?;
        if candidate.status().is_redirection() {
            let location = candidate
                .headers()
                .get(reqwest::header::LOCATION)?
                .to_str()
                .ok()?;
            target = target.join(location).ok()?;
            if target.origin() != origin
                || !target.username().is_empty()
                || target.password().is_some()
            {
                return None;
            }
        } else {
            response = Some(candidate.error_for_status().ok()?);
            break;
        }
    }
    let mut response = response?;
    if response
        .content_length()
        .is_some_and(|n| n > MAX_BYTES as u64)
    {
        return None;
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.ok()? {
        if bytes.len() + chunk.len() > MAX_BYTES {
            return None;
        }
        bytes.extend_from_slice(&chunk);
    }
    Some((target, bytes))
}

fn icon_links(html: &str, origin: &Url) -> Vec<Url> {
    // Attribute order, quote style and case do not matter. Deliberately only
    // accept same-origin assets; no favicon services, CDN tracking or SVG loads.
    let links = regex::Regex::new(r"(?is)<link\s+[^>]*>").expect("constant regex");
    let attributes =
        regex::Regex::new(r#"(?is)\s(rel|href)\s*=\s*(?:"([^"]*)"|'([^']*)'|([^\s>]+))"#)
            .expect("constant regex");
    links
        .find_iter(html)
        .filter_map(|link| {
            let mut rel = String::new();
            let mut href = String::new();
            for attr in attributes.captures_iter(link.as_str()) {
                let value = attr
                    .get(2)
                    .or_else(|| attr.get(3))
                    .or_else(|| attr.get(4))?
                    .as_str();
                match attr[1].to_ascii_lowercase().as_str() {
                    "rel" => rel = value.to_ascii_lowercase(),
                    "href" => href = value.replace("&amp;", "&"),
                    _ => {}
                }
            }
            if !rel
                .split_whitespace()
                .any(|r| matches!(r, "icon" | "apple-touch-icon"))
            {
                return None;
            }
            let url = origin.join(&href).ok()?;
            ((url.origin() == origin.origin() || url.scheme() == "data")
                && url.username().is_empty()
                && url.password().is_none())
            .then_some(url)
        })
        .take(3)
        .collect()
}

fn inline_icon(url: &Url) -> Option<Vec<u8>> {
    let (header, payload) = url.as_str().split_once(',')?;
    if !header.starts_with("data:image/")
        || !header.ends_with(";base64")
        || payload.len() > MAX_BYTES
    {
        return None;
    }
    base64::engine::general_purpose::STANDARD
        .decode(payload)
        .ok()
}

fn png_data(bytes: &[u8]) -> Option<String> {
    let mut reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .ok()?;
    let mut limits = image::Limits::default();
    limits.max_image_width = Some(1024);
    limits.max_image_height = Some(1024);
    limits.max_alloc = Some(8 * 1024 * 1024);
    reader.limits(limits);
    let image = reader.decode().ok()?.thumbnail(64, 64);
    let mut output = Cursor::new(Vec::new());
    image.write_to(&mut output, image::ImageFormat::Png).ok()?;
    Some(format!(
        "data:image/png;base64,{}",
        base64::engine::general_purpose::STANDARD.encode(output.into_inner())
    ))
}

async fn discover(endpoint_url: &str) -> Option<String> {
    let origin = origin_url(endpoint_url)?;
    let client = Client::builder()
        .timeout(Duration::from_secs(2))
        .redirect(reqwest::redirect::Policy::none())
        .build()
        .ok()?;
    let mut candidates = Vec::new();
    if let Some((page_url, bytes)) = fetch_document(&client, origin.clone()).await {
        candidates = icon_links(&String::from_utf8_lossy(&bytes), &page_url);
    }
    candidates.push(origin.join("favicon.ico").ok()?);
    for url in candidates {
        let bytes = if url.scheme() == "data" {
            inline_icon(&url)
        } else {
            fetch(&client, url).await
        };
        if let Some(data) = bytes.as_deref().and_then(png_data) {
            return Some(data);
        }
    }
    None
}

/// Called only after an explicit endpoint add/edit; rendering reads local cache.
#[tauri::command]
pub async fn discover_resource_icon(endpoint_url: String) -> Option<String> {
    tokio::time::timeout(Duration::from_secs(8), discover(&endpoint_url))
        .await
        .ok()
        .flatten()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn strips_endpoint_credentials_path_and_query() {
        assert_eq!(
            origin_url("https://user:secret@example.com:8443/api?key=secret#x")
                .unwrap()
                .as_str(),
            "https://example.com:8443/"
        );
        for invalid in ["file:///etc/passwd", "data:image/png,test", "not a url"] {
            assert!(origin_url(invalid).is_none());
        }
    }

    #[test]
    fn resolves_declared_icons_and_excludes_other_origins() {
        let origin = origin_url("https://example.com/api").unwrap();
        let urls = icon_links(
            r#"<LINK HREF='/brand.png' REL='shortcut icon'><link rel="apple-touch-icon" href=/apple.png><link rel=stylesheet href=/style.css><link rel=icon href=https://tracker.example/icon><link rel=icon href=//example.com/ok.png>"#,
            &origin,
        );
        assert_eq!(
            urls.iter().map(Url::path).collect::<Vec<_>>(),
            ["/brand.png", "/apple.png", "/ok.png"]
        );
    }

    #[test]
    fn normalizes_raster_and_rejects_active_content() {
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(128, 128)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let data = png_data(png.get_ref()).unwrap();
        let decoded = base64::engine::general_purpose::STANDARD
            .decode(data.strip_prefix("data:image/png;base64,").unwrap())
            .unwrap();
        assert_eq!(image::load_from_memory(&decoded).unwrap().width(), 64);
        assert!(png_data(b"<svg onload='evil()'></svg>").is_none());
        assert!(png_data(b"not an image").is_none());
    }

    #[tokio::test]
    async fn discovers_fallback_without_endpoint_secrets() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(16, 16)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(404))
            .mount(&server)
            .await;
        Mock::given(path("/favicon.ico"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png.into_inner()))
            .mount(&server)
            .await;
        assert!(discover(&format!("{}/private?token=secret", server.uri()))
            .await
            .is_some());
        for request in server.received_requests().await.unwrap() {
            assert!(request.url.query().is_none());
            assert!(!request.headers.contains_key("authorization"));
        }
    }
    #[tokio::test]
    async fn discovers_html_link_and_inline_image_before_fallback() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let mut png = Cursor::new(Vec::new());
        image::DynamicImage::new_rgba8(16, 16)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/site/"))
            .mount(&server)
            .await;
        Mock::given(path("/site/"))
            .respond_with(
                ResponseTemplate::new(200).set_body_string("<link href='brand.png' rel='icon'>"),
            )
            .mount(&server)
            .await;
        Mock::given(path("/site/brand.png"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(png.get_ref().clone()))
            .mount(&server)
            .await;
        assert!(discover(&server.uri()).await.is_some());
        assert_eq!(server.received_requests().await.unwrap().len(), 3);
        server.reset().await;
        let inline = format!(
            "<link rel=icon href='data:image/png;base64,{}'>",
            base64::engine::general_purpose::STANDARD.encode(png.into_inner())
        );
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_string(inline))
            .mount(&server)
            .await;
        assert!(discover(&server.uri()).await.is_some());
        assert_eq!(server.received_requests().await.unwrap().len(), 1);
    }

    #[tokio::test]
    async fn follows_local_redirects_but_never_another_origin() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        let other = MockServer::start().await;
        let client = Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .unwrap();
        Mock::given(path("/local"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", "/asset"))
            .mount(&server)
            .await;
        Mock::given(path("/asset"))
            .respond_with(ResponseTemplate::new(200).set_body_string("image"))
            .mount(&server)
            .await;
        assert_eq!(
            fetch(
                &client,
                Url::parse(&format!("{}/local", server.uri())).unwrap()
            )
            .await
            .unwrap(),
            b"image"
        );
        Mock::given(path("/external"))
            .respond_with(ResponseTemplate::new(302).insert_header("Location", other.uri()))
            .mount(&server)
            .await;
        assert!(fetch(
            &client,
            Url::parse(&format!("{}/external", server.uri())).unwrap()
        )
        .await
        .is_none());
        assert!(other.received_requests().await.unwrap().is_empty());
    }

    #[tokio::test]
    async fn rejects_large_responses_and_missing_icons() {
        use wiremock::{matchers::path, Mock, MockServer, ResponseTemplate};
        let server = MockServer::start().await;
        Mock::given(path("/"))
            .respond_with(ResponseTemplate::new(200).set_body_bytes(vec![0; MAX_BYTES + 1]))
            .mount(&server)
            .await;
        assert!(discover(&server.uri()).await.is_none());
    }
}
