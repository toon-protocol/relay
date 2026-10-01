//! The store accepts a verified event and nothing else.

use nostr::event::Event;
use relay::Store;

async fn save(store: &Store, event: &Event) {
    let _ = store.save(event).await;
}

fn main() {}
