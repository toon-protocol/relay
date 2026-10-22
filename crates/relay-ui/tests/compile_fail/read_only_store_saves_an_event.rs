use relay_ui::ReadOnlyStore;

async fn app(store: ReadOnlyStore, event: nostr::event::Event) {
    store.save(&event).await;
}

fn main() {}
