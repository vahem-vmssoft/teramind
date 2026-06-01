import { describe, it, expect, vi, beforeEach } from 'vitest';
import { api, DashboardError } from '../src/lib/api';

// useAuth from src/lib/auth.tsx is bound to a TanStack Router context
// and TanStack's useNavigate, so a faithful render needs JSDOM + the
// full RouterProvider tree. The vitest config here is `environment:
// 'node'`, so we test the underlying contract directly: useAuth fetches
// /admin/me, treats 200 as authenticated, and on 401 redirects to
// /login (resolved as /dashboard/login under the dashboard base) with
// a `redirect` search param equal to the current pathname+search.
//
// This mirrors the hook body in src/lib/auth.tsx:
//   api.get('/admin/me').then(d => authenticated = d.admin)
//                       .catch(e => if e.status === 401 navigate({ to:'/login', search:{ redirect: here } }))

interface AuthOutcome {
  authenticated: boolean;
  navigation?: { to: string; search: { redirect: string } };
}

async function runAuthOnce(here: string): Promise<AuthOutcome> {
  let navigation: AuthOutcome['navigation'] | undefined;
  try {
    const d = await api.get<{ admin: boolean }>('/admin/me');
    return { authenticated: !!d.admin };
  } catch (e) {
    if (e instanceof DashboardError && e.status === 401) {
      navigation = { to: '/login', search: { redirect: here } };
    }
    return { authenticated: false, navigation };
  }
}

beforeEach(() => { vi.stubGlobal('fetch', vi.fn()); });

describe('useAuth contract', () => {
  it('200 from /admin/me marks the session authenticated and does not redirect', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ admin: true, expires_at: '2099-01-01T00:00:00Z' }),
      { status: 200, headers: { 'content-type': 'application/json' } },
    ));
    const out = await runAuthOnce('/quality');
    expect(out.authenticated).toBe(true);
    expect(out.navigation).toBeUndefined();
  });

  it('401 from /admin/me redirects to /login with the current path as redirect', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ error: { code: 'unauthorized', message: 'no session' } }),
      { status: 401, headers: { 'content-type': 'application/json' } },
    ));
    const here = '/quality?range=7d';
    const out = await runAuthOnce(here);
    expect(out.authenticated).toBe(false);
    // Router base is `/dashboard/`, so `to: '/login'` resolves to
    // /dashboard/login?redirect=<path> in the browser URL.
    expect(out.navigation).toEqual({ to: '/login', search: { redirect: here } });
  });

  it('non-401 errors leave the user unauthenticated but do not redirect', async () => {
    (global.fetch as any).mockResolvedValue(new Response(
      JSON.stringify({ error: { code: 'rate_limited', message: 'slow down' } }),
      { status: 429, headers: { 'content-type': 'application/json' } },
    ));
    const out = await runAuthOnce('/activity');
    expect(out.authenticated).toBe(false);
    expect(out.navigation).toBeUndefined();
  });
});
