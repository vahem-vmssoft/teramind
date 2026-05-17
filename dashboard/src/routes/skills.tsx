import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api, DashboardError } from '../lib/api';

type Source = 'all' | 'authored' | 'codified' | 'pending' | 'rejected';

interface SkillRow {
  id: string;
  name: string;
  description: string;
  source: string;
  status?: string;
  applies_to_cwds: string[];
}

interface SkillDetail extends SkillRow { body: string }
interface Candidate {
  id: string;
  name: string;
  description: string;
  body: string;
  applies_to_cwds: string[];
  source_session_ids: string[];
  model: string;
  status: string;
  generated_at: string;
}

export function Skills() {
  const [source, setSource] = useState<Source>('all');
  const [selectedId, setSelectedId] = useState<string | null>(null);

  const list = useQuery({
    queryKey: ['skills', source],
    queryFn: () => api.get<{ skills?: SkillRow[]; candidates?: Candidate[] }>(
      source === 'pending' || source === 'rejected'
        ? `/admin/candidates?status=${source}&limit=100`
        : `/admin/skills?source=${source === 'all' ? 'all' : source}&limit=100`),
  });

  const detail = useQuery({
    queryKey: ['skill-detail', source, selectedId],
    queryFn: () => {
      if (!selectedId) return Promise.resolve(null);
      if (source === 'pending' || source === 'rejected') {
        return api.get<Candidate>(`/admin/candidates/${selectedId}`);
      }
      return api.get<SkillDetail>(`/admin/skills/${selectedId}`);
    },
    enabled: !!selectedId,
  });

  const rows: Array<{ id: string; name: string; description: string }> =
    (list.data?.skills ?? list.data?.candidates ?? []).map(r => ({
      id: r.id, name: r.name, description: r.description,
    }));

  return (
    <div className="grid grid-cols-12 gap-4 h-full">
      <aside className="col-span-4 bg-white rounded border border-neutral-200 flex flex-col">
        <div className="p-3 border-b border-neutral-200 flex flex-col gap-2">
          <h1 className="font-medium">Skills</h1>
          <div className="flex flex-wrap gap-1 text-xs">
            {(['all', 'authored', 'codified', 'pending', 'rejected'] as Source[]).map(s => (
              <button key={s}
                      onClick={() => { setSource(s); setSelectedId(null); }}
                      className={`px-2 py-1 rounded ${source === s ? 'bg-neutral-900 text-white' : 'bg-neutral-100 hover:bg-neutral-200'}`}>
                {s}
              </button>
            ))}
          </div>
        </div>
        <div className="flex-1 overflow-auto">
          {rows.map(r => (
            <button key={r.id}
                    onClick={() => setSelectedId(r.id)}
                    className={`w-full text-left px-3 py-2 border-b border-neutral-100 hover:bg-neutral-50 ${selectedId === r.id ? 'bg-neutral-100' : ''}`}>
              <div className="font-medium text-sm">{r.name}</div>
              <div className="text-xs text-neutral-500 truncate">{r.description}</div>
            </button>
          ))}
        </div>
      </aside>
      <section className="col-span-8 bg-white rounded border border-neutral-200 p-4 overflow-auto">
        {!selectedId ? (
          <div className="text-sm text-neutral-500">Pick a skill on the left.</div>
        ) : source === 'pending' || source === 'rejected' ? (
          <CandidateReview candidate={detail.data as Candidate | null | undefined}
                           onChanged={() => { list.refetch(); detail.refetch(); }} />
        ) : (
          <SkillDetailPanel skill={detail.data as SkillDetail | null | undefined} />
        )}
      </section>
    </div>
  );
}

function SkillDetailPanel({ skill }: { skill?: SkillDetail | null }) {
  if (!skill) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <article>
      <h2 className="text-xl font-medium">{skill.name}</h2>
      <div className="text-sm text-neutral-500 mt-1">source: {skill.source} · applies_to: {(skill.applies_to_cwds || []).join(', ') || 'global'}</div>
      <p className="mt-3 text-sm">{skill.description}</p>
      <pre className="mt-4 p-3 bg-neutral-50 border border-neutral-200 rounded text-xs whitespace-pre-wrap">{skill.body}</pre>
    </article>
  );
}

function CandidateReview({ candidate, onChanged }: { candidate?: Candidate | null; onChanged: () => void }) {
  const [description, setDesc] = useState('');
  const [body, setBody] = useState('');
  const [cwds, setCwds] = useState('');
  const [error, setError] = useState<string | null>(null);
  const qc = useQueryClient();

  // Sync editable fields when candidate changes.
  if (candidate && description === '' && body === '' && cwds === '') {
    setDesc(candidate.description);
    setBody(candidate.body);
    setCwds(candidate.applies_to_cwds.join('\n'));
  }

  const save = useMutation({
    mutationFn: () => api.patch(`/admin/candidates/${candidate!.id}`, {
      description, body, applies_to_cwds: cwds.split('\n').map(s => s.trim()).filter(Boolean),
    }),
    onSuccess: () => { setError(null); onChanged(); },
    onError: (e: DashboardError) => setError(e.message),
  });
  const approve = useMutation({
    mutationFn: () => api.post(`/admin/candidates/${candidate!.id}/approve`, { reviewer: 'admin' }),
    onSuccess: () => { onChanged(); qc.invalidateQueries(); },
    onError: (e: DashboardError) => setError(e.message),
  });
  const reject = useMutation({
    mutationFn: () => api.post(`/admin/candidates/${candidate!.id}/reject`, { reviewer: 'admin' }),
    onSuccess: () => { onChanged(); qc.invalidateQueries(); },
    onError: (e: DashboardError) => setError(e.message),
  });

  if (!candidate) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <article className="space-y-4">
      <header>
        <h2 className="text-xl font-medium">{candidate.name}</h2>
        <div className="text-sm text-neutral-500">
          status: {candidate.status} · model: {candidate.model} · generated {candidate.generated_at}
        </div>
      </header>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">Description</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 text-sm" rows={2}
                  value={description} onChange={e => setDesc(e.target.value)} />
      </div>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">Body</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 font-mono text-xs" rows={20}
                  value={body} onChange={e => setBody(e.target.value)} />
      </div>
      <div>
        <label className="text-xs uppercase tracking-wide text-neutral-500">applies_to_cwds (one per line)</label>
        <textarea className="w-full border border-neutral-300 rounded p-2 font-mono text-xs" rows={4}
                  value={cwds} onChange={e => setCwds(e.target.value)} />
      </div>
      {error && <div className="text-sm text-red-600">{error}</div>}
      <footer className="flex justify-end gap-2">
        <button onClick={() => reject.mutate()} className="px-3 py-1.5 text-sm rounded bg-red-50 text-red-700 hover:bg-red-100">Reject</button>
        <button onClick={() => save.mutate()} className="px-3 py-1.5 text-sm rounded bg-neutral-100 hover:bg-neutral-200">Save edits</button>
        <button onClick={() => approve.mutate()} className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white hover:bg-neutral-800">Approve & Promote</button>
      </footer>
    </article>
  );
}
