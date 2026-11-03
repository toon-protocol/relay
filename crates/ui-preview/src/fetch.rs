//! Fetching the pictures a profile card draws (ADR 0011): the one outbound
//! request the relay makes, and only from a card route.
//!
//! A URL comes from a kind 0 anyone can write, so each request is held to
//! what a picture needs and nothing more:
//!
//! - `https` on port 443 only, with no user info, to a host name or address
//!   that resolves to public addresses alone (every address is checked, and
//!   the connection goes to a checked one, so DNS cannot swap it after);
//! - at most [`MAX_REDIRECTS`] redirects, each checked the same way;
//! - [`TIMEOUT`] for the whole of it, [`MAX_BYTES`] of body, and
//!   [`FETCHES`] at once;
//! - the answer kept only if its bytes are a [`Picture`].
//!
//! Every answer, a failure too, is cached for a while, in a cache bounded in
//! entries and bytes.

use std::collections::HashMap;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr, SocketAddr};
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};
use std::time::{Duration, Instant};

use http_body_util::{BodyExt, Empty, Limited};
use hyper::body::Bytes;
use hyper::header::{ACCEPT, HOST, LOCATION, USER_AGENT};
use hyper::{Request, StatusCode};
use hyper_util::rt::TokioIo;
use tokio::net::TcpStream;
use tokio::sync::Semaphore;
use tokio_rustls::TlsConnector;
use tokio_rustls::rustls::pki_types::ServerName;
use tokio_rustls::rustls::{ClientConfig, RootCertStore};

use crate::Picture;

/// How long one picture may take, redirects and body included.
pub const TIMEOUT: Duration = Duration::from_secs(5);

/// The most body bytes a picture may be.
pub const MAX_BYTES: usize = 5 * 1024 * 1024;

/// The most redirects followed.
pub const MAX_REDIRECTS: usize = 3;

/// How many pictures are fetched at once.
pub const FETCHES: usize = 4;

/// How long a fetched picture is kept, and a failure remembered.
const FRESH: Duration = Duration::from_secs(60 * 60);
const FAILED: Duration = Duration::from_secs(10 * 60);

/// The cache's bounds.
const CACHE_ENTRIES: usize = 512;
const CACHE_BYTES: usize = 64 * 1024 * 1024;

/// What the relay says it is.
const AGENT: &str = "TOON-relay-preview/1 (+https://toonprotocol.dev)";

/// Fetched pictures, by URL. Cheap to clone: clones share the cache and the
/// limit on fetches.
#[derive(Clone)]
pub struct Pictures {
    cache: Arc<Mutex<Cache>>,
    fetching: Arc<Semaphore>,
}

impl Default for Pictures {
    fn default() -> Self {
        Self {
            cache: Arc::default(),
            fetching: Arc::new(Semaphore::new(FETCHES)),
        }
    }
}

impl std::fmt::Debug for Pictures {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.debug_struct("Pictures").finish_non_exhaustive()
    }
}

impl Pictures {
    /// The picture at `url`, from the cache or fetched now, or none when it
    /// could not be had within the bounds.
    pub async fn get(&self, url: &str) -> Option<Picture> {
        if let Some(found) = self.lock().get(url, Instant::now()) {
            return found;
        }
        let picture = tokio::time::timeout(TIMEOUT, async {
            let _turn = self.fetching.acquire().await.ok()?;
            fetch(url).await
        })
        .await
        .ok()
        .flatten();
        self.lock().put(url, picture.clone(), Instant::now());
        picture
    }

    fn lock(&self) -> MutexGuard<'_, Cache> {
        self.cache
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }
}

struct Entry {
    picture: Option<Picture>,
    until: Instant,
}

#[derive(Default)]
struct Cache {
    entries: HashMap<String, Entry>,
    bytes: usize,
}

impl Cache {
    fn get(&mut self, url: &str, now: Instant) -> Option<Option<Picture>> {
        let entry = self.entries.get(url)?;
        if entry.until > now {
            return Some(entry.picture.clone());
        }
        self.remove(url);
        None
    }

    fn remove(&mut self, url: &str) {
        if let Some(entry) = self.entries.remove(url) {
            self.bytes -= entry.picture.as_ref().map_or(0, Picture::len);
        }
    }

    fn put(&mut self, url: &str, picture: Option<Picture>, now: Instant) {
        self.remove(url);
        let size = picture.as_ref().map_or(0, Picture::len);
        if size > CACHE_BYTES {
            return;
        }
        while self.entries.len() >= CACHE_ENTRIES || self.bytes + size > CACHE_BYTES {
            // The one that would go stale first goes.
            let Some(oldest) = self
                .entries
                .iter()
                .min_by_key(|(_, entry)| entry.until)
                .map(|(url, _)| url.clone())
            else {
                break;
            };
            self.remove(&oldest);
        }
        let until = now + if picture.is_some() { FRESH } else { FAILED };
        self.bytes += size;
        self.entries
            .insert(url.to_owned(), Entry { picture, until });
    }
}

