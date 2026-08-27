/**
 * Package version, surfaced on `GET /health`.
 *
 * Kept in lockstep with `packages/relay/package.json` by
 * `version.test.ts`, which fails the build if the two ever disagree — a
 * `/health` that reports a version the image is not running is worse than
 * no version at all.
 */
export const VERSION = '2.0.2';
