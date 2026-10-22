//! A fake network for the native tests.

use std::collections::BTreeMap;

use crate::{Fetch, FetchError};

/// A table of URL to bytes; anything else fails.
pub(crate) struct Table(pub(crate) BTreeMap<String, Vec<u8>>);

impl Fetch for Table {
    async fn get(&self, url: &str, limit: u64) -> Result<Vec<u8>, FetchError> {
        if !url.starts_with("https://") {
            return Err(FetchError::NotHttps);
        }
        let bytes = self.0.get(url).ok_or(FetchError::Failed)?;
        if bytes.len() as u64 > limit {
            return Err(FetchError::Oversized);
        }
        Ok(bytes.clone())
    }
}