/// An `https` URL a picture may be fetched from: a host and the path asked.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Target {
    host: String,
    /// From `/`, the query kept and the fragment left off.
    path: String,
}

impl Target {
    /// `url` when it is `https`, names a host on port 443 and has no user
    /// info. A bracketed IPv6 literal is refused.
    fn parse(url: &str) -> Option<Self> {
        let rest = url.trim().strip_prefix("https://")?;
        let end = rest.find(['/', '?', '#']).unwrap_or(rest.len());
        let authority = &rest[..end];
        let mut path = rest[end..].split('#').next().unwrap_or_default().to_owned();
        if !path.starts_with('/') {
            path.insert(0, '/');
        }
        if path.bytes().any(|byte| byte <= b' ' || byte == 0x7f) {
            return None;
        }
        let host = match authority.split_once(':') {
            Some((host, "443")) => host,
            Some(_) => return None,
            None => authority,
        };
        let host = host.trim_end_matches('.').to_ascii_lowercase();
        let valid = !host.is_empty()
            && host.len() <= 253
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'.');
        valid.then_some(Self { host, path })
    }

    /// Where a redirect's `Location` goes from here, if it is `https`.
    fn follow(&self, location: &str) -> Option<Self> {
        if location.starts_with("https://") {
            return Self::parse(location);
        }
        if let Some(rest) = location.strip_prefix("//") {
            return Self::parse(&format!("https://{rest}"));
        }
        if location.starts_with('/') {
            return Self::parse(&format!("https://{}{location}", self.host));
        }
        None
    }
}

/// Whether `ip` is an address on the public internet: not loopback, private,
/// link-local, shared, reserved, documentation or multicast, and no IPv6
/// form of one.
fn public(ip: IpAddr) -> bool {
    match ip {
        IpAddr::V4(ip) => public_v4(ip),
        IpAddr::V6(ip) => public_v6(ip),
    }
}

fn public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_broadcast()
        || ip.is_documentation()
        || ip.is_unspecified()
        || ip.is_multicast()
        || a == 0
        || (a == 100 && (b & 0xc0) == 64)
        || (a == 192 && b == 0 && c == 0)
        || (a == 198 && (b & 0xfe) == 18)
        || a >= 240)
}

fn public_v6(ip: Ipv6Addr) -> bool {
    if let Some(v4) = ip.to_ipv4_mapped() {
        return public_v4(v4);
    }
    let s = ip.segments();
    !(ip.is_loopback()
        || ip.is_unspecified()
        || ip.is_multicast()
        // ::/96 (IPv4-compatible) and the rest of ::/80.
        || s[..5] == [0; 5]
        // Unique local and link-local.
        || (s[0] & 0xfe00) == 0xfc00
        || (s[0] & 0xffc0) == 0xfe80
        // NAT64, 6to4 and Teredo, which carry an IPv4 address inside.
        || (s[0] == 0x64 && s[1] == 0xff9b)
        || s[0] == 0x2002
        || (s[0] == 0x2001 && s[1] < 0x0200)
        // Documentation.
        || (s[0] == 0x2001 && s[1] == 0x0db8))
}

fn tls() -> TlsConnector {
    static CONNECTOR: OnceLock<TlsConnector> = OnceLock::new();
    CONNECTOR
        .get_or_init(|| {
            let roots = RootCertStore {
                roots: webpki_roots::TLS_SERVER_ROOTS.to_vec(),
            };
            let provider = Arc::new(tokio_rustls::rustls::crypto::ring::default_provider());
            let config = ClientConfig::builder_with_provider(provider)
                .with_safe_default_protocol_versions()
                .expect("ring supports the default protocol versions")
                .with_root_certificates(roots)
                .with_no_client_auth();
            TlsConnector::from(Arc::new(config))
        })
        .clone()
}

/// What one request answered.
enum Answer {
    Body(Vec<u8>),
    Redirect(String),
}

async fn fetch(url: &str) -> Option<Picture> {
    let mut target = Target::parse(url)?;
    for _ in 0..=MAX_REDIRECTS {
        match request(&target).await? {
            Answer::Body(bytes) => return Picture::new(bytes),
            Answer::Redirect(location) => target = target.follow(&location)?,
        }
    }
    None
}

/// The public addresses `host` resolves to, or none if any is not public.
async fn resolve(host: &str) -> Option<Vec<SocketAddr>> {
    let addresses: Vec<SocketAddr> = tokio::net::lookup_host((host, 443)).await.ok()?.collect();
    (!addresses.is_empty() && addresses.iter().all(|address| public(address.ip())))
        .then_some(addresses)
}

