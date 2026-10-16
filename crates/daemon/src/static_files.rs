use axum::body::Body;
use axum::http::{header, StatusCode, Uri};
use axum::response::{IntoResponse, Response};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../../ui/dist/nixos-manager-ui/browser/"]
struct Assets;

/// Serve the Angular bundle; unknown paths fall back to index.html so the
/// client-side router works on reload.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    let path = if path.is_empty() { "index.html" } else { path };
    match Assets::get(path) {
        Some(content) => file_response(path, content.data.into_owned()),
        None => match Assets::get("index.html") {
            Some(index) => file_response("index.html", index.data.into_owned()),
            None => (StatusCode::NOT_FOUND, "UI not built").into_response(),
        },
    }
}

fn file_response(path: &str, data: Vec<u8>) -> Response {
    let mime = mime_guess::from_path(path).first_or_octet_stream();
    let cache = if path == "index.html" { "no-cache" } else { "public, max-age=31536000, immutable" };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, mime.as_ref())
        .header(header::CACHE_CONTROL, cache)
        .body(Body::from(data))
        .unwrap()
}
