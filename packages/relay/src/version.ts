/**
 * Package version, surfaced on `GET /health`.
 *
 * `__RELAY_VERSION__` is replaced at build time with `package.json`'s
 * `version` -- by tsup for the shipped bundle, and by vitest for the tests
 * (both configs read the same file).
 *
 * It is injected rather than written down here because `changeset version`
 * bumps `package.json` and nothing else. A hand-maintained copy in the source
 * drifts the moment a release is cut, and then `/health` reports a version the
 * image is not running -- which is exactly what happened before this: a
 * hardcoded `0.1.0` served by a shipped `2.0.2`.
 */
declare const __RELAY_VERSION__: string;

export const VERSION: string = __RELAY_VERSION__;
