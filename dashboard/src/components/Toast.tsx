import { useEffect, useState } from 'react';

export function Toast({ message, onClose }: { message: string; onClose: () => void }) {
  useEffect(() => {
    const t = setTimeout(onClose, 4000);
    return () => clearTimeout(t);
  }, [onClose]);
  return (
    <div className="fixed bottom-6 right-6 bg-red-600 text-white rounded-md shadow-lg px-4 py-3 text-sm">
      {message}
    </div>
  );
}

let _setter: ((s: string | null) => void) | null = null;
export function ToastHost() {
  const [msg, setMsgState] = useState<string | null>(null);
  _setter = setMsgState;
  return msg ? <Toast message={msg} onClose={() => setMsgState(null)} /> : null;
}
export function showToast(msg: string) { _setter?.(msg); }
