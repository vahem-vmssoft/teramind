import { useEffect, useRef, useState } from 'react';

export interface TeamEvent {
  type: 'session_ended' | 'wiki_page_ready' | 'skill_saved';
  session_id?: string;
  user_id?: string;
  cwd?: string;
  title?: string;
  name?: string;
  ts: string;
}

export function useEventStream(enabled: boolean): TeamEvent[] {
  const [events, setEvents] = useState<TeamEvent[]>([]);
  const wsRef = useRef<WebSocket | null>(null);

  useEffect(() => {
    if (!enabled) return;
    const proto = window.location.protocol === 'https:' ? 'wss' : 'ws';
    const ws = new WebSocket(`${proto}://${window.location.host}/admin/events`);
    wsRef.current = ws;
    ws.onmessage = (e) => {
      try {
        const evt = JSON.parse(e.data) as TeamEvent | { type: 'hello' };
        if ((evt as TeamEvent).ts) {
          setEvents(prev => [evt as TeamEvent, ...prev].slice(0, 200));
        }
      } catch { /* ignore */ }
    };
    ws.onclose = () => { wsRef.current = null; };
    return () => { ws.close(); };
  }, [enabled]);

  return events;
}
