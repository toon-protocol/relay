/**
 * Operator event blocklist — the escape hatch for litter that NEITHER NIP
 * can clear.
 *
 * WHY THIS EXISTS. NIP-01 replacement needs the author's key. NIP-09 deletion
 * needs the author's key. When a node publishes an announce and then the key
 * is lost — a throwaway proof rig wiped off an operator workstation, say —
 * the announce is unretractable BY CONSTRUCTION, and if it also carries no
 * NIP-40 `expiration` tag then nothing in the protocol will ever remove it.
 * That is exactly the state devnet was in: a kind:5094-era swap maker
 * advertising `g.toon.swap.sol` at `ws://127.0.0.1:3401` — a loopback address
 * that resolves to whatever machine READS it — with no expiry and no key.
 *
 * WHY IT IS SHAPED LIKE THIS. Any mechanism that lets an operator remove
 * other people's events is a censorship surface, so the scope is drawn as
 * narrowly as the job allows:
 *
 *   - **Event ids only, never pubkeys.** Blocking a pubkey silences an
 *     identity's entire past and future output with one line of config.
 *     Blocking a 64-hex id removes exactly one event that the operator had to
 *     name explicitly, having already seen it. A key that is still alive can
 *     simply publish again, so this cannot be used to suppress a live
 *     participant — only to sweep a specific dead artifact.
 *   - **Config at startup, not an API.** There is no admin endpoint, no
 *     authenticated mutation, nothing network-reachable. The list arrives as
 *     process configuration (`TOON_BLOCKED_EVENT_IDS`) and changing it means
 *     restarting the process with a changed deployment — an act that lands in
 *     a git history and a deploy log rather than in an unlogged HTTP call.
 *   - **Loud.** The launcher prints every blocked id at startup. A relay that
 *     is withholding events should say so on every boot.
 *
 * A blocked id is refused on write, filtered on read, and swept from the
 * database — but the block still lives only in ONE relay's configuration.
 * Other relays serving the same event are unaffected, which is the correct
 * outcome: this is an operator declining to carry a specific artifact, not a
 * protocol-level retraction.
 *
 * @module
 */

/** 64-char lowercase hex — the canonical wire form of an event id. */
const HEX_64 = /^[0-9a-f]{64}$/;

/**
 * Parse an operator blocklist from its configuration string.
 *
 * Accepts a comma- and/or whitespace-separated list of 64-char hex event ids.
 * Case is normalized to lowercase. Entries that are not well-formed event ids
 * are reported separately rather than silently dropped: a typo in a blocklist
 * must be visible, because its failure mode (an event the operator believes
 * is blocked but is still being served) is silent otherwise.
 *
 * @param raw - Raw configuration value, or undefined.
 * @returns The accepted ids and any rejected entries.
 */
export function parseBlockedEventIds(raw: string | undefined): {
  ids: string[];
  invalid: string[];
} {
  const ids = new Set<string>();
  const invalid: string[] = [];

  for (const entry of (raw ?? '').split(/[\s,]+/)) {
    if (entry === '') continue;
    const normalized = entry.toLowerCase();
    if (HEX_64.test(normalized)) {
      ids.add(normalized);
    } else {
      invalid.push(entry);
    }
  }

  return { ids: [...ids], invalid };
}
