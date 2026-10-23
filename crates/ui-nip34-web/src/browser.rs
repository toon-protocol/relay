//! [`Fetch`] over the browser's `fetch`.

use js_sys::{Object, Reflect, Uint8Array};
use wasm_bindgen::{JsCast, JsValue, closure::Closure};
use wasm_bindgen_futures::JsFuture;
use web_sys::{
    AbortController, AbortSignal, ReadableStreamDefaultReader, ReferrerPolicy, RequestCredentials,
    RequestInit, RequestMode, Response, Window,
};

use crate::object::{Error, Fetch};

/// How long one request may take, body included.
const TIMEOUT_MS: i32 = 15_000;

/// How long to wait before asking again after a request got no answer.
const RETRY_MS: i32 = 500;

pub(crate) struct Browser;

/// Why a request failed: with no answer at all (the network dropped, or
/// changed under it), which is worth asking once more, or anything else.
enum Failure {
    NoAnswer,
    Final(Error),
}

impl From<Error> for Failure {
    fn from(error: Error) -> Self {
        Self::Final(error)
    }
}

impl Fetch for Browser {
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, Error> {
        let window = web_sys::window().ok_or(Error::Failed)?;
        match attempt(&window, url, limit).await {
            Err(Failure::NoAnswer) => {
                // A blip, not an answer: one more try. An object that is
                // missing, wrong or too big was answered, and is not retried.
                sleep(&window, RETRY_MS).await;
                match attempt(&window, url, limit).await {
                    Ok(bytes) => Ok(bytes),
                    Err(Failure::NoAnswer) => Err(Error::Failed),
                    Err(Failure::Final(error)) => Err(error),
                }
            }
            Err(Failure::Final(error)) => Err(error),
            Ok(bytes) => Ok(bytes),
        }
    }
}

/// Resolves after `ms`.
async fn sleep(window: &Window, ms: i32) {
    let promise = js_sys::Promise::new(&mut |resolve, _| {
        let _ = window.set_timeout_with_callback_and_timeout_and_arguments_0(&resolve, ms);
    });
    let _ = JsFuture::from(promise).await;
}

/// One request, abandoned after [`TIMEOUT_MS`].
async fn attempt(window: &Window, url: &str, limit: u64) -> Result<Vec<u8>, Failure> {
    let controller = AbortController::new().map_err(|_| Error::Failed)?;
    let abort = controller.clone();
    let on_timeout = Closure::once_into_js(move || abort.abort());
    let timer = window
        .set_timeout_with_callback_and_timeout_and_arguments_0(
            on_timeout.unchecked_ref(),
            TIMEOUT_MS,
        )
        .map_err(|_| Error::Failed)?;
    let signal = controller.signal();
    let result = read(window, url, limit, &signal).await;
    window.clear_timeout_with_handle(timer);
    match result {
        // A request the timer aborted is final: it already had its time.
        Err(Failure::NoAnswer) if signal.aborted() => Err(Failure::Final(Error::Failed)),
        other => other,
    }
}

async fn read(
    window: &Window,
    url: &str,
    limit: u64,
    signal: &AbortSignal,
) -> Result<Vec<u8>, Failure> {
    let init = RequestInit::new();
    init.set_method("GET");
    init.set_mode(RequestMode::Cors);
    init.set_credentials(RequestCredentials::Omit);
    init.set_referrer_policy(ReferrerPolicy::NoReferrer);
    init.set_signal(Some(signal));
    // `fetch` rejects only when no response came back at all.
    let response: Response = JsFuture::from(window.fetch_with_str_and_init(url, &init))
        .await
        .map_err(|_| Failure::NoAnswer)?
        .dyn_into()
        .map_err(|_| Error::Failed)?;
    if !response.ok() {
        return Err(Error::Failed.into());
    }
    let declared = response
        .headers()
        .get("content-length")
        .ok()
        .flatten()
        .and_then(|length| length.parse::<u64>().ok());
    if declared.is_some_and(|length| length > limit) {
        return Err(Error::Oversized.into());
    }
    let reader: ReadableStreamDefaultReader = response
        .body()
        .ok_or(Error::Failed)?
        .get_reader()
        .dyn_into()
        .map_err(|_| Error::Failed)?;
    let mut bytes = Vec::new();
    loop {
        let chunk = JsFuture::from(reader.read())
            .await
            .map_err(|_| Error::Failed)?;
        if field(&chunk, "done").as_bool().unwrap_or(true) {
            return Ok(bytes);
        }
        let part = Uint8Array::new(&field(&chunk, "value"));
        if (bytes.len() as u64).saturating_add(u64::from(part.length())) > limit {
            // Stop the download: the rest is not wanted.
            let _ = reader.cancel();
            return Err(Error::Oversized.into());
        }
        bytes.extend_from_slice(&part.to_vec());
    }
}

fn field(object: &JsValue, name: &str) -> JsValue {
    Reflect::get(object.unchecked_ref::<Object>(), &JsValue::from_str(name))
        .unwrap_or(JsValue::UNDEFINED)
}
