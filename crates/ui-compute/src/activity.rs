//! A Provider's activity log: its Eviction Notices, the Takeovers naming it
//! and the Deployments naming it, newest first. The store keeps only the
//! newest Liveness, Profile and Listing, so there is no uptime or price
//! history.

use std::collections::{HashMap, HashSet};

use nostr::event::{Event, Kind};
use nostr::filter::Filter;
use nostr::key::PublicKey;
use relay_ui::chrome::{self, Profiles, escape};

use crate::Pages;
use crate::deployment::{
    Deployment, EVICTION_KIND, Standing, TAKEOVER_KIND, deployment_path, note, notes_html,
    tag_slices, valid_takeovers,
};
use crate::provider::provider_link;

/// One line of the log.
enum Entry {
    Eviction {
        reason: Option<String>,
        message: Option<String>,
    },
    Takeover {
        winner: PublicKey,
        primary: PublicKey,
    },
    Deployment(Box<Deployment>),
}

/// The entries for `provider`, newest first, as `(created_at, entry)`. An
/// Eviction Notice counts only with its `x` workload id; a Takeover is listed
/// once, however many of the Deployments share its workload id.
fn entries(
    provider: &PublicKey,
    deployments: &[Deployment],
    evictions: &[Event],
    takeovers: &[Event],
) -> Vec<(u64, Entry)> {
    let mut log: Vec<(u64, Entry)> = evictions
        .iter()
        .filter(|e| e.kind == Kind::from(EVICTION_KIND) && e.pubkey == *provider)
        .filter(|e| tag_slices(e).any(|tag| matches!(tag, [name, _, ..] if name == "x")))
        .map(|e| {
            (
                e.created_at.as_secs(),
                Entry::Eviction {
                    reason: note(e, "reason"),
                    message: note(e, "message"),
                },
            )
        })
        .collect();
    let mut listed = HashSet::new();
    for d in deployments.iter().filter(|d| d.names(provider)) {
        log.push((d.created_at, Entry::Deployment(Box::new(d.clone()))));
        let fresh = valid_takeovers(d, takeovers).filter(|e| listed.insert(e.id));
        log.extend(fresh.map(|e| {
            (
                e.created_at.as_secs(),
                Entry::Takeover {
                    winner: e.pubkey,
                    primary: d.primary,
                },
            )
        }));
    }
    log.sort_by_key(|(at, _)| std::cmp::Reverse(*at));
    log
}

/// The activity log section of a Provider page. `served` is
/// [`serving`](crate::deployment::serving)'s answer for the provider.
pub(crate) async fn section(
    pages: &Pages,
    provider: &PublicKey,
    served: &[(Deployment, Standing)],
) -> String {
    let deployments: Vec<Deployment> = served.iter().map(|(d, _)| d.clone()).collect();
    let workloads: Vec<String> = deployments.iter().map(|d| d.workload.clone()).collect();
    let (evictions, takeovers) = tokio::join!(
        pages.store.query(
            Filter::new()
                .kind(Kind::from(EVICTION_KIND))
                .author(*provider)
                .limit(crate::READ_LIMIT)
        ),
        async {
            match workloads.is_empty() {
                true => Vec::new(),
                false => {
                    pages
                        .store
                        .query(
                            Filter::new()
                                .kind(Kind::from(TAKEOVER_KIND))
                                .identifiers(workloads)
                                .limit(crate::READ_LIMIT),
                        )
                        .await
                }
            }
        }
    );
    let log = entries(provider, &deployments, &evictions, &takeovers);
    let mut body = String::from("<h2>Activity</h2>");
    if log.is_empty() {
        body.push_str("<div class=\"box\"><div class=\"empty\">No activity yet.</div></div>");
        return body;
    }
    let mut all: Vec<PublicKey> = log
        .iter()
        .flat_map(|(_, entry)| match entry {
            Entry::Takeover { winner, primary } => vec![*winner, *primary],
            Entry::Deployment(d) => d.keys(),
            Entry::Eviction { .. } => Vec::new(),
        })
        .collect();
    all.sort();
    all.dedup();
    let names: HashMap<PublicKey, String> = Profiles::load(&pages.store, &all).await.into_names();
    body.push_str(
        "<div class=\"box\"><div class=\"table-wrap\"><table class=\"data-table\" data-activity>\
         <thead><tr><th>When</th><th>Event</th><th>Details</th></tr></thead><tbody>",
    );
    for (at, entry) in &log {
        let when = escape(&chrome::date(i64::try_from(*at).unwrap_or(i64::MAX)));
        let (kind, details) = match entry {
            Entry::Eviction { reason, message } => (
                "eviction",
                notes_html(reason.as_deref(), message.as_deref()),
            ),
            Entry::Takeover { winner, primary } => (
                "takeover",
                format!(
                    "{} took over from {}",
                    provider_link(&names, winner),
                    provider_link(&names, primary)
                ),
            ),
            Entry::Deployment(d) => (
                "deployment",
                format!(
                    "<a href=\"{}\">{}</a> {}",
                    deployment_path(&d.signer, &d.environment),
                    escape(&d.environment),
                    chrome::name(&names, &d.signer)
                ),
            ),
        };
        body.push_str(&format!(
            "<tr data-activity-kind=\"{kind}\" data-at=\"{at}\"><td>{when}</td><td>{kind}</td>\
             <td>{details}</td></tr>"
        ));
    }
    body.push_str("</tbody></table></div></div>");
    body
}

#[cfg(test)]
mod tests {
    use super::*;
    use nostr::event::{EventBuilder, FinalizeEvent, Tag};
    use nostr::key::Keys;
    use nostr::types::Timestamp;

    fn at(keys: &Keys, kind: u16, content: &str, tags: &[&[&str]], secs: u64) -> Event {
        EventBuilder::new(Kind::from(kind), content)
            .tags(tags.iter().map(|t| Tag::parse(t.to_vec()).expect("tag")))
            .custom_created_at(Timestamp::from_secs(secs))
            .finalize(keys)
            .expect("event")
    }

    #[test]
    fn other_providers_events_do_not_appear_and_an_unknown_reason_is_kept() {
        let (me, other) = (Keys::generate(), Keys::generate());
        let log = entries(
            &me.public_key(),
            &[],
            &[
                at(
                    &me,
                    4_433,
                    r#"{"reason":"zzz","message":"m"}"#,
                    &[&["x", "w1"]],
                    10,
                ),
                at(&other, 4_433, r#"{"reason":"abuse"}"#, &[&["x", "w2"]], 20),
                at(&me, 4_433, r#"{"reason":"policy"}"#, &[&["x", "w3"]], 30),
                // No `x` workload id: not an Eviction Notice.
                at(&me, 4_433, r#"{"reason":"policy"}"#, &[], 40),
            ],
            &[],
        );
        let times: Vec<u64> = log.iter().map(|(t, _)| *t).collect();
        assert_eq!(times, [30, 10]);
        assert!(matches!(
            &log[1].1,
            Entry::Eviction { reason: Some(r), message: Some(m) } if r == "zzz" && m == "m"
        ));
    }
}
