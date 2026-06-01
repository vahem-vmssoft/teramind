import { useState } from 'react';
import { useQuery, useMutation, useQueryClient } from '@tanstack/react-query';
import { api } from '../lib/api';
import { CopyModal } from '../components/CopyModal';

interface Member {
  id: string;
  email: string;
  device_count: number;
  last_seen_at?: string;
  revoked_at?: string;
}
interface Invite { id: string; invited_email: string; expires_at: string }
interface Device { id: string; name: string; last_seen_at?: string }

export function Members() {
  const qc = useQueryClient();
  const members = useQuery({ queryKey: ['members'], queryFn: () => api.get<{ users: Member[] }>('/admin/members') });
  const invites = useQuery({ queryKey: ['invites'], queryFn: () => api.get<{ invites: Invite[] }>('/admin/invites') });
  const [issueOpen, setIssueOpen] = useState(false);
  const [generatedCode, setGeneratedCode] = useState<string | null>(null);
  const [expanded, setExpanded] = useState<string | null>(null);
  const [email, setEmail] = useState('');
  const [days, setDays] = useState(7);

  const create = useMutation({
    mutationFn: () => api.post<{ code: string }>('/admin/invites', { email, expires_in_days: days }),
    onSuccess: (data) => { setGeneratedCode(data.code); setIssueOpen(false); setEmail(''); qc.invalidateQueries({ queryKey: ['invites'] }); },
  });
  const revokeMember = useMutation({
    mutationFn: (uid: string) => api.post(`/admin/members/${uid}/revoke`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['members'] }),
  });
  const revokeInvite = useMutation({
    mutationFn: (iid: string) => api.post(`/admin/invites/${iid}/revoke`),
    onSuccess: () => qc.invalidateQueries({ queryKey: ['invites'] }),
  });
  const revokeDevice = useMutation({
    mutationFn: (did: string) => api.post(`/admin/devices/${did}/revoke`),
    onSuccess: () => qc.invalidateQueries(),
  });

  return (
    <div>
      <header className="flex justify-between items-center mb-4">
        <h1 className="text-xl font-medium">Members</h1>
        <button onClick={() => setIssueOpen(true)} className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white">+ Issue invite</button>
      </header>
      <div className="bg-white rounded border border-neutral-200 overflow-hidden">
        <table className="w-full text-sm">
          <thead className="bg-neutral-50 text-xs text-neutral-500 uppercase">
            <tr><th className="text-left px-4 py-2">Email</th><th className="text-left">Devices</th><th className="text-left">Last seen</th><th className="text-left">Status</th><th></th></tr>
          </thead>
          <tbody>
            {(members.data?.users ?? []).map(m => (
              <>
                <tr key={m.id} className="border-t border-neutral-100">
                  <td className="px-4 py-2">{m.email}</td>
                  <td>{m.device_count}</td>
                  <td>{m.last_seen_at ? new Date(m.last_seen_at).toLocaleString() : '—'}</td>
                  <td>{m.revoked_at ? 'revoked' : 'active'}</td>
                  <td className="text-right pr-4 space-x-2">
                    <button onClick={() => setExpanded(expanded === m.id ? null : m.id)}
                            className="text-xs text-neutral-600 hover:text-neutral-900">
                      {expanded === m.id ? 'hide' : 'devices'}
                    </button>
                    <button onClick={() => revokeMember.mutate(m.id)}
                            disabled={!!m.revoked_at}
                            className="text-xs text-red-600 hover:text-red-800 disabled:text-neutral-400">
                      revoke
                    </button>
                  </td>
                </tr>
                {expanded === m.id && <DeviceList userId={m.id} onRevoke={(d) => revokeDevice.mutate(d)} />}
              </>
            ))}
          </tbody>
        </table>
      </div>
      <h2 className="text-base font-medium mt-8 mb-2">Open invites</h2>
      <div className="bg-white rounded border border-neutral-200 overflow-hidden">
        <table className="w-full text-sm">
          <tbody>
            {(invites.data?.invites ?? []).map(i => (
              <tr key={i.id} className="border-t border-neutral-100">
                <td className="px-4 py-2">{i.invited_email}</td>
                <td>expires {new Date(i.expires_at).toLocaleDateString()}</td>
                <td className="text-right pr-4">
                  <button onClick={() => revokeInvite.mutate(i.id)} className="text-xs text-red-600 hover:text-red-800">revoke</button>
                </td>
              </tr>
            ))}
          </tbody>
        </table>
      </div>
      {issueOpen && (
        <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
          <div className="bg-white rounded-lg p-6 w-full max-w-sm shadow-xl">
            <h2 className="text-lg font-medium mb-3">Issue invite</h2>
            <input value={email} onChange={e => setEmail(e.target.value)} placeholder="email"
                   className="w-full border border-neutral-300 rounded px-2 py-1.5 text-sm" />
            <label className="text-xs text-neutral-500 mt-3 block">Expires in {days} days</label>
            <input type="range" min={1} max={30} value={days} onChange={e => setDays(Number(e.target.value))}
                   className="w-full mt-1" />
            <div className="flex justify-end gap-2 mt-4">
              <button onClick={() => setIssueOpen(false)} className="px-3 py-1.5 text-sm rounded bg-neutral-100">Cancel</button>
              <button onClick={() => create.mutate()} disabled={!email.includes('@')}
                      className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white disabled:opacity-50">
                Issue
              </button>
            </div>
          </div>
        </div>
      )}
      {generatedCode && (
        <CopyModal title="Invite code" value={generatedCode} onClose={() => setGeneratedCode(null)} />
      )}
    </div>
  );
}

function DeviceList({ userId, onRevoke }: { userId: string; onRevoke: (deviceId: string) => void }) {
  const { data } = useQuery({
    queryKey: ['user-devices', userId],
    queryFn: () => api.get<Device[]>(`/admin/members/${userId}/devices`),
  });
  const devices = data ?? [];
  return (
    <tr><td colSpan={5} className="bg-neutral-50 px-4 py-2 text-xs">
      {devices.length === 0 ? '(no active devices)' : (
        <ul className="space-y-1">
          {devices.map(d => (
            <li key={d.id} className="flex justify-between">
              <span className="font-mono">{d.name}</span>
              <span className="text-neutral-500">{d.last_seen_at ?? 'never'}</span>
              <button onClick={() => onRevoke(d.id)} className="text-red-600 hover:text-red-800">revoke</button>
            </li>
          ))}
        </ul>
      )}
    </td></tr>
  );
}
