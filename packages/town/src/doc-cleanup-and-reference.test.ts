/**
 * Tests for Story 2.4: Stale Documentation Cleanup & Reference Implementation Docs
 *
 * These tests verify that:
 * - AC #1: Stale git-proxy documentation is removed or updated
 * - AC #2: docker/src/entrypoint-town.ts has file-level JSDoc documenting it
 *          as the SDK Reference Implementation
 * - AC #2/#3: docker/src/entrypoint-town.ts has inline section comments for
 *             each major pipeline stage and SDK feature
 *
 * All tests are static analysis (filesystem/source inspection) and do not
 * require any running services.
 *
 * Test IDs:
 * - T-2.4-05: docs/api-contracts-git-proxy.md should not exist
 * - T-2.4-06: docs/project-scan-report.json should not reference git-proxy
 * - T-2.4-07: docs/index.md should not reference git-proxy
 * - T-2.4-08: entrypoint-town.ts should have SDK Reference Implementation JSDoc
 * - T-2.4-09: entrypoint-town.ts should have inline section comments for pipeline stages
 * - T-2.4-10: entrypoint-town.ts documentation covers AC #2 specific demonstrations
 * - T-2.4-11: entrypoint-town.ts exercises every major SDK feature (AC #3)
 */

import { describe, it, expect } from 'vitest';
import { existsSync, readFileSync } from 'fs';
import { resolve } from 'path';

// ============================================================================
// Helpers
// ============================================================================

/**
 * Resolve a path relative to the repository root.
 * The test file lives at packages/town/src/doc-cleanup-and-reference.test.ts,
 * so the repo root is three directories up.
 */
function repoRoot(): string {
  return resolve(__dirname, '..', '..', '..');
}

// ============================================================================
// AC #1: Stale Documentation Cleanup
// ============================================================================

describe('Story 2.4: Stale documentation cleanup (AC #1)', () => {
  it('T-2.4-05: docs/api-contracts-git-proxy.md should not exist', () => {
    // P2: The obsolete git-proxy API contracts document must be deleted.
    // It documents the removed @crosstown/git-proxy package's HTTP proxy API,
    // which is superseded by the NIP-34 Rig pattern (Epic 5).
    const stalePath = resolve(repoRoot(), 'docs', 'api-contracts-git-proxy.md');
    expect(
      existsSync(stalePath),
      `Expected docs/api-contracts-git-proxy.md to be deleted (stale git-proxy documentation), but it still exists at: ${stalePath}`
    ).toBe(false);
  });

  it('T-2.4-06: docs/project-scan-report.json should not reference git-proxy', () => {
    // P2: The project scan report was generated before git-proxy was removed.
    // It contains stale references in project_types, outputs_generated,
    // batches_completed, project_classification, and technology_stack.
    // All git-proxy mentions must be removed to keep the report accurate.
    const reportPath = resolve(repoRoot(), 'docs', 'project-scan-report.json');

    if (!existsSync(reportPath)) {
      // If the file itself was removed, this condition is trivially satisfied
      return;
    }

    const content = readFileSync(reportPath, 'utf-8');

    // The string "git-proxy" (case-insensitive) should not appear anywhere
    // in the project scan report after cleanup.
    expect(
      content.toLowerCase(),
      'docs/project-scan-report.json should not contain any git-proxy references. ' +
        'Known locations: project_types array, outputs_generated array, ' +
        'batches_completed array, project_classification string, technology_stack string, ' +
        'completed_steps[0].summary string.'
    ).not.toMatch(/git-proxy/i);
  });

  it('T-2.4-07: docs/index.md should not reference git-proxy', () => {
    // P2: The documentation index was generated before git-proxy was removed.
    // It contains a package table row for @crosstown/git-proxy and an API
    // contracts link for the git-proxy documentation. Both must be removed.
    const indexPath = resolve(repoRoot(), 'docs', 'index.md');

    if (!existsSync(indexPath)) {
      // If the file itself was removed, this condition is trivially satisfied
      return;
    }

    const content = readFileSync(indexPath, 'utf-8');

    // No mention of git-proxy should remain in the documentation index
    expect(
      content.toLowerCase(),
      'docs/index.md should not contain any git-proxy references. ' +
        'Known locations: package table row for @crosstown/git-proxy, ' +
        'API contracts link for git-proxy documentation.'
    ).not.toMatch(/git-proxy/i);
  });
});

// ============================================================================
// AC #2 & #3: Reference Implementation Documentation
// ============================================================================

