import { useQuery } from '@tanstack/react-query';
import { api } from '../lib/api';

export function Health() {
  const { data, isLoading } = useQuery({
    queryKey: ['health'],
    queryFn: () => api.get<Record<string, any>>('/admin/health'),
    refetchInterval: 5000,
  });
  if (isLoading) return <div className="text-sm text-neutral-500">Loading…</div>;
  return (
    <div>
      <h1 className="text-xl font-medium mb-4">Health</h1>
      <pre className="bg-white border border-neutral-200 rounded p-4 text-xs whitespace-pre-wrap">{JSON.stringify(data, null, 2)}</pre>
    </div>
  );
}
