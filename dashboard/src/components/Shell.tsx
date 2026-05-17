import { Link, Outlet } from '@tanstack/react-router';
import { Activity, BookText, Users, BarChart3, Heart } from 'lucide-react';
import { useAuth } from '../lib/auth';
import { api } from '../lib/api';

const nav = [
  { to: '/activity', label: 'Activity', icon: Activity },
  { to: '/skills',   label: 'Skills',   icon: BookText },
  { to: '/members',  label: 'Members',  icon: Users },
  { to: '/quality',  label: 'Quality',  icon: BarChart3 },
  { to: '/health',   label: 'Health',   icon: Heart },
];

export function Shell() {
  const auth = useAuth();
  if (auth.loading) return <div className="p-8 text-neutral-500">Loading…</div>;
  if (!auth.authenticated) return null;
  return (
    <div className="min-h-screen flex">
      <aside className="w-56 bg-white border-r border-neutral-200 p-4">
        <div className="text-sm font-semibold text-neutral-500 mb-4 px-2">TERAMIND</div>
        <nav className="space-y-1">
          {nav.map(n => (
            <Link key={n.to} to={n.to}
                  className="flex items-center gap-2 px-2 py-2 rounded hover:bg-neutral-100 [&.active]:bg-neutral-200 [&.active]:font-medium">
              <n.icon size={16} /> {n.label}
            </Link>
          ))}
        </nav>
      </aside>
      <main className="flex-1 flex flex-col">
        <header className="border-b border-neutral-200 bg-white px-6 py-3 flex justify-between items-center">
          <div className="text-sm text-neutral-500">Dashboard · {window.location.host}</div>
          <button onClick={() => api.post('/admin/logout').then(() => window.location.href = '/dashboard/login')}
                  className="text-sm text-neutral-600 hover:text-neutral-900">
            Logout
          </button>
        </header>
        <section className="flex-1 p-6 overflow-auto"><Outlet /></section>
      </main>
    </div>
  );
}
