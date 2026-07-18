use std::borrow::Cow;

use wry::http::{
    header::{
        CACHE_CONTROL, CONTENT_LENGTH, CONTENT_SECURITY_POLICY, CONTENT_TYPE,
        X_CONTENT_TYPE_OPTIONS,
    },
    Method, Request, Response, StatusCode,
};

use crate::{HostError, WebAsset, EDITOR_CSP};

const PROTOCOL_HOST: &str = "localhost";

#[derive(Clone, Copy, Debug)]
pub(crate) struct AssetRouter {
    protocol: &'static str,
    entry_path: &'static str,
    assets: &'static [WebAsset],
}

impl AssetRouter {
    pub(crate) fn new(
        protocol: &'static str,
        entry_path: &'static str,
        assets: &'static [WebAsset],
    ) -> Result<Self, HostError> {
        validate_protocol(protocol)?;
        let entry_path = normalize_config_path(entry_path)?;
        if assets.is_empty() {
            return Err(HostError::InvalidConfig(
                "at least one embedded WebAsset is required".into(),
            ));
        }

        let mut has_entry = false;
        for (index, asset) in assets.iter().enumerate() {
            let normalized = normalize_config_path(asset.path)?;
            if normalized != asset.path {
                return Err(HostError::InvalidConfig(format!(
                    "WebAsset path must already be canonical: {}",
                    asset.path
                )));
            }
            mime_for_path(asset.path).ok_or_else(|| {
                HostError::InvalidConfig(format!(
                    "WebAsset has an unsupported file extension: {}",
                    asset.path
                ))
            })?;
            if asset.path == entry_path {
                has_entry = true;
            }
            if assets[..index]
                .iter()
                .any(|candidate| candidate.path == asset.path)
            {
                return Err(HostError::InvalidConfig(format!(
                    "duplicate WebAsset path: {}",
                    asset.path
                )));
            }
        }

        if !has_entry {
            return Err(HostError::InvalidConfig(format!(
                "entry WebAsset does not exist: {entry_path}"
            )));
        }

        Ok(Self {
            protocol,
            entry_path,
            assets,
        })
    }

    pub(crate) fn initial_url(&self) -> String {
        format!("{}://{PROTOCOL_HOST}/", self.protocol)
    }

    pub(crate) fn allows_navigation(&self, url: &str) -> bool {
        if url == "about:blank" {
            return true;
        }

        let custom_origin = format!("{}://{PROTOCOL_HOST}", self.protocol);
        let webview2_http_origin = format!("http://{}.localhost", self.protocol);
        let webview2_https_origin = format!("https://{}.localhost", self.protocol);
        has_exact_origin(url, &custom_origin)
            || has_exact_origin(url, &webview2_http_origin)
            || has_exact_origin(url, &webview2_https_origin)
    }

    pub(crate) fn response(&self, request: Request<Vec<u8>>) -> Response<Cow<'static, [u8]>> {
        if request.method() != Method::GET && request.method() != Method::HEAD {
            return error_response(
                StatusCode::METHOD_NOT_ALLOWED,
                "only GET and HEAD are supported",
            );
        }

        let raw_path = request.uri().path();
        let normalized = if raw_path.is_empty() || raw_path == "/" {
            Ok(self.entry_path)
        } else {
            normalize_request_path(raw_path)
        };
        let path = match normalized {
            Ok(path) => path,
            Err(message) => return error_response(StatusCode::BAD_REQUEST, &message),
        };

        let Some(asset) = self.assets.iter().find(|asset| asset.path == path) else {
            return error_response(StatusCode::NOT_FOUND, "asset not found");
        };
        let Some(content_type) = mime_for_path(asset.path) else {
            return error_response(StatusCode::UNSUPPORTED_MEDIA_TYPE, "unsupported asset type");
        };
        let body = if request.method() == Method::HEAD {
            Cow::Borrowed(&[][..])
        } else {
            Cow::Borrowed(asset.bytes)
        };

        secure_response(StatusCode::OK, content_type, asset.bytes.len(), body)
    }
}

pub(crate) fn has_exact_origin(url: &str, origin: &str) -> bool {
    url == origin
        || url
            .strip_prefix(origin)
            .is_some_and(|suffix| suffix.starts_with('/') || suffix.starts_with('#'))
}

