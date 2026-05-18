export class DashboardError extends Error {
  code: string;
  details?: unknown;
  status: number;
  constructor(status: number, code: string, message: string, details?: unknown) {
    super(message);
    this.status = status;
    this.code = code;
    this.details = details;
  }
}

async function call<T>(path: string, init?: RequestInit): Promise<T> {
  const r = await fetch(path, { credentials: 'include', ...init });
  if (r.status === 204) return undefined as T;
  const ct = r.headers.get('content-type') || '';
  const body = ct.includes('application/json') ? await r.json() : await r.text();
  if (!r.ok) {
    const err = (body as any)?.error || {};
    throw new DashboardError(r.status, err.code || 'unknown', err.message || `HTTP ${r.status}`, err.details);
  }
  return body as T;
}

export const api = {
  get: <T,>(p: string) => call<T>(p),
  post: <T,>(p: string, body?: unknown) => call<T>(p, {
    method: 'POST',
    headers: body ? { 'Content-Type': 'application/json' } : undefined,
    body: body !== undefined ? JSON.stringify(body) : undefined,
  }),
  patch: <T,>(p: string, body: unknown) => call<T>(p, {
    method: 'PATCH',
    headers: { 'Content-Type': 'application/json' },
    body: JSON.stringify(body),
  }),
  delete: <T,>(p: string) => call<T>(p, { method: 'DELETE' }),
};
