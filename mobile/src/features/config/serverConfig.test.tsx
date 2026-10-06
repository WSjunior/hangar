// @vitest-environment happy-dom
import { act, createElement } from 'react';
import { createRoot } from 'react-dom/client';
import { describe, expect, it, vi } from 'vitest';
import { useServerConfig, type ServerConfig } from './serverConfig';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const calls = vi.hoisted(() => ({ get: vi.fn(), patch: vi.fn() }));
const server = { id: 'server-a', label: 'A', baseUrl: 'https://a.local', token: 't' };

vi.mock('../../stores/servers', () => ({
  useServers: (selector: (state: { active: () => typeof server }) => unknown) => selector({ active: () => server }),
}));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  getConfigForServer: calls.get,
  patchConfigForServer: calls.patch,
}));

const STORED = { id: 'a', kind: 'elevenlabs', name: '', base_url: '', api_key: 'xi_••••', model: '' };

describe('useServerConfig', () => {
  it('segura o Salvar enquanto um serviço de transcrição não tem chave', async () => {
    const campos = { transcription_providers: { valor: [STORED], definido: true, origem: 'app' } };
    calls.get.mockResolvedValue({ campos });
    calls.patch.mockResolvedValue({ campos });
    let cfg!: ServerConfig;
    const Probe = () => { cfg = useServerConfig(); return null; };
    const root = createRoot(document.createElement('div'));
    await act(async () => root.render(createElement(Probe)));
    expect(cfg.load.status).toBe('ready');
    expect(cfg.saveBlocked).toBe(false);

    act(() => { cfg.stage('transcription_providers', [STORED, { ...STORED, id: 'b', api_key: '' }]); });
    expect(cfg.saveBlocked).toBe(true);
    act(() => cfg.save());
    expect(calls.patch).not.toHaveBeenCalled();

    const typed = [STORED, { ...STORED, id: 'b', api_key: 'nova' }];
    act(() => { cfg.stage('transcription_providers', typed); });
    expect(cfg.saveBlocked).toBe(false);
    await act(async () => cfg.save());
    expect(calls.patch).toHaveBeenCalledWith(server, { transcription_providers: typed });
    root.unmount();
  });
});