fn validate_protocol(protocol: &str) -> Result<(), HostError> {
    let mut chars = protocol.chars();
    if !chars.next().is_some_and(|first| first.is_ascii_lowercase())
        || !chars.all(|character| {
            character.is_ascii_lowercase()
                || character.is_ascii_digit()
                || matches!(character, '+' | '-' | '.')
        })
        || matches!(protocol, "http" | "https" | "file" | "data" | "javascript")
    {
        return Err(HostError::InvalidConfig(format!(
            "invalid or reserved custom protocol: {protocol}"
        )));
    }
    Ok(())
}

fn normalize_config_path(path: &'static str) -> Result<&'static str, HostError> {
    validate_canonical_relative_path(path)
        .map(|()| path)
        .map_err(|message| {
            HostError::InvalidConfig(format!("invalid WebAsset path {path:?}: {message}"))
        })
}

fn normalize_request_path(path: &str) -> Result<&str, String> {
    let relative = path
        .strip_prefix('/')
        .ok_or_else(|| "asset path must be absolute".to_string())?;
    if relative.starts_with('/') {
        return Err("asset path contains an empty component".into());
    }

    let decoded = decode_percent_path(relative)?;
    validate_canonical_relative_path(&decoded)?;

    // Borrow directly from the request when no decoding was necessary. The response lookup is
    // completed before this function returns, so decoded data can be handled by the caller below.
    if decoded == relative {
        Ok(relative)
    } else {
        // Embedded asset paths are deliberately restricted to an unescaped ASCII-safe subset.
        // A decoded request therefore cannot name a valid configured asset and is rejected rather
        // than leaking an allocation or creating two URL spellings for the same resource.
        Err("percent-encoded asset paths are not accepted".into())
    }
}

fn decode_percent_path(path: &str) -> Result<String, String> {
    let bytes = path.as_bytes();
    let mut decoded = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            if index + 2 >= bytes.len() {
                return Err("asset path contains an incomplete percent escape".into());
            }
            let high = decode_hex(bytes[index + 1])
                .ok_or_else(|| "asset path contains an invalid percent escape".to_string())?;
            let low = decode_hex(bytes[index + 2])
                .ok_or_else(|| "asset path contains an invalid percent escape".to_string())?;
            decoded.push((high << 4) | low);
            index += 3;
        } else {
            decoded.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(decoded).map_err(|_| "asset path is not valid UTF-8".into())
}

fn decode_hex(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'a'..=b'f' => Some(byte - b'a' + 10),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
}

fn validate_canonical_relative_path(path: &str) -> Result<(), String> {
    if path.is_empty() || path.starts_with('/') || path.ends_with('/') {
        return Err("asset path must be a non-empty relative file path".into());
    }
    if !path.is_ascii() {
        return Err("asset paths must be ASCII".into());
    }
    if path
        .bytes()
        .any(|byte| byte == b'\\' || byte == b'\0' || byte == b'%' || byte.is_ascii_control())
    {
        return Err("asset path contains a forbidden character".into());
    }
    if path.split('/').any(|component| {
        component.is_empty()
            || component == "."
            || component == ".."
            || !component
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    }) {
        return Err("asset path contains a forbidden component".into());
    }
    Ok(())
}

fn mime_for_path(path: &str) -> Option<&'static str> {
    let extension = path.rsplit_once('.')?.1.to_ascii_lowercase();
    match extension.as_str() {
        "html" => Some("text/html; charset=utf-8"),
        "js" | "mjs" => Some("text/javascript; charset=utf-8"),
        "css" => Some("text/css; charset=utf-8"),
        "json" | "map" => Some("application/json; charset=utf-8"),
        "wasm" => Some("application/wasm"),
        "svg" => Some("image/svg+xml"),
        "png" => Some("image/png"),
        "jpg" | "jpeg" => Some("image/jpeg"),
        "gif" => Some("image/gif"),
        "webp" => Some("image/webp"),
        "ico" => Some("image/x-icon"),
        "woff" => Some("font/woff"),
        "woff2" => Some("font/woff2"),
        "ttf" => Some("font/ttf"),
        "otf" => Some("font/otf"),
        "txt" => Some("text/plain; charset=utf-8"),
        _ => None,
    }
}

fn error_response(status: StatusCode, message: &str) -> Response<Cow<'static, [u8]>> {
    let body: Cow<'static, [u8]> = Cow::Owned(message.as_bytes().to_vec());
    secure_response(status, "text/plain; charset=utf-8", body.len(), body)
}

