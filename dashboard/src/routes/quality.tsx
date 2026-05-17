import { useQuery } from '@tanstack/react-query';
import { LineChart, Line, XAxis, YAxis, CartesianGrid, ResponsiveContainer, Tooltip } from 'recharts';
import { api } from '../lib/api';

interface QualityRun {
  id: string; baseline_label: string; model: string | null;
  ndcg10: number; mrr: number; p95_latency_ms: number;
  ran_at: string;
}

export function Quality() {
  const runs = useQuery({
    queryKey: ['quality'],
    queryFn: () => api.get<{ runs: QualityRun[] }>('/admin/quality?limit=60'),
  });
  const cfg = useQuery({
    queryKey: ['quality-config'],
    queryFn: () => api.get<{ enabled: boolean; cron: string | null }>('/admin/quality/config'),
  });

  const rows = (runs.data?.runs ?? []).slice().reverse();   // oldest → newest for charting
  const latest = rows[rows.length - 1];

  if (rows.length === 0) {
    return (
      <div className="bg-white border border-neutral-200 rounded p-6">
        <h1 className="text-xl font-medium mb-3">Search quality</h1>
        <p className="text-sm text-neutral-600 mb-3">No eval history yet.</p>
        <pre className="bg-neutral-50 border border-neutral-200 rounded p-3 text-xs whitespace-pre-wrap">
{`Run periodic search-quality benchmarks by adding [quality] to your config:
[quality]
enabled = true
cron    = "0 2 * * *"   # 02:00 daily

Or upload a one-off result: POST /admin/quality/runs`}
        </pre>
      </div>
    );
  }

  return (
    <div className="space-y-6">
      <h1 className="text-xl font-medium">Search quality</h1>
      <Chart rows={rows} dataKey="ndcg10" label="nDCG@10" yDomain={[0, 1]} />
      <Chart rows={rows} dataKey="mrr"    label="MRR"      yDomain={[0, 1]} />
      <Chart rows={rows} dataKey="p95_latency_ms" label="p95 latency (ms)" yDomain={[0, 5000]} />
      <div className="bg-white border border-neutral-200 rounded p-4 text-sm">
        <div className="text-neutral-500 mb-1">Latest run: {latest && new Date(latest.ran_at).toLocaleString()}</div>
        <div>nDCG@10: <b>{latest?.ndcg10.toFixed(3)}</b> · MRR: <b>{latest?.mrr.toFixed(3)}</b> · p95: <b>{latest?.p95_latency_ms.toFixed(0)} ms</b></div>
        <div className="text-xs text-neutral-500 mt-1">Model: {latest?.model ?? '—'}</div>
      </div>
      <div className="text-xs text-neutral-500">
        Scheduler: {cfg.data?.enabled ? 'enabled' : 'disabled'} · cron: {cfg.data?.cron ?? '—'}
      </div>
    </div>
  );
}

function Chart({ rows, dataKey, label, yDomain }: { rows: any[]; dataKey: string; label: string; yDomain: [number, number] }) {
  return (
    <div className="bg-white border border-neutral-200 rounded p-4">
      <div className="text-sm text-neutral-500 mb-2">{label}</div>
      <div style={{ width: '100%', height: 180 }}>
        <ResponsiveContainer>
          <LineChart data={rows.map(r => ({ ...r, ts: new Date(r.ran_at).getTime() }))}>
            <CartesianGrid strokeDasharray="2 2" stroke="#eee" />
            <XAxis dataKey="ts" type="number" domain={['dataMin', 'dataMax']} hide />
            <YAxis domain={yDomain} fontSize={11} />
            <Tooltip labelFormatter={(v) => new Date(v).toLocaleDateString()} />
            <Line type="monotone" dataKey={dataKey} stroke="#171717" dot={false} />
          </LineChart>
        </ResponsiveContainer>
      </div>
    </div>
  );
}
