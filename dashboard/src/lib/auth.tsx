import { useEffect, useState } from 'react';
import { useNavigate, useRouter } from '@tanstack/react-router';
import { api, DashboardError } from './api';

export interface AuthState { authenticated: boolean; loading: boolean; expiresAt?: string }

export function useAuth(): AuthState {
  const [state, setState] = useState<AuthState>({ authenticated: false, loading: true });
  const navigate = useNavigate();
  useEffect(() => {
    api.get<{ admin: boolean; expires_at: string }>('/admin/me')
      .then(d => setState({ authenticated: d.admin, loading: false, expiresAt: d.expires_at }))
      .catch((e: DashboardError) => {
        setState({ authenticated: false, loading: false });
        if (e.status === 401) {
          const here = window.location.pathname + window.location.search;
          navigate({ to: '/login', search: { redirect: here } as any });
        }
      });
  }, [navigate]);
  return state;
}
