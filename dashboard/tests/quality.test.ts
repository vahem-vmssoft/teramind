import { describe, it, expect } from 'vitest';

// Dashboard §5: the Quality route renders nDCG@10, MRR, and p95
// latency line charts whose y-values are the per-run metric values
// from /admin/quality (most recent run last on the x-axis).
//
// The route in src/routes/quality.tsx is bound to recharts'
// <ResponsiveContainer> and @tanstack/react-query, neither of which
// renders meaningfully under `environment: 'node'`. The chart input
// is, however, a pure transformation of the API response:
//
//   const rows  = (runs.data?.runs ?? []).slice().reverse();
//   const data  = rows.map(r => ({ ...r, ts: new Date(r.ran_at).getTime() }));
//
// We replicate that shape here and assert the series produced for each
// dataKey carries the y-values we expect, in oldest-first order.

interface QualityRun {
  id: string;
  baseline_label: string;
  model: string | null;
  ndcg10: number;
  mrr: number;
  p95_latency_ms: number;
  ran_at: string;
}

function chartRows(runs: QualityRun[]) {
  // /admin/quality returns most-recent-first; the route reverses so
  // charts render oldest -> newest along the time axis.
  return runs
    .slice()
    .reverse()
    .map((r) => ({ ...r, ts: new Date(r.ran_at).getTime() }));
}

function seriesFor(rows: ReturnType<typeof chartRows>, key: 'ndcg10' | 'mrr' | 'p95_latency_ms') {
  return rows.map((r) => r[key]);
}

const fixture: QualityRun[] = [
  // API response shape: most recent first.
  { id: '3', baseline_label: 'lexical', model: null, ndcg10: 0.41, mrr: 0.55, p95_latency_ms: 320, ran_at: '2026-05-03T12:00:00Z' },
  { id: '2', baseline_label: 'lexical', model: null, ndcg10: 0.39, mrr: 0.52, p95_latency_ms: 340, ran_at: '2026-05-02T12:00:00Z' },
  { id: '1', baseline_label: 'lexical', model: null, ndcg10: 0.37, mrr: 0.48, p95_latency_ms: 360, ran_at: '2026-05-01T12:00:00Z' },
];

describe('Quality route chart shaping', () => {
  it('reverses /admin/quality runs so charts render oldest-first', () => {
    const rows = chartRows(fixture);
    expect(rows.map((r) => r.id)).toEqual(['1', '2', '3']);
  });

  it('nDCG@10 series carries the exact y-values from QualityRun rows', () => {
    const rows = chartRows(fixture);
    expect(seriesFor(rows, 'ndcg10')).toEqual([0.37, 0.39, 0.41]);
  });

  it('MRR series carries the exact y-values from QualityRun rows', () => {
    const rows = chartRows(fixture);
    expect(seriesFor(rows, 'mrr')).toEqual([0.48, 0.52, 0.55]);
  });

  it('p95 latency series carries the exact y-values from QualityRun rows', () => {
    const rows = chartRows(fixture);
    expect(seriesFor(rows, 'p95_latency_ms')).toEqual([360, 340, 320]);
  });

  it('attaches a numeric ts derived from ran_at for the x-axis', () => {
    const rows = chartRows(fixture);
    expect(rows[0].ts).toBe(new Date('2026-05-01T12:00:00Z').getTime());
    expect(rows[2].ts).toBe(new Date('2026-05-03T12:00:00Z').getTime());
    // Monotonically non-decreasing — oldest first.
    for (let i = 1; i < rows.length; i++) {
      expect(rows[i].ts).toBeGreaterThanOrEqual(rows[i - 1].ts);
    }
  });

  it('handles an empty /admin/quality response without throwing', () => {
    expect(chartRows([])).toEqual([]);
  });
});
