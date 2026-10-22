//! [`Fetch`] over the browser's `fetch`.

use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, AbortSignal, ReadableStreamDefaultReader, ReferrerPolicy, RequestCredentials,
    RequestInit, RequestMode, Response, Window,
};

use crate::object::{Fetch, FetchError};

/// How long one request may take, body included.
const TIMEOUT_MS: i32 = 15_000;

pub(crate) struct Browser;

impl Fetch for Browser {
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if !url.starts_with("https://") {
            return Err(FetchError::NotHttps);
        }
        let window = web_sys::window().ok_or(FetchError::Failed)?;
        let controller = AbortController::new().map_err(|_| FetchError::Failed)?;
        let abort = controller.clone();
        let on_timeout = Closure::once_into_js(move || abort.abort());
        let timer = window
            .set_timeout_with_callback_and_timeout_and_arguments_0(
                on_timeout.unchecked_ref(),
                TIMEOUT_MS,
            )
            .map_err(|_| FetchError::Failed)?;
        let result = read(&window, url, limit, &controller.signal()).await;
        window.clear_timeout_with_handle(timer);
        result
    }
}

async fn read(
    window: &Window,
    url: &str,
    limit: u64,
    signal: &AbortSignal,
) -> Result<Vec<u8>, FetchError> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_mode(RequestMode::Cors);
    init.set_credentials(RequestCredentials::Omit);
    init.set_referrer_policy(ReferrerPolicy::NoReferrer);
    init.set_signal(Some(signal));
    let response: Response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|_| FetchError::Failed)?
        .dyn_into()
        .map_err(|_| FetchError::Failed)?;
    if !response.ok() {
        return Err(FetchError::Failed);
    }
    let declared = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|length| length.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit) {
        return Err(FetchError::Oversized);
    }
    let reader: ReadableStreamDefaultReader = response
        .body()
        .ok_or(FetchError::Failed)?
        .get_reader()
        .dyn_into()
        .map_err(|_| FetchError::Failed)?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| FetchError::Failed)?;
        if field(&chunk, "done").as_bool().unwrap_or(true) {
            return Ok(bytes);
        }
        let part = Uint8Array::new(&field(&chunk, "value"));
        if (bytes.len() as u64).saturating_add(u64::from(part.length())) > limit {
            // Stop the download: the rest is not wanted.
            let _ = reader.cancel();
            return Err(FetchError::Oversized);
        }
        bytes.extend_from_slice(&part.to_vec());
    }
}

fn field(object: &JsValue, name: &str) -> JsValue {
    Reflect::get(object.unchecked_ref::<Object>(), &JsValue::from_str(name))
        .unwrap_or(JsValue::UNDEFINED)
}
