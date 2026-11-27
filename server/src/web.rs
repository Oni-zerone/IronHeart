//! Dashboard (Vue build in `web/dist`) embedded in the binary.
//!
//! In release builds the files are compiled in; in debug builds rust-embed reads
//! them from disk, so `pnpm build` is enough to see changes. Without a build the
//! server falls back to the temporary login page (`main::home`).

use axum::{
    http::{StatusCode, Uri, header},
    response::{IntoResponse, Response},
};
use rust_embed::RustEmbed;

#[derive(RustEmbed)]
#[folder = "../web/dist"]
#[allow_missing = true]
struct Assets;

/// True when the dashboard build is available.
pub fn available() -> bool {
    Assets::get("index.html").is_some()
}

/// Static files, and `index.html` for the routes handled by Vue Router.
pub async fn serve(uri: Uri) -> Response {
    let path = uri.path().trim_start_matches('/');
    if let Some(file) = (!path.is_empty()).then(|| Assets::get(path)).flatten() {
        // Vite puts a content hash in the names under `assets/`.
        let cache = if path.starts_with("assets/") {
            "public, max-age=31536000, immutable"
        } else {
            "no-cache"
        };
        return (
            [(header::CONTENT_TYPE, file.metadata.mimetype().to_owned()), (header::CACHE_CONTROL, cache.to_owned())],
            file.data,
        )
            .into_response();
    }
    // Unknown API paths and missing files stay 404; everything else is a client route.
    let is_client_route = !(path.starts_with("api/") || path.starts_with("auth/") || last_segment_has_dot(path));
    match Assets::get("index.html").filter(|_| is_client_route) {
        Some(index) => (
            [(header::CONTENT_TYPE, "text/html; charset=utf-8"), (header::CACHE_CONTROL, "no-cache")],
            index.data,
        )
            .into_response(),
        None => StatusCode::NOT_FOUND.into_response(),
    }
}

fn last_segment_has_dot(path: &str) -> bool {
    path.rsplit('/').next().is_some_and(|s| s.contains('.'))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_names_are_not_client_routes() {
        assert!(last_segment_has_dot("favicon.ico"));
        assert!(last_segment_has_dot("assets/index-abc.js"));
        assert!(!last_segment_has_dot("apps/0190a1b2-c3d4"));
        assert!(!last_segment_has_dot(""));
    }
}
