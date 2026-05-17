import { describe, it, expect, vi, beforeEach } from 'vitest';
import { api, DashboardError } from '../src/lib/api';

beforeEach(() => { vi.stubGlobal('fetch', vi.fn()); });

describe('api client', () => {
  it('parses error JSON into DashboardError', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ error: { code: 'rate_limited', message: 'slow down' } }),
      { status: 429, headers: { 'content-type': 'application/json' } },
    ));
    try {
      await api.get('/admin/me');
      throw new Error('expected throw');
    } catch (e) {
      expect(e).toBeInstanceOf(DashboardError);
      expect((e as DashboardError).code).toBe('rate_limited');
      expect((e as DashboardError).status).toBe(429);
    }
  });

  it('returns parsed JSON on success', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ admin: true }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    ));
    const out = await api.get<{ admin: boolean }>('/admin/me');
    expect(out.admin).toBe(true);
  });
});
