//! A fake network for the native tests, so the fetching and verifying logic
//! runs under `cargo test` without a browser.

use std::collections::BTreeMap;

use crate::{Error, Fetch};

/// A table of URL to bytes; anything else fails.
#[derive(Default)]
pub(crate) struct Table(pub(crate) BTreeMap<String, Vec<u8>>);

impl Fetch for Table {
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, Error> {
        let bytes = self.0.get(url).ok_or(Error::Failed)?;
        if bytes.len() as u64 > limit {
            return Err(Error::Oversized);
        }
        Ok(bytes.clone())
    }
}
