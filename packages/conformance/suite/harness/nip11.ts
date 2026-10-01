/** The parts of the document the suite reads; anything else is `unknown`. */
export interface RelayDocument {
  pubkey?: string;
  supported_nips?: number[];
  limitation?: { payment_required?: boolean; restricted_writes?: boolean };
  toon?: { carriage?: string; price?: number };
  [key: string]: unknown;
}

export interface FetchedDocument {
  status: number;
  headers: Headers;
  body: RelayDocument;
}

/** GET the Relay Information Document from a read port. */
export async function getDocument(readUrl: string): Promise<FetchedDocument> {
  const response = await fetch(readUrl, {
    headers: { accept: 'application/nostr+json' },
  });
  return {
    status: response.status,
    headers: response.headers,
    body: (await response.json()) as RelayDocument,
  };
}

/**
 * Poll the document until `ready` holds. The relay reads its edge in the
 * background, so the first fetch after /health may predate it.
 */
export async function waitForDocument(
  readUrl: string,
  ready: (body: RelayDocument) => boolean,
  timeoutMs = 30_000
): Promise<RelayDocument> {
  const deadline = Date.now() + timeoutMs;
  let body: RelayDocument = {};
  do {
    body = (await getDocument(readUrl)).body;
    if (ready(body)) return body;
    await new Promise((resolve) => setTimeout(resolve, 250));
  } while (Date.now() < deadline);
  throw new Error(`document never became ready: ${JSON.stringify(body)}`);
}

/** The document once the relay has had every chance to learn its edge. */
export function waitForEdge(readUrl: string): Promise<RelayDocument> {
  return waitForDocument(readUrl, (body) => body['toon'] !== undefined);
}

/**
 * The document of a relay that must publish no edge. There is nothing to
 * poll for, so wait until the stub has been asked (when it can be) and then
 * give the relay time to have acted on the answer.
 */
export async function settledDocument(
  readUrl: string,
  asked?: () => number
): Promise<RelayDocument> {
  const deadline = Date.now() + 15_000;
  while (asked !== undefined && asked() === 0 && Date.now() < deadline) {
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  await new Promise((resolve) => setTimeout(resolve, 1_000));
  return (await getDocument(readUrl)).body;
}
