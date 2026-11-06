//! The Listing page's "Use with an agent" box: a script-free GET form, and
//! the request a person copies to their agent. The relay only does the
//! arithmetic and the text; every number comes from the Listing.

use nostr::event::Kind;
use nostr::nips::nip01::Coordinate;
use nostr::nips::nip19::{Nip19Coordinate, ToBech32};
use relay_ui::chrome::{escape, param};

use crate::directory::{LISTING_KIND, Listing};

/// The longest lease the box writes a request for: 30 days.
const MAX_SECONDS: u64 = 2_592_000;

/// What was asked for.
enum Duration {
    Hours(u64),
    Intervals(u64),
}

/// A lease worked out from a Listing and a duration.
struct Lease {
    intervals: u64,
    total: u64,
    standby: Option<u64>,
}

/// A whole positive number; `None` for anything else.
fn whole(text: &str) -> Option<u64> {
    let text = text.trim();
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok().filter(|n| *n > 0)
}

/// The lease `duration` is on `listing`, or the note saying why there is none.
fn work_out(listing: &Listing, duration: &Duration) -> Result<Lease, &'static str> {
    let (Some(price), Some(interval)) =
        (listing.price, listing.lease_interval_s.filter(|s| *s > 0))
    else {
        return Err("This Listing states no price or interval to work from.");
    };
    let too_long = "A lease runs from one Lease Interval to 30 days.";
    let too_large = "That lease costs more than can be worked out.";
    let intervals = match *duration {
        Duration::Hours(h) if h > MAX_SECONDS / 3600 => return Err(too_long),
        Duration::Hours(h) => (h * 3600).div_ceil(interval),
        Duration::Intervals(n) => match n.checked_mul(interval) {
            Some(seconds) if seconds <= MAX_SECONDS => n,
            _ => return Err(too_long),
        },
    };
    let total = price.checked_mul(intervals).ok_or(too_large)?;
    let standby = match listing.standby_price {
        Some(p) => Some(p.checked_mul(intervals).ok_or(too_large)?),
        None => None,
    };
    Ok(Lease {
        intervals,
        total,
        standby,
    })
}

/// A Lease Interval's length, in minutes when it is whole minutes.
fn interval_text(seconds: u64) -> String {
    match seconds % 60 {
        0 => format!("{} min", seconds / 60),
        _ => format!("{seconds} s"),
    }
}

/// The Listing's own `naddr`, with no relay hints.
fn naddr(listing: &Listing) -> Option<String> {
    let coordinate =
        Coordinate::new(Kind::from(LISTING_KIND), listing.provider).identifier(listing.d.clone());
    Nip19Coordinate::new(coordinate, []).to_bech32().ok()
}

/// The duration the query asks for: nothing, one, or the note why not.
fn asked(query: Option<&str>) -> Result<Option<Duration>, &'static str> {
    let given = |name: &str| param(query, name).filter(|v| !v.trim().is_empty());
    let invalid = "Give hours or intervals as a whole number above zero.";
    match (given("hours"), given("intervals")) {
        (None, None) => Ok(None),
        (Some(_), Some(_)) => Err("Give hours or intervals, not both."),
        (Some(h), None) => whole(&h).map(Duration::Hours).map(Some).ok_or(invalid),
        (None, Some(n)) => whole(&n).map(Duration::Intervals).map(Some).ok_or(invalid),
    }
}

/// The "Use with an agent" section of `listing`'s page.
pub(crate) fn section(
    listing: &Listing,
    provider: &str,
    query: Option<&str>,
    action: &str,
) -> String {
    let text = |name: &str| {
        param(query, name)
            .map(|v| v.trim().to_owned())
            .unwrap_or_default()
    };
    let worked = asked(query).and_then(|duration| match duration {
        None => Ok(None),
        Some(duration) => {
            let lease = work_out(listing, &duration)?;
            let naddr = naddr(listing).ok_or("This Listing has no address to name.")?;
            Ok(Some((duration, lease, naddr)))
        }
    });
    // A request echoes what it was worked out from; anything else leaves
    // the form empty.
    let shown = |name: &str| match worked {
        Ok(Some(_)) => text(name),
        _ => String::new(),
    };
    let field = |name: &str, label: &str| {
        format!(
            "<label class=\"small muted\">{label} <input type=\"text\" name=\"{name}\" \
             value=\"{}\"></label> ",
            escape(&shown(name))
        )
    };
    let mut out = format!(
        "<h2>Use with an agent</h2>\
         <form method=\"get\" action=\"{}\" class=\"box\" data-agent-form>{}{}{}{}\
         <button type=\"submit\">Work out request</button></form>",
        escape(action),
        field("hours", "Hours"),
        field("intervals", "or intervals"),
        field("image", "Image"),
        field("port", "Port"),
    );
    if !listing.live {
        out.push_str(
            "<p class=\"muted small\" data-agent-down>This Listing's Provider is down: a spawn \
             would fail until the Provider is live again.</p>",
        );
    }
    let (duration, lease, naddr) = match worked {
        Ok(Some(worked)) => worked,
        Ok(None) => return out,
        Err(note) => {
            out.push_str(&format!(
                "<p class=\"muted small\" data-agent-note>{note}</p>"
            ));
            return out;
        }
    };
    let interval = listing
        .lease_interval_s
        .map(interval_text)
        .unwrap_or_default();
    let intervals = |n: u64| match n {
        1 => format!("1 × {interval} Lease Interval"),
        _ => format!("{n} × {interval} Lease Intervals"),
    };
    let phrase = match duration {
        Duration::Hours(h) => format!(
            "{h} {} ({})",
            if h == 1 { "hour" } else { "hours" },
            intervals(lease.intervals)
        ),
        Duration::Intervals(n) => intervals(n),
    };
    let or = |value: String, placeholder: &str| match value.is_empty() {
        true => placeholder.to_owned(),
        false => value,
    };
    let request = format!(
        "Run `{}` on Listing `{naddr}` ({} / {}) for {phrase}, port {}, at most {} µUSDC. \
         Extend only while the job is still running; end the lease when it finishes.",
        or(text("image"), "<image>"),
        provider,
        listing.d,
        or(text("port"), "<port>"),
        lease.total,
    );
    out.push_str(&format!(
        "<p class=\"small\" data-agent-total>{} Lease Intervals, {} µUSDC in total{}.</p>\
         <pre data-agent-request>{}</pre>",
        lease.intervals,
        lease.total,
        lease
            .standby
            .map(|s| format!("; {s} µUSDC for one Warm Standby"))
            .unwrap_or_default(),
        escape(&request),
    ));
    out
}
