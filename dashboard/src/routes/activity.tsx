import { useState } from 'react';
import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';
import { useEventStream, TeamEvent } from '../lib/event_stream';

interface ActivityRow {
  id: string;
  kind: string;
  user_id?: string;
  cwd?: string;
  payload: any;
  ts: string;
}

export function Activity() {
  const [auto, setAuto] = useState(true);
  const [kindFilter, setKindFilter] = useState('');
  const live = useEventStream(auto);

  const { data, isLoading } = useQuery({
    queryKey: ['activity', kindFilter],
    queryFn: async () => {
      const qs = kindFilter ? `?kind=${kindFilter}&limit=100` : `?limit=100`;
      return api.get<{ events: ActivityRow[] }>(`/admin/activity${qs}`);
    },
  });

  const liveRows: ActivityRow[] = live.map(e => ({
    id: `live-${e.ts}-${Math.random()}`,
    kind: e.type, user_id: e.user_id, cwd: e.cwd,
    payload: e, ts: e.ts,
  }));
  const combined = [...liveRows, ...(data?.events ?? [])];

  return (
    <div>
      <header className="flex justify-between items-center mb-4">
        <h1 className="text-xl font-medium">Activity</h1>
        <div className="flex items-center gap-3 text-sm">
          <select value={kindFilter} onChange={e => setKindFilter(e.target.value)}
                  className="border border-neutral-300 rounded px-2 py-1 text-sm">
            <option value="">All kinds</option>
            <option value="session_ended">Session ended</option>
            <option value="skill_saved">Skill saved</option>
            <option value="wiki_page_ready">Wiki page ready</option>
          </select>
          <label className="flex items-center gap-1.5">
            <input type="checkbox" checked={auto} onChange={e => setAuto(e.target.checked)} />
            Live
          </label>
        </div>
      </header>
      {isLoading ? <div className="text-sm text-neutral-500">Loading…</div> : null}
      <div className="bg-white rounded border border-neutral-200 divide-y divide-neutral-100">
        {combined.map(r => (
          <div key={r.id} className="px-4 py-2 flex items-center text-sm">
            <span className="text-neutral-400 font-mono text-xs w-44 shrink-0">{r.ts.replace('T', ' ').slice(0, 19)}</span>
            <span className="font-mono text-xs w-44 shrink-0">{r.kind}</span>
            <span className="flex-1 truncate text-neutral-600">{r.cwd ?? r.payload?.name ?? ''}</span>
          </div>
        ))}
        {combined.length === 0 && !isLoading ? (
          <div className="px-4 py-12 text-center text-sm text-neutral-500">No activity yet.</div>
        ) : null}
      </div>
    </div>
  );
}
