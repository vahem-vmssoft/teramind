import { useState } from 'react';
import { useNavigate, useSearch } from '@tanstack/react-router';
import { api, DashboardError } from '../lib/api';

export function Login() {
  const navigate = useNavigate();
  const search = useSearch({ strict: false }) as { redirect?: string };
  const [password, setPassword] = useState('');
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);

  async function submit(e: React.FormEvent) {
    e.preventDefault();
    setBusy(true); setError(null);
    try {
      await api.post('/admin/login', { password });
      navigate({ to: search.redirect ?? '/activity' });
    } catch (err) {
      const e = err as DashboardError;
      setError(e.code === 'rate_limited'
        ? 'Too many attempts. Wait 5 minutes.'
        : e.code === 'invalid_password' ? 'Incorrect password.' : e.message);
    } finally {
      setBusy(false);
    }
  }

  return (
    <div className="min-h-screen flex items-center justify-center bg-neutral-50">
      <form onSubmit={submit} className="bg-white shadow-xl rounded-lg p-8 w-full max-w-sm">
        <h1 className="text-xl font-medium mb-1">Teramind Dashboard</h1>
        <p className="text-sm text-neutral-500 mb-6">Admin sign-in</p>
        <input
          type="password" autoFocus placeholder="Admin password" value={password}
          onChange={e => setPassword(e.target.value)}
          className="w-full border border-neutral-300 rounded px-3 py-2 text-sm focus:border-neutral-900 focus:ring-1 focus:ring-neutral-900 outline-none"
        />
        {error && <div className="mt-3 text-sm text-red-600">{error}</div>}
        <button disabled={busy || !password}
                className="mt-4 w-full bg-neutral-900 text-white rounded px-3 py-2 text-sm font-medium disabled:opacity-50">
          {busy ? 'Signing in…' : 'Sign in'}
        </button>
      </form>
    </div>
  );
}
