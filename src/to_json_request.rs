use headers::{ContentLength, ContentType, HeaderMapExt};
use http::header::{CONTENT_LENGTH, CONTENT_TYPE};
use http::Request;
use serde::Serialize;

pub fn to_json_request<T, B>(request: Request<T>) -> Result<Request<B>, serde_json::Error>
where
    T: Serialize,
    B: From<String>,
{
    let (mut parts, body) = request.into_parts();
    let body = serde_json::to_string(&body)?;
    if !parts.headers.contains_key(CONTENT_LENGTH) {
        parts.headers.typed_insert(ContentLength(body.len() as _));
    }
    if !parts.headers.contains_key(CONTENT_TYPE) {
        parts.headers.typed_insert(ContentType::json());
    }
    Ok(Request::from_parts(parts, body.into()))
}
