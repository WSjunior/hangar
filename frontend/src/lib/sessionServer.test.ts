// @vitest-environment happy-dom
// O chat fixa o servidor da sessão na entrada. Depois disso o ativo pode mudar (overlay,
// withServer, outra aba) e as chamadas da sessão continuam indo à máquina dela.
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import {
  configureApi, getSessionPlanPreview, getPermissionModes, getRunners, getSubagents, getOrqGrupo,
  sendInput, pressPluginButton, uploadUrl,
} from '@hangar/core';
import { getActiveId, getRouteBaseUrl, getToken, selectServer, type Server } from './auth';
import { sessionServerFor } from './sessionServer';

const PRINCIPAL: Server = { id: 'principal', label: 'P', baseUrl: 'http://p.local:8765', token: 't-p' };
const NOTEBOOK: Server = { id: 'notebook', label: 'N', baseUrl: 'http://n.local:8765', token: 't-n' };

let chamadas: { url: string; token: string | null }[] = [];
let resposta: { status: number; corpo: unknown } = { status: 200, corpo: {} };
const onUnauthorized = vi.fn();

beforeEach(() => {
  chamadas = [];
  resposta = { status: 200, corpo: {} };
  onUnauthorized.mockReset();
  localStorage.setItem('cp_servers', JSON.stringify([PRINCIPAL, NOTEBOOK]));
  localStorage.setItem('cp_active', 'notebook');
  // O mesmo ambiente do main.ts: o caminho sem servidor lê o ativo NA HORA da chamada.
  configureApi({
    getBaseUrl: getRouteBaseUrl,
    getToken,
    onUnauthorized,
    origin: 'http://outra.origem',
    createEventSource: () => ({}) as unknown as import('@hangar/core').EventSourceLike,
  });
  vi.spyOn(globalThis, 'fetch').mockImplementation(async (input: RequestInfo | URL, init?: RequestInit) => {
    chamadas.push({ url: String(input), token: new Headers(init?.headers as HeadersInit).get('Authorization') });
    const corpo = JSON.stringify(resposta.corpo);
    return new Response(corpo, { status: resposta.status, headers: { 'Content-Type': 'application/json' } });
  });
});
afterEach(() => vi.restoreAllMocks());

describe('servidor da sessão fixado no chat', () => {
  it.each([
    ['plan-preview', (s: Server) => getSessionPlanPreview('hangar', false, s), '/api/sessions/hangar/plan-preview?content=false'],
    ['permission-modes', (s: Server) => getPermissionModes('hangar', false, s), '/api/sessions/hangar/permission-modes'],
    ['runners', (s: Server) => getRunners('hangar', s), '/api/sessions/hangar/runners'],
    ['subagents', (s: Server) => getSubagents('hangar', s), '/api/sessions/hangar/subagents'],
    ['orq', (s: Server) => getOrqGrupo('hangar', s), '/api/sessions/hangar/orq'],
    ['input', (s: Server) => sendInput('hangar', 'oi', s), '/api/sessions/hangar/input'],
    ['plugin/press', (s: Server) => pressPluginButton('hangar', 'faixa', 'k', s), '/api/sessions/hangar/plugin/press'],
  ])('%s continua no servidor da sessão depois de o ativo mudar', async (_rota, chamar, caminho) => {
    const sessao = sessionServerFor(getActiveId() ?? '');
    selectServer('principal');
    await chamar(sessao()!);
    expect(chamadas).toEqual([{ url: `http://n.local:8765${caminho}`, token: 'Bearer t-n' }]);
  });

  it('URL de anexo leva o endereço e o token da sessão', () => {
    const sessao = sessionServerFor('notebook');
    selectServer('principal');
    expect(uploadUrl('hangar', 'a.png', false, sessao())).toBe(
      'http://n.local:8765/api/sessions/hangar/uploads/a.png?token=t-n');
  });

  // Controle: sem servidor, a chamada segue o ativo do momento. É o caminho fora do chat.
  it('sem servidor, vai ao ativo', async () => {
    selectServer('principal');
    await getRunners('hangar');
    expect(chamadas).toEqual([{ url: 'http://p.local:8765/api/sessions/hangar/runners', token: 'Bearer t-p' }]);
  });

  it('dono fixado que saiu da lista é erro, nunca o ativo', () => {
    const sessao = sessionServerFor('notebook');
    localStorage.setItem('cp_servers', JSON.stringify([PRINCIPAL]));
    expect(() => sessao()).toThrow();
    expect(sessionServerFor('')()).toBeUndefined();
  });

  it('erro da sessão sai na forma do apiFetch: mensagem limpa, status e código', async () => {
    selectServer('principal');
    resposta = { status: 404, corpo: { detail: { code: 'erro_x', msg: 'sessão não encontrada' } } };
    const erro = await sendInput('hangar', 'oi', NOTEBOOK).catch((e: unknown) => e);
    expect(erro).toMatchObject({ status: 404, code: 'erro_x' });
    expect((erro as Error).message).not.toMatch(/^404:/);
  });

  it('401 de outra máquina não derruba a credencial ativa', async () => {
    selectServer('principal');
    resposta = { status: 401, corpo: { detail: 'token' } };
    await expect(getRunners('hangar', NOTEBOOK)).rejects.toMatchObject({ status: 401 });
    expect(onUnauthorized).not.toHaveBeenCalled();
  });

  it('401 da própria credencial ativa segue derrubando, como antes', async () => {
    resposta = { status: 401, corpo: { detail: 'token' } };
    await expect(getRunners('hangar', NOTEBOOK)).rejects.toMatchObject({ status: 401 });
    expect(onUnauthorized).toHaveBeenCalledTimes(1);
  });
});
