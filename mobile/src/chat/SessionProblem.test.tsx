// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it } from 'vitest';
import * as m from '../paraglide/messages';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

import { SessionProblem } from './SessionProblem';

async function texto(problem: string) {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(createElement(SessionProblem, { problem, detail: null })));
  const out = container.textContent;
  act(() => root.unmount());
  return out;
}

describe('SessionProblem', () => {
  it('traduz os códigos do estado do Rust', async () => {
    expect(await texto('state_facts_unavailable')).toBe(m.problema_state_facts_unavailable());
    expect(await texto('permission_observe_failed')).toBe(m.problema_permission_observe_failed());
  });
});
