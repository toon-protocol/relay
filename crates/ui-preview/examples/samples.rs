//! Draws one card of each face into a folder, to look at while changing the
//! drawing: `cargo run -p ui-preview --example samples -- <dir>`.

use nostr::key::PublicKey;
use relay_ui::chrome::Identicon;
use ui_preview::{Article, Card, Face, Profile, Repo};

fn main() {
    let dir = std::env::args()
        .nth(1)
        .unwrap_or_else(|| "preview-samples".to_owned());
    if let Err(error) = std::fs::create_dir_all(&dir) {
        eprintln!("{dir}: {error}");
        std::process::exit(1);
    }
    let host = Some("relay.toonprotocol.dev".to_owned());
    let avatar = || {
        let key = "9955302dd51d10db5c05756ce1647708addd9c1daf10500b7d6d52c01bac10e5";
        Identicon::of(&PublicKey::from_hex(key).expect("a key"))
    };
    let repo = Repo {
        owner: "Maya Tide".into(),
        avatar: avatar(),
        name: "harmonics".into(),
        description: Some(
            "A tiny synthesizer that runs in the browser: oscillators, envelopes and a \
             sequencer, written in Rust and compiled to WebAssembly. No samples, no servers, \
             just math and a speaker."
                .into(),
        ),
        topics: vec![
            "rust".into(),
            "wasm".into(),
            "audio".into(),
            "synth".into(),
            "music".into(),
        ],
        stars: 1_284,
        followers: 37,
        forks: Some(5),
    };
    let cards = [
        ("repo", Card::new(host.clone(), Face::Repo(repo.clone()))),
        (
            "repo-long",
            Card::new(
                host.clone(),
                Face::Repo(Repo {
                    owner: "npub1qqqqqqqq…".into(),
                    avatar: avatar(),
                    name: "an-extraordinarily-long-repository-name-that-will-not-fit".into(),
                    description: None,
                    topics: vec![],
                    stars: 0,
                    followers: 1,
                    forks: None,
                }),
            ),
        ),
        (
            "ship",
            Card::new(
                host.clone(),
                Face::Ship {
                    headline: "harmonics 1.0 is out: polyphony, a step sequencer and MIDI input"
                        .into(),
                    repo: repo.clone(),
                },
            ),
        ),
        (
            "profile",
            Card::new(
                host.clone(),
                Face::Profile(Profile {
                    avatar: avatar(),
                    name: "Maya Tide".into(),
                    nip05: Some("maya@tide.dev".into()),
                    about: Some(
                        "Builds small instruments and the tools to make them. Rust, WebAssembly, \
                         Nostr. Shipping one thing a month in public."
                            .into(),
                    ),
                    repos: 12,
                    followers: 1,
                    bot: true,
                    ..Profile::default()
                }),
            ),
        ),
        (
            "article",
            Card::new(
                host.clone(),
                Face::Article(Article {
                    title: "How harmonics schedules notes without a clock thread".into(),
                    author: "Maya Tide".into(),
                    summary: Some("Audio worklets, a ring buffer and one lookahead loop.".into()),
                }),
            ),
        ),
        (
            "site",
            Card::new(
                host,
                Face::Site {
                    title: "Social".into(),
                    tagline: "Builders sharing repos, launches and devlogs, read from this \
                              relay's Nostr events."
                        .into(),
                },
            ),
        ),
    ];
    for (name, card) in cards {
        let started = std::time::Instant::now();
        match card.png() {
            Ok(bytes) => {
                let path = format!("{dir}/{name}.png");
                if let Err(error) = std::fs::write(&path, &bytes) {
                    eprintln!("{path}: {error}");
                    std::process::exit(1);
                }
                println!("{path}: {} bytes in {:?}", bytes.len(), started.elapsed());
            }
            Err(error) => {
                eprintln!("{name}: {error}");
                std::process::exit(1);
            }
        }
    }
}
