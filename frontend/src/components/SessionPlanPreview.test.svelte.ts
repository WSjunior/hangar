// @vitest-environment happy-dom
import { afterEach, beforeEach, expect, it, vi } from 'vitest';
import { mount, tick, unmount } from 'svelte';
import { overwriteGetLocale } from '../paraglide/runtime';
import * as api from '@hangar/core';
import * as m from '../paraglide/messages';
import SessionPlanPreview from './SessionPlanPreview.svelte';

vi.mock('@hangar/core', async (original) => ({ ...await original<typeof api>(), getSessionPlanPreview: vi.fn() }));

const montados: ReturnType<typeof mount>[] = [];
const getPreview = vi.mocked(api.getSessionPlanPreview);

async function estabilizar() {
  for (let i = 0; i < 4; i++) {
    await tick();
    await new Promise((resolve) => setTimeout(resolve, 0));
  }
}

function botao(texto: string) {
  return [...document.querySelectorAll<HTMLButtonElement>('button')]
    .find((item) => item.textContent?.includes(texto));
}

async function montar(sobrescritas: Record<string, unknown> = {}) {
  const alvo = document.createElement('div');
  document.body.appendChild(alvo);
  const props = $state({
    sessionName: 'sessao', provider: 'claude', revision: 'r1', desktop: true,
    codexPlan: null, disabled: false, onImplement: vi.fn().mockResolvedValue(undefined),
    ...sobrescritas,
  });
  montados.push(mount(SessionPlanPreview, { target: alvo, props }));
  await estabilizar();
  return props;
}

beforeEach(() => {
  overwriteGetLocale(() => 'pt');
  vi.clearAllMocks();
  const valores = new Map<string, string>();
  const storage = {
    getItem: (chave: string) => valores.get(chave) ?? null,
    setItem: (chave: string, valor: string) => valores.set(chave, valor),
    removeItem: (chave: string) => valores.delete(chave),
    clear: () => valores.clear(),
    key: (indice: number) => [...valores.keys()][indice] ?? null,
    get length() { return valores.size; },
  };
  Object.defineProperty(globalThis, 'localStorage', { value: storage, configurable: true });
  Object.defineProperty(window, 'localStorage', { value: storage, configurable: true });
});

afterEach(async () => {
  for (const componente of montados.splice(0)) await unmount(componente);
  document.body.innerHTML = '';
});

it('busca somente metadados na montagem e conteúdo apenas ao abrir', async () => {
  getPreview.mockResolvedValueOnce({ name: 'meu-plano', path: '/planos/meu-plano.md' })
    .mockResolvedValueOnce({ name: 'meu-plano', path: '/planos/meu-plano.md', markdown: '# Título\n\n**Forte**' });
  await montar();

  expect(getPreview).toHaveBeenNthCalledWith(1, 'sessao', false);
  expect(getPreview).toHaveBeenCalledTimes(1);
  botao(m.chat_plan_ver())!.click();
  await estabilizar();

  expect(getPreview).toHaveBeenNthCalledWith(2, 'sessao');
  expect(document.querySelector('.prose h1')?.textContent).toBe('Título');
  expect(document.querySelector('.prose strong')?.textContent).toBe('Forte');
});

it('descobre na abertura e ao concluir o turno, sem reler quando começa a trabalhar', async () => {
  getPreview.mockResolvedValue(null);
  const props = await montar({ revision: 'working' });
  expect(getPreview).toHaveBeenCalledTimes(1);
  props.revision = 'idle'; await estabilizar();
  expect(getPreview).toHaveBeenCalledTimes(2);
  props.revision = 'working'; await estabilizar();
  expect(getPreview).toHaveBeenCalledTimes(2);
  props.revision = 'awaiting_input'; await estabilizar();
  expect(getPreview).toHaveBeenCalledTimes(3);
});

it('permite repetir descoberta que falhou durante o turno', async () => {
  getPreview.mockRejectedValueOnce(new Error('Indisponível')).mockResolvedValueOnce(null);
  await montar({ revision: 'working' });
  botao(m.lista_tentar_novamente())!.click(); await estabilizar();
  expect(getPreview).toHaveBeenCalledTimes(2);
  expect(document.querySelector('[role="alert"]')).toBeNull();
});

it('busca conteúdo novo toda vez que o plano é reaberto', async () => {
  getPreview.mockResolvedValueOnce({ name: 'plano', path: '/planos/plano.md' })
    .mockResolvedValueOnce({ name: 'plano', path: '/planos/plano.md', markdown: 'Versão um' })
    .mockResolvedValueOnce({ name: 'plano', path: '/planos/plano.md', markdown: 'Versão dois' });
  await montar();
  botao(m.chat_plan_ver())!.click(); await estabilizar();
  expect(document.querySelector('.prose')?.textContent).toContain('Versão um');
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); await estabilizar();
  botao(m.chat_plan_ver())!.click(); await estabilizar();

  expect(document.querySelector('.prose')?.textContent).toContain('Versão dois');
  expect(getPreview).toHaveBeenCalledTimes(3);
});

it('informa quando o arquivo foi removido e permite tentar novamente', async () => {
  getPreview.mockResolvedValueOnce({ name: 'plano', path: '/planos/plano.md' })
    .mockRejectedValueOnce(Object.assign(new Error('removido'), { status: 404 }));
  await montar();
  botao(m.chat_plan_ver())!.click(); await estabilizar();

  expect(document.querySelector('[role="alert"]')?.textContent).toBe(m.chat_plan_ausente());
  expect(botao(m.lista_tentar_novamente())).toBeTruthy();
});

it('fechar a visualização não executa o plano', async () => {
  const onImplement = vi.fn().mockResolvedValue(undefined);
  await montar({ provider: 'codex', codexPlan: '# Plano Codex', onImplement });
  botao(m.chat_plan_ver())!.click(); await estabilizar();
  window.dispatchEvent(new KeyboardEvent('keydown', { key: 'Escape', bubbles: true })); await estabilizar();

  expect(onImplement).not.toHaveBeenCalled();
});

it('Codex executa somente ao aprovar e continuar planejando apenas dispensa as ações', async () => {
  const executar = vi.fn().mockResolvedValue(undefined);
  await montar({ provider: 'codex', codexPlan: '# Primeiro', onImplement: executar });
  botao(m.chat_plan_implementar())!.click(); await estabilizar();
  expect(executar).toHaveBeenCalledWith('# Primeiro');

  await unmount(montados.pop()!);
  document.body.innerHTML = '';
  const continuar = vi.fn().mockResolvedValue(undefined);
  await montar({ provider: 'codex', codexPlan: '# Segundo', onImplement: continuar });
  botao(m.chat_plan_continuar())!.click(); await estabilizar();
  expect(continuar).not.toHaveBeenCalled();
  expect(botao(m.chat_plan_implementar())).toBeUndefined();
});

it('mantém a ação disponível e mostra o erro quando a execução falha', async () => {
  const executar = vi.fn().mockRejectedValue(new Error('Turno encerrado'));
  await montar({ provider: 'codex', codexPlan: '# Plano', onImplement: executar });
  botao(m.chat_plan_implementar())!.click(); await estabilizar();

  expect(document.querySelector('[role="alert"]')?.textContent).toBe('Turno encerrado');
  expect(botao(m.chat_plan_implementar())).toBeTruthy();
});
