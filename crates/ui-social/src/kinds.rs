//! The event kinds `/social` reads, by the NIP that defines each.

/// NIP-01 short text note.
pub(crate) const NOTE: u16 = 1;
/// NIP-02 follow list.
pub(crate) const FOLLOWS: u16 = 3;
/// NIP-25 reaction.
pub(crate) const REACTION: u16 = 7;
/// NIP-7D thread.
pub(crate) const THREAD: u16 = 11;
/// NIP-18 generic repost.
pub(crate) const GENERIC_REPOST: u16 = 16;
/// NIP-68 picture.
pub(crate) const PICTURE: u16 = 20;
/// NIP-71 video.
pub(crate) const VIDEO: u16 = 21;
/// NIP-71 short video.
pub(crate) const SHORT_VIDEO: u16 = 22;
/// NIP-22 comment.
pub(crate) const COMMENT: u16 = 1_111;
/// NIP-C0 code snippet.
pub(crate) const SNIPPET: u16 = 1_337;
/// NIP-34 pull request.
pub(crate) const PULL_REQUEST: u16 = 1_618;
/// NIP-34 issue.
pub(crate) const ISSUE: u16 = 1_621;
/// NIP-34 status: applied, merged or resolved.
pub(crate) const STATUS_APPLIED: u16 = 1_631;
/// NIP-56 report.
pub(crate) const REPORT: u16 = 1_984;
/// NIP-32 label.
pub(crate) const LABEL: u16 = 1_985;
/// NIP-85 trusted service providers: whose assertions a key trusts.
pub(crate) const TRUSTED_PROVIDERS: u16 = 10_040;
/// NIP-85 trusted assertion about an addressable event (here, a repo).
pub(crate) const ADDRESS_ASSERTION: u16 = 30_384;
/// NIP-84 highlight.
pub(crate) const HIGHLIGHT: u16 = 9_802;
/// NIP-51 git authors: the code authors a key follows.
pub(crate) const GIT_AUTHORS: u16 = 10_017;
/// NIP-5A root site manifest.
pub(crate) const SITE_ROOT: u16 = 15_128;
/// NIP-23 long-form article.
pub(crate) const ARTICLE: u16 = 30_023;
/// NIP-89 handler recommendation.
pub(crate) const HANDLER_RECOMMENDATION: u16 = 31_989;
/// NIP-51 curation set of articles: a series, when its author keeps one.
pub(crate) const CURATION_SET: u16 = 30_004;
/// NIP-89 application handler; Shakespeare's app event.
pub(crate) const APP_HANDLER: u16 = 31_990;
/// Software application (nips#1336, the "NIP-82" draft: an open PR, not a
/// merged NIP; the kind is in the index's table with no NIP named).
pub(crate) const SOFTWARE_APP: u16 = 32_267;
/// NIP-5A named site manifest.
pub(crate) const SITE_NAMED: u16 = 35_128;

/// NIP-82 (draft, nostr-protocol/nips#1336) software asset.
pub(crate) const SOFTWARE_ASSET: u16 = 3_063;
/// NIP-75 zap goal.
pub(crate) const ZAP_GOAL: u16 = 9_041;
/// NIP-51 mute list.
pub(crate) const MUTE_LIST: u16 = 10_000;
/// NIP-82 software release (NIP-51's release artifact set kind).
pub(crate) const SOFTWARE_RELEASE: u16 = 30_063;
/// NIP-51 app curation set.
pub(crate) const APP_SET: u16 = 30_267;
/// NIP-99 classified listing.
pub(crate) const CLASSIFIED: u16 = 30_402;
/// The tip NIP's (`docs/nips/nip-tips.md`) tip acknowledgement, signed by the
/// recipient.
pub(crate) const TIP_ACKNOWLEDGEMENT: u16 = 9_851;
/// The tip NIP's tip route, a replaceable event the recipient signs.
pub(crate) const TIP_ROUTE: u16 = 10_850;

pub(crate) use ui_nip34::card::FOLLOWED_REPOS;
pub(crate) use ui_nip34::repo::ANNOUNCEMENT;

/// What a reader calls a kind, in a chip.
pub(crate) fn label(kind: u16) -> &'static str {
    match kind {
        NOTE => "note",
        THREAD => "thread",
        GENERIC_REPOST => "boost",
        PICTURE => "picture",
        VIDEO | SHORT_VIDEO => "video",
        COMMENT => "comment",
        SNIPPET => "snippet",
        HIGHLIGHT => "highlight",
        ARTICLE => "article",
        APP_HANDLER => "app",
        ISSUE => "issue",
        PULL_REQUEST => "pull request",
        ANNOUNCEMENT => "repository",
        _ => "event",
    }
}
