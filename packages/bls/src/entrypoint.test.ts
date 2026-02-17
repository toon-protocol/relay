import { describe, it, expect } from 'vitest';
import { Hono } from 'hono';
import { BusinessLogicServer } from './bls/index.js';
import { InMemoryEventStore } from './storage/index.js';

/**
 * Tests for the entrypoint's Hono app composition pattern.
 *
 * The entrypoint creates a new Hono app and registers an enhanced /health
 * endpoint BEFORE mounting the BLS sub-app, because Hono uses first-match-wins
 * routing. These tests verify that pattern works correctly.
 */
describe('entrypoint health endpoint composition', () => {
  function createComposedApp(nodeInfo: {
    nodeId: string;
    pubkey: string;
    ilpAddress: string;
  }) {
    const eventStore = new InMemoryEventStore();
    const bls = new BusinessLogicServer(
      { basePricePerByte: 10n },
      eventStore
    );

    const app = new Hono();

    // Register enhanced health endpoint BEFORE mounting sub-app
    // (mirrors entrypoint.ts pattern)
    app.get('/health', (c) => {
      return c.json({
        status: 'healthy',
        nodeId: nodeInfo.nodeId,
        pubkey: nodeInfo.pubkey,
        ilpAddress: nodeInfo.ilpAddress,
        timestamp: Date.now(),
      });
    });

    // Mount BLS routes
    app.route('/', bls.getApp());

    return app;
  }

  it('should serve enhanced health endpoint with node identity', async () => {
    const app = createComposedApp({
      nodeId: 'test-node-1',
      pubkey: 'a'.repeat(64),
      ilpAddress: 'g.crosstown.test',
    });

    const res = await app.request('/health');
    expect(res.status).toBe(200);

    const body = await res.json();
    expect(body.status).toBe('healthy');
    expect(body.nodeId).toBe('test-node-1');
    expect(body.pubkey).toBe('a'.repeat(64));
    expect(body.ilpAddress).toBe('g.crosstown.test');
    expect(body.timestamp).toBeTypeOf('number');
  });

  it('should still serve handle-packet endpoint from BLS sub-app', async () => {
    const app = createComposedApp({
      nodeId: 'test-node-1',
      pubkey: 'a'.repeat(64),
      ilpAddress: 'g.crosstown.test',
    });

    const res = await app.request('/handle-packet', {
      method: 'POST',
      headers: { 'Content-Type': 'application/json' },
      body: JSON.stringify({ amount: '100', destination: 'test', data: '' }),
    });

    // Should get a 400 (bad request) not 404 - proving the route is reachable
    expect(res.status).toBe(400);
  });

  it('should NOT serve basic health (without nodeId) when enhanced is registered first', async () => {
    const app = createComposedApp({
      nodeId: 'my-node',
      pubkey: 'b'.repeat(64),
      ilpAddress: 'g.test.node',
    });

    const res = await app.request('/health');
    const body = await res.json();

    // Verify it's the enhanced version, not the basic one
    expect(body).toHaveProperty('nodeId');
    expect(body).toHaveProperty('pubkey');
    expect(body).toHaveProperty('ilpAddress');
    expect(body.nodeId).toBe('my-node');
  });
});
