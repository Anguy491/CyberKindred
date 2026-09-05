use std::sync::{Arc, RwLock};

use sha2::{Digest, Sha256};
use tauri::http::{Method, Request, Response, StatusCode, header};

const ARTWORK_URI_PREFIX: &str = "asset://artwork/system/";
const MAX_ARTWORK_BYTES: usize = 5 * 1024 * 1024;

#[derive(Clone)]
struct ArtworkAsset {
    key: String,
    content_type: &'static str,
    bytes: Arc<[u8]>,
}

/// Holds only the current system-session artwork in Rust-owned process memory.
/// The renderer receives an opaque content-addressed URI, never OS paths.
#[derive(Clone, Default)]
pub struct ArtworkAssetStore {
    current: Arc<RwLock<Option<ArtworkAsset>>>,
}

impl ArtworkAssetStore {
    #[must_use]
    pub(crate) fn replace(&self, bytes: Vec<u8>) -> Option<String> {
        if bytes.is_empty() || bytes.len() > MAX_ARTWORK_BYTES {
            self.clear();
            return None;
        }
        let Some(content_type) = supported_content_type(&bytes) else {
            self.clear();
            return None;
        };
        let key = hex::encode(Sha256::digest(&bytes));
        let asset = ArtworkAsset {
            key: key.clone(),
            content_type,
            bytes: bytes.into(),
        };
        *self.current.write().ok()? = Some(asset);
        Some(format!("{ARTWORK_URI_PREFIX}{key}"))
    }

    pub(crate) fn clear(&self) {
        if let Ok(mut current) = self.current.write() {
            current.take();
        }
    }

    pub(crate) fn protocol_response(&self, request: &Request<Vec<u8>>) -> Response<Vec<u8>> {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return response(StatusCode::METHOD_NOT_ALLOWED, "text/plain", Vec::new());
        }
        let Some(key) = request_key(request.uri().path()) else {
            return response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
        };
        let Some(asset) = self
            .current
            .read()
            .ok()
            .and_then(|current| current.as_ref().filter(|asset| asset.key == key).cloned())
        else {
            return response(StatusCode::NOT_FOUND, "text/plain", Vec::new());
        };
        let body = if request.method() == Method::HEAD {
            Vec::new()
        } else {
            asset.bytes.as_ref().to_vec()
        };
        response(StatusCode::OK, asset.content_type, body)
    }
}

fn supported_content_type(bytes: &[u8]) -> Option<&'static str> {
    match infer::get(bytes)?.mime_type() {
        "image/jpeg" => Some("image/jpeg"),
        "image/png" => Some("image/png"),
        "image/webp" => Some("image/webp"),
        "image/gif" => Some("image/gif"),
        _ => None,
    }
}

fn request_key(path: &str) -> Option<&str> {
    let key = path
        .strip_prefix("/artwork/system/")
        .or_else(|| path.strip_prefix("/system/"))?;
    (key.len() == 64 && key.bytes().all(|byte| byte.is_ascii_hexdigit())).then_some(key)
}

fn response(status: StatusCode, content_type: &'static str, body: Vec<u8>) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header(header::CONTENT_TYPE, content_type)
        .header(header::CACHE_CONTROL, "private, max-age=60")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .unwrap_or_else(|_| Response::new(Vec::new()))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png() -> Vec<u8> {
        vec![
            0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a, 0, 0, 0, 0, b'I', b'H', b'D', b'R',
        ]
    }

    #[test]
    fn system_artwork_store_exposes_only_current_sniffed_asset() {
        let store = ArtworkAssetStore::default();
        let uri = store.replace(png()).expect("supported artwork");
        let key = uri.strip_prefix(ARTWORK_URI_PREFIX).expect("opaque prefix");
        let request = Request::builder()
            .uri(format!("http://asset.localhost/artwork/system/{key}"))
            .body(Vec::new())
            .expect("request");
        let response = store.protocol_response(&request);
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.headers()[header::CONTENT_TYPE], "image/png");
        assert_eq!(response.body(), &png());

        assert!(
            store
                .replace(b"<html>not artwork</html>".to_vec())
                .is_none()
        );
        assert_eq!(
            store.protocol_response(&request).status(),
            StatusCode::NOT_FOUND
        );
    }

    #[test]
    fn system_artwork_protocol_rejects_unscoped_keys_and_writes() {
        let store = ArtworkAssetStore::default();
        let _uri = store.replace(png()).expect("supported artwork");
        for (method, uri) in [
            (Method::GET, "http://asset.localhost/covers/private.png"),
            (
                Method::GET,
                "http://asset.localhost/artwork/system/../secret",
            ),
            (
                Method::POST,
                "http://asset.localhost/artwork/system/aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            ),
        ] {
            let request = Request::builder()
                .method(method)
                .uri(uri)
                .body(Vec::new())
                .expect("request");
            assert_ne!(store.protocol_response(&request).status(), StatusCode::OK);
        }
    }
}
