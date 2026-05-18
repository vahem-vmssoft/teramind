import { describe, it, expect } from 'vitest';

// Pure reducer extracted from the hook — keep it simple in v1.
function appendEvents(prev: any[], next: any, cap = 200): any[] {
  return [next, ...prev].slice(0, cap);
}

describe('event stream', () => {
  it('prepends new events', () => {
    const out = appendEvents([{ id: 1 }], { id: 2 });
    expect(out[0].id).toBe(2);
    expect(out.length).toBe(2);
  });
  it('caps at the limit', () => {
    let s: any[] = [];
    for (let i = 0; i < 250; i++) s = appendEvents(s, { id: i }, 200);
    expect(s.length).toBe(200);
    expect(s[0].id).toBe(249);
  });
});
