import { useState } from 'react';

export function CopyModal({ title, value, onClose }: { title: string; value: string; onClose: () => void }) {
  const [copied, setCopied] = useState(false);
  return (
    <div className="fixed inset-0 bg-black/50 flex items-center justify-center z-50">
      <div className="bg-white rounded-lg p-6 max-w-lg w-full shadow-xl">
        <h2 className="text-lg font-medium mb-2">{title}</h2>
        <p className="text-sm text-red-600 mb-3">⚠️ This is the only time this code will be shown.</p>
        <input value={value} readOnly
               className="w-full font-mono text-sm border border-neutral-300 rounded px-2 py-1.5 bg-neutral-50" />
        <div className="mt-4 flex justify-end gap-2">
          <button onClick={() => { navigator.clipboard.writeText(value); setCopied(true); }}
                  className="px-3 py-1.5 text-sm rounded bg-neutral-900 text-white hover:bg-neutral-800">
            {copied ? 'Copied ✓' : 'Copy'}
          </button>
          <button onClick={onClose} className="px-3 py-1.5 text-sm rounded bg-neutral-100 hover:bg-neutral-200">
            Close
          </button>
        </div>
      </div>
    </div>
  );
}