fn secure_response(
    status: StatusCode,
    content_type: &'static str,
    content_length: usize,
    body: Cow<'static, [u8]>,
) -> Response<Cow<'static, [u8]>> {
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, content_type)
        .header(CONTENT_LENGTH, content_length.to_string())
        .header(CONTENT_SECURITY_POLICY, EDITOR_CSP)
        .header(X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(CACHE_CONTROL, "no-store")
        .body(body)
        .expect("host-generated response headers are valid")
}

#[cfg(test)]
mod tests {
    use super::*;

    static ASSETS: &[WebAsset] = &[
        WebAsset::new("index.html", b"<main>editor</main>"),
        WebAsset::new("assets/app.js", b"export const ready = true"),
        WebAsset::new("assets/app.css", b"body{}"),
    ];

    fn router() -> AssetRouter {
        AssetRouter::new("engine-editor", "index.html", ASSETS).unwrap()
    }

    fn request(path: &str) -> Request<Vec<u8>> {
        Request::builder()
            .uri(format!("engine-editor://localhost{path}"))
            .body(Vec::new())
            .unwrap()
    }

    #[test]
    fn root_routes_to_entry_asset_with_security_headers() {
        let response = router().response(request("/"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body().as_ref(), ASSETS[0].bytes);
        assert_eq!(response.headers()[CONTENT_TYPE], "text/html; charset=utf-8");
        assert_eq!(response.headers()[CONTENT_SECURITY_POLICY], EDITOR_CSP);
        assert_eq!(response.headers()[X_CONTENT_TYPE_OPTIONS], "nosniff");
    }

    #[test]
    fn nested_asset_routes_with_derived_mime() {
        let response = router().response(request("/assets/app.js"));
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(response.body().as_ref(), ASSETS[1].bytes);
        assert_eq!(
            response.headers()[CONTENT_TYPE],
            "text/javascript; charset=utf-8"
        );
    }

    #[test]
    fn head_returns_metadata_without_body() {
        let request = Request::builder()
            .method(Method::HEAD)
            .uri("engine-editor://localhost/assets/app.css")
            .body(Vec::new())
            .unwrap();
        let response = router().response(request);
        assert_eq!(response.status(), StatusCode::OK);
        assert!(response.body().is_empty());
        assert_eq!(
            response.headers()[CONTENT_LENGTH],
            ASSETS[2].bytes.len().to_string()
        );
    }

    #[test]
    fn missing_asset_is_not_spa_fallback() {
        let response = router().response(request("/missing.js"));
        assert_eq!(response.status(), StatusCode::NOT_FOUND);
    }

    #[test]
    fn rejects_plain_and_encoded_parent_traversal() {
        for path in [
            "/../index.html",
            "/assets/../index.html",
            "/%2e%2e/index.html",
            "/assets/%2E%2E/index.html",
            "/%252e%252e/index.html",
        ] {
            let response = router().response(request(path));
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "path: {path}");
        }
    }

    #[test]
    fn rejects_backslashes_empty_components_and_nul() {
        for path in [
            "/assets\\app.js",
            "/assets/%5capp.js",
            "//assets/app.js",
            "/assets//app.js",
            "/assets/%00app.js",
        ] {
            let response = router().response(request(path));
            assert_eq!(response.status(), StatusCode::BAD_REQUEST, "path: {path}");
        }
    }

    #[test]
    fn navigation_is_restricted_to_protocol_origins() {
        let router = router();
        assert!(router.allows_navigation("engine-editor://localhost/"));
        assert!(router.allows_navigation("http://engine-editor.localhost/assets/app.js"));
        assert!(router.allows_navigation("https://engine-editor.localhost/#scene"));
        assert!(router.allows_navigation("about:blank"));
        assert!(!router.allows_navigation("https://example.com"));
        assert!(!router.allows_navigation("https://engine-editor.localhost.evil.test"));
        assert!(!router.allows_navigation("javascript:alert(1)"));
        assert!(!router.allows_navigation("file:///etc/passwd"));
    }

    #[test]
    fn duplicate_and_unsupported_assets_fail_configuration() {
        static DUPLICATE: &[WebAsset] = &[
            WebAsset::new("index.html", b"a"),
            WebAsset::new("index.html", b"b"),
        ];
        static UNSUPPORTED: &[WebAsset] = &[WebAsset::new("index.bin", b"a")];
        assert!(AssetRouter::new("engine-editor", "index.html", DUPLICATE).is_err());
        assert!(AssetRouter::new("engine-editor", "index.bin", UNSUPPORTED).is_err());
    }
}