async fn request(target: &Target) -> Option<Answer> {
    let addresses = resolve(&target.host).await?;
    let mut tcp = None;
    for address in addresses {
        if let Ok(stream) = TcpStream::connect(address).await {
            tcp = Some(stream);
            break;
        }
    }
    let name = ServerName::try_from(target.host.clone()).ok()?;
    let stream = tls().connect(name, tcp?).await.ok()?;
    let (mut sender, connection) = hyper::client::conn::http1::handshake(TokioIo::new(stream))
        .await
        .ok()?;
    tokio::spawn(connection);
    let request = Request::get(target.path.as_str())
        .header(HOST, target.host.as_str())
        .header(USER_AGENT, AGENT)
        .header(ACCEPT, "image/png,image/jpeg,image/gif,image/webp")
        .body(Empty::<Bytes>::new())
        .ok()?;
    let response = sender.send_request(request).await.ok()?;
    let status = response.status();
    if matches!(
        status,
        StatusCode::MOVED_PERMANENTLY
            | StatusCode::FOUND
            | StatusCode::SEE_OTHER
            | StatusCode::TEMPORARY_REDIRECT
            | StatusCode::PERMANENT_REDIRECT
    ) {
        let location = response.headers().get(LOCATION)?.to_str().ok()?;
        return Some(Answer::Redirect(location.to_owned()));
    }
    if status != StatusCode::OK {
        return None;
    }
    let body = Limited::new(response.into_body(), MAX_BYTES)
        .collect()
        .await
        .ok()?;
    Some(Answer::Body(body.to_bytes().to_vec()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::picture::tests::png;

    #[test]
    fn only_https_on_443_to_a_plain_host_is_a_target() {
        assert_eq!(
            Target::parse("https://Img.Test/a/b.png?s=1#x"),
            Some(Target {
                host: "img.test".into(),
                path: "/a/b.png?s=1".into()
            })
        );
        assert_eq!(
            Target::parse("https://img.test:443").map(|t| t.path),
            Some("/".into())
        );
        for refused in [
            "http://img.test/a.png",
            "https://img.test:8443/a.png",
            "https://user@img.test/a.png",
            "https://[::1]/a.png",
            "https:///a.png",
            "https://img.test/a b.png",
            "ftp://img.test/a.png",
            "data:image/png;base64,AAAA",
        ] {
            assert_eq!(Target::parse(refused), None, "{refused}");
        }
    }

    #[test]
    fn a_redirect_is_followed_only_to_https() {
        let from = Target::parse("https://a.test/x").expect("a target");
        assert_eq!(
            from.follow("/y").map(|t| (t.host, t.path)),
            Some(("a.test".into(), "/y".into()))
        );
        assert_eq!(
            from.follow("//b.test/z").map(|t| t.host),
            Some("b.test".into())
        );
        assert_eq!(
            from.follow("https://c.test/").map(|t| t.host),
            Some("c.test".into())
        );
        assert_eq!(from.follow("http://c.test/"), None);
        assert_eq!(from.follow("y"), None);
    }

    #[test]
    fn only_public_addresses_are_public() {
        for private in [
            "127.0.0.1",
            "10.1.2.3",
            "172.17.0.2",
            "192.168.1.1",
            "169.254.169.254",
            "100.64.0.1",
            "0.0.0.0",
            "0.1.2.3",
            "192.0.0.8",
            "198.18.0.1",
            "224.0.0.1",
            "240.0.0.1",
            "255.255.255.255",
            "192.0.2.1",
            "::1",
            "::",
            "::ffff:127.0.0.1",
            "::ffff:10.0.0.1",
            "::127.0.0.1",
            "fc00::1",
            "fd12::1",
            "fe80::1",
            "64:ff9b::7f00:1",
            "2002:7f00:1::",
            "2001:0:4136::1",
            "2001:db8::1",
            "ff02::1",
        ] {
            let ip: IpAddr = private.parse().expect("an address");
            assert!(!public(ip), "{private}");
        }
        for open in [
            "1.1.1.1",
            "93.184.216.34",
            "2606:4700::1111",
            "::ffff:8.8.8.8",
        ] {
            let ip: IpAddr = open.parse().expect("an address");
            assert!(public(ip), "{open}");
        }
    }

    #[tokio::test]
    async fn a_host_that_resolves_to_loopback_is_never_asked() {
        assert_eq!(resolve("localhost").await, None);
        assert_eq!(resolve("127.0.0.1").await, None);
        let pictures = Pictures::default();
        assert!(pictures.get("https://localhost/a.png").await.is_none());
        assert!(pictures.get("https://10.0.0.1/a.png").await.is_none());
    }

    #[test]
    fn the_cache_is_bounded_and_forgets_failures_sooner() {
        let mut cache = Cache::default();
        let now = Instant::now();
        let picture = Picture::new(png(4, 4));
        cache.put("a", picture.clone(), now);
        cache.put("b", None, now);
        assert_eq!(cache.get("a", now), Some(picture.clone()));
        assert_eq!(cache.get("b", now), Some(None));
        let later = now + FAILED + Duration::from_secs(1);
        assert_eq!(cache.get("b", later), None, "a failure is asked again");
        assert_eq!(cache.get("a", later), Some(picture));
        for n in 0..CACHE_ENTRIES + 10 {
            cache.put(&n.to_string(), None, now);
        }
        assert!(cache.entries.len() <= CACHE_ENTRIES);
        let bytes: usize = cache
            .entries
            .values()
            .map(|entry| entry.picture.as_ref().map_or(0, Picture::len))
            .sum();
        assert_eq!(cache.bytes, bytes);
    }
}