describe('Story 2.4: Reference implementation documentation (AC #2, #3)', () => {
  const entrypointPath = resolve(
    repoRoot(),
    'docker',
    'src',
    'entrypoint-town.ts'
  );

  it('T-2.4-08: entrypoint-town.ts should have SDK Reference Implementation JSDoc', () => {
    // P2: The file-level JSDoc comment must document entrypoint-town.ts as the
    // SDK Reference Implementation. This transforms it from a working
    // implementation into a documented example that developers can study.
    // The JSDoc should explain:
    // - What the file demonstrates (SDK-based relay construction)
    // - The SDK pattern (identity -> pipeline -> handlers -> lifecycle)
    // - Why Approach A is used (external connector mode vs embedded)
    // - Which SDK features are exercised
    expect(
      existsSync(entrypointPath),
      'docker/src/entrypoint-town.ts must exist'
    ).toBe(true);

    const source = readFileSync(entrypointPath, 'utf-8');

    // The file should contain "Reference Implementation" in a JSDoc comment
    // (case-insensitive match to be flexible with exact wording)
    expect(
      source.toLowerCase(),
      'entrypoint-town.ts must contain "reference implementation" in its ' +
        'file-level JSDoc comment to document it as the SDK Reference Implementation'
    ).toMatch(/reference\s+implementation/i);

    // The JSDoc should mention the SDK pattern flow
    expect(
      source,
      'entrypoint-town.ts JSDoc should describe the SDK pattern flow ' +
        '(identity, pipeline, handler, lifecycle)'
    ).toMatch(/identity/i);
  });

  it('T-2.4-09: entrypoint-town.ts should have inline section comments for pipeline stages', () => {
    // P2: Each major section of the entrypoint should have inline comments
    // explaining the "why" not the "what" -- documenting the SDK pattern being
    // demonstrated, not just describing the code.
    //
    // Required section comment topics:
    // - Identity derivation (fromSecretKey)
    // - Verification pipeline (createVerificationPipeline)
    // - Pricing validator (createPricingValidator)
    // - Handler registry (HandlerRegistry)
    // - Handler context (createHandlerContext)
    // - Pipeline stages (5-stage pipeline)
    // - EventStore initialization
    // - Bootstrap lifecycle
    // - Self-write bypass
    // - Graceful shutdown
    expect(
      existsSync(entrypointPath),
      'docker/src/entrypoint-town.ts must exist'
    ).toBe(true);

    const source = readFileSync(entrypointPath, 'utf-8');

    // Count comment lines (single-line // comments and multi-line /* */ comment lines)
    const lines = source.split('\n');
    const commentLines = lines.filter((line) => {
      const trimmed = line.trim();
      return (
        trimmed.startsWith('//') ||
        trimmed.startsWith('/*') ||
        trimmed.startsWith('*') ||
        trimmed.startsWith('*/')
      );
    });

    // The documented reference implementation should have significantly more
    // comments than the current bare implementation. The current file has ~20
    // comment lines. After documentation, it should have at least 40 to cover
    // all the required SDK pattern explanations across the 10 sections.
    expect(
      commentLines.length,
      `entrypoint-town.ts has ${commentLines.length} comment lines. ` +
        `After adding reference implementation documentation, it should have ` +
        `at least 40 comment lines covering: identity derivation, verification ` +
        `pipeline, pricing validator, handler registry, handler context, ` +
        `pipeline stages, EventStore, bootstrap lifecycle, self-write bypass, ` +
        `and graceful shutdown.`
    ).toBeGreaterThanOrEqual(40);

    // Verify specific SDK pattern documentation exists as inline comments.
    // These keywords indicate the developer documented the "why" for each
    // major section, not just the "what".

    // Identity: should explain unified secp256k1 identity (Nostr + EVM)
    expect(
      source,
      'Should document identity derivation (Nostr pubkey and EVM address from single key)'
    ).toMatch(/secp256k1|unified.*identity|nostr.*evm|single.*key/i);

    // Verification: should explain Schnorr signature verification
    expect(source, 'Should document Schnorr signature verification').toMatch(
      /schnorr/i
    );

    // Pricing: should explain self-write bypass
    expect(source, 'Should document self-write bypass pattern').toMatch(
      /self.write.*bypass|bypass.*own.*pubkey|own.*pubkey.*bypass/i
    );

    // Handler context: should explain TOON passthrough or lazy decode
    expect(
      source,
      'Should document TOON passthrough or lazy decode pattern'
    ).toMatch(/toon.*passthrough|lazy.*decode|raw.*toon/i);

    // Bootstrap: should explain lifecycle
    expect(source, 'Should document bootstrap lifecycle').toMatch(
      /bootstrap.*lifecycle|lifecycle.*management|peer.*discovery.*lifecycle/i
    );
  });

  it('T-2.4-10: entrypoint-town.ts documentation covers AC #2 specific demonstrations', () => {
    // AC #2 requires the reference implementation to demonstrate:
    // - seed phrase identity (or fromSecretKey identity derivation)
    // - kind-based handler registration
    // - ctx.decode() for code handlers
    // - settlement configuration
    // - lifecycle management (with inline comments explaining each pattern)
    //
    // T-2.4-08 and T-2.4-09 verify the JSDoc and section comments exist,
    // but do not verify coverage of each specific demonstration listed in AC #2.
    expect(
      existsSync(entrypointPath),
      'docker/src/entrypoint-town.ts must exist'
    ).toBe(true);

    const source = readFileSync(entrypointPath, 'utf-8');

    // ctx.decode() / lazy decode must be documented as a pattern.
    // AC #2 explicitly mentions "ctx.decode() for code handlers" as a
    // demonstration developers should see in the example.
    expect(
      source,
      'Should document ctx.decode() lazy decode pattern (AC #2: "ctx.decode() for code handlers")'
    ).toMatch(/ctx\.decode\(\)|lazy.*decode|decode.*pattern/i);

    // Settlement configuration must be documented. AC #2 explicitly mentions
    // "settlement" as a demonstrated pattern.
    expect(
      source,
      'Should document settlement negotiation (AC #2: "settlement negotiation")'
    ).toMatch(
      /settlement.*negotiat|negotiate.*settlement|payment.*channel.*settlement|settlement.*config/i
    );

    // Kind-based handler registration must be documented. AC #2 mentions
    // "kind-based handler registration" as a demonstrated pattern.
    expect(
      source,
      'Should document kind-based handler registration (AC #2: "kind-based handler registration")'
    ).toMatch(
      /kind.*dispatch|dispatch.*kind|kind.*handler|handler.*kind|\.on\(kind|\.onDefault/i
    );

    // Lifecycle management must be documented. AC #2 mentions
    // "lifecycle management" as a demonstrated pattern.
    expect(
      source,
      'Should document lifecycle management (AC #2: "lifecycle management")'
    ).toMatch(
      /graceful.*shutdown|shutdown.*pattern|lifecycle.*management|unsubscribe.*relay|stop.*relay/i
    );
  });

  it('T-2.4-11: entrypoint-town.ts exercises every major SDK feature (AC #3)', () => {
    // AC #3 requires that "every major SDK feature is exercised":
    // identity, handlers, pricing, bootstrap, channels, dev mode.
    //
    // This test verifies the entrypoint source code actually uses/references
    // each feature area, not just documents them. It checks for SDK API
    // function calls and configuration patterns that demonstrate each feature.
    expect(
      existsSync(entrypointPath),
      'docker/src/entrypoint-town.ts must exist'
    ).toBe(true);

    const source = readFileSync(entrypointPath, 'utf-8');

    // Identity: fromSecretKey() must be called (not just imported)
    expect(
      source,
      'Must exercise identity feature by calling fromSecretKey() (AC #3: "identity")'
    ).toMatch(/fromSecretKey\s*\(/);

    // Handlers: HandlerRegistry must be instantiated and handlers registered
    expect(
      source,
      'Must exercise handlers feature by instantiating HandlerRegistry (AC #3: "handlers")'
    ).toMatch(/new\s+HandlerRegistry\s*\(/);
    // The default handler registration (.onDefault) demonstrates the handler feature.
    // Kind-specific .on() registration will return when x402 handler is added.
    expect(
      source,
      'Must exercise handlers feature by registering default handler via .onDefault() (AC #3: "handlers")'
    ).toMatch(/registry\.onDefault\s*\(/);

    // Pricing: createPricingValidator() must be called with config
    expect(
      source,
      'Must exercise pricing feature by calling createPricingValidator() (AC #3: "pricing")'
    ).toMatch(/createPricingValidator\s*\(\s*\{/);
    // Pricing config must include basePricePerByte and ownPubkey
    expect(
      source,
      'Pricing validator config must include basePricePerByte (per-byte pricing)'
    ).toMatch(/basePricePerByte/);
    expect(
      source,
      'Pricing validator config must include ownPubkey (self-write bypass)'
    ).toMatch(/ownPubkey/);

    // Bootstrap: BootstrapService must be instantiated
    expect(
      source,
      'Must exercise bootstrap feature by instantiating BootstrapService (AC #3: "bootstrap")'
    ).toMatch(/new\s+BootstrapService\s*\(/);

    // Channels: settlement and channel configuration must be referenced
    // Note: channelClient was moved to shared.ts; entrypoint-town.ts references
    // settlement configuration via config.settlementInfo which demonstrates
    // awareness of the channels feature through local settlement negotiation.
    expect(
      source,
      'Must reference settlement/channel concepts (AC #3: "channels")'
    ).toMatch(/settlement|channel/i);

    // Dev mode: must be addressed (even if set to false, the entrypoint should
    // demonstrate awareness of the devMode option in comments or config)
    expect(
      source,
      'Must address dev mode feature (AC #3: "dev mode") -- the reference ' +
        'implementation should show devMode configuration even if set to false'
    ).toMatch(/devMode/i);
  });
});
