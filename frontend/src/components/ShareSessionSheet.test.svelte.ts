// @vitest-environment happy-dom
import { describe, it, expect, vi, beforeEach, afterEach } from 'vitest';
import { mount, unmount, tick } from 'svelte';
import * as m from '../paraglide/messages';

const core = vi.hoisted(() => ({
  listShares: vi.fn(), createShare: vi.fn(), revokeShare: vi.fn(), revokeAllShares: vi.fn(), sharePrereqs: vi.fn(),
}));
vi.mock('../lib/auth', () => ({ listServers: () => [{ id: 'srv-a', label: 'A', baseUrl: 'http://a', token: 't' }] }));
vi.mock('../lib/clipboard', () => ({ copyText: vi.fn(async () => {}) }));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  ...core,
}));

const ShareSessionSheet = (await import('./ShareSessionSheet.svelte')).default;
const { SharePrerequisiteError } = await import('@hangar/core');

async function settle() { for (let i = 0; i < 6; i++) { await Promise.resolve(); await tick(); } }
function montar(props: { open: boolean } = { open: true }) {
  const el = document.createElement('div');
  document.body.appendChild(el);
  return mount(ShareSessionSheet, { target: el, props: { get open() { return props.open; }, name: 's1', serverId: 'srv-a', onClose: vi.fn() } });
}
const botao = (t: string) => [...document.querySelectorAll('button')].find((b) => b.textContent?.trim() === t)!;
const agora = () => Math.floor(Date.now() / 1000);

beforeEach(() => {
  document.body.innerHTML = '';
  for (const f of Object.values(core)) f.mockReset();
});

describe('ShareSessionSheet', () => {
  it('sem acesso ainda: aviso de confiança e estado vazio', async () => {
    core.listShares.mockResolvedValue({ shares: [] });
    const c = montar();
    await settle();
    expect(document.body.textContent).toContain(m.compartilhar_aviso_confianca());
    expect(document.body.textContent).toContain(m.compartilhar_vazio());
    // Abrir só carrega a lista: o link nasce no botão e aparece uma vez (o backend guarda só o hash).
    expect(core.createShare).not.toHaveBeenCalled();
    unmount(c);
  });

  it('gerar link mostra o link e o WhatsApp leva o link no texto', async () => {
    core.listShares.mockResolvedValue({ shares: [] });
    core.createShare.mockResolvedValue({ id: 'x1', link: 'https://d.ts.net:8443/convite/K7P2', expires_at: agora() + 86400 });
    const c = montar();
    await settle();
    botao(m.compartilhar_gerar()).click();
    await settle();
    expect(core.createShare).toHaveBeenCalledWith('s1', false, expect.objectContaining({ id: 'srv-a' }));
    const campo = document.querySelector<HTMLInputElement>(`input[aria-label="${m.compartilhar_link_novo()}"]`)!;
    expect(campo.value).toBe('https://d.ts.net:8443/convite/K7P2');
    const wa = document.querySelector<HTMLAnchorElement>('a[href^="https://wa.me/"]')!;
    expect(decodeURIComponent(wa.href)).toContain('https://d.ts.net:8443/convite/K7P2');
    unmount(c);
  });

  it('pré-requisito faltando: mostra o que falta e o comando', async () => {
    core.listShares.mockResolvedValue({ shares: [] });
    core.createShare.mockRejectedValue(new SharePrerequisiteError(['funnel'], 'https://login.tailscale.com/f/funnel', 'x'));
    const c = montar();
    await settle();
    botao(m.compartilhar_gerar()).click();
    await settle();
    expect(document.body.textContent).toContain(m.compartilhar_falta_funnel());
    expect(document.body.textContent).toContain('https://login.tailscale.com/f/funnel');
    unmount(c);
  });

  describe('liberar no Tailscale', () => {
    const LIBERAR = 'https://login.tailscale.com/f/funnel?node=n1';
    const liberar = () => document.querySelector<HTMLAnchorElement>(`a[href="${LIBERAR}"]`);
    async function bloqueado(missing = ['funnel'], fix = 'libere', enableUrl: string | null = LIBERAR) {
      core.listShares.mockResolvedValue({ shares: [] });
      core.createShare.mockRejectedValue(new SharePrerequisiteError(missing, fix, 'x', enableUrl));
      botao(m.compartilhar_gerar()).click();
      await settle();
    }
    // O link abre em outra aba; no teste só interessa o clique chegar ao componente.
    function clicarLiberar() {
      document.addEventListener('click', (e) => e.preventDefault(), { capture: true, once: true });
      liberar()!.click();
    }

    beforeEach(() => vi.useFakeTimers());
    afterEach(() => vi.useRealTimers());

    it('confere a cada 3 s e, liberado, some o aviso e o gerar volta', async () => {
      core.listShares.mockResolvedValue({ shares: [] });
      const c = montar();
      await settle();
      await bloqueado();
      clicarLiberar();
      await settle();
      expect(botao(m.compartilhar_gerar()).disabled).toBe(true);
      expect(document.body.textContent).toContain(m.compartilhar_conferindo());

      core.sharePrereqs.mockResolvedValueOnce({ missing: ['funnel'], fix: 'libere', enable_url: LIBERAR });
      await vi.advanceTimersByTimeAsync(3000);
      await settle();
      expect(liberar()).not.toBeNull();

      core.sharePrereqs.mockResolvedValueOnce({ missing: [], fix: '', enable_url: null });
      await vi.advanceTimersByTimeAsync(3000);
      await settle();
      expect(liberar()).toBeNull();
      expect(document.body.textContent).not.toContain(m.compartilhar_pre_requisito());
      expect(botao(m.compartilhar_gerar()).disabled).toBe(false);
      await vi.advanceTimersByTimeAsync(9000);
      expect(core.sharePrereqs).toHaveBeenCalledTimes(2);
      unmount(c);
    });

    it('fechar a folha para a conferência', async () => {
      core.listShares.mockResolvedValue({ shares: [] });
      core.sharePrereqs.mockResolvedValue({ missing: ['funnel'], fix: 'libere', enable_url: LIBERAR });
      const props = $state({ open: true });
      const c = montar(props);
      await settle();
      await bloqueado();
      clicarLiberar();
      await vi.advanceTimersByTimeAsync(3000);
      expect(core.sharePrereqs).toHaveBeenCalledTimes(1);
      props.open = false;
      await settle();
      await vi.advanceTimersByTimeAsync(30_000);
      expect(core.sharePrereqs).toHaveBeenCalledTimes(1);
      unmount(c);
    });

    it('para sozinha depois de 5 min', async () => {
      core.listShares.mockResolvedValue({ shares: [] });
      core.sharePrereqs.mockRejectedValue(new Error('409'));
      const c = montar();
      await settle();
      await bloqueado();
      clicarLiberar();
      await vi.advanceTimersByTimeAsync(5 * 60_000 + 6000);
      const n = core.sharePrereqs.mock.calls.length;
      expect(n).toBeGreaterThanOrEqual(99);
      await vi.advanceTimersByTimeAsync(30_000);
      expect(core.sharePrereqs).toHaveBeenCalledTimes(n);
      expect(botao(m.compartilhar_gerar()).disabled).toBe(false);
      unmount(c);
    });

    it('operador faltando: copia só a linha do comando e dá a dica do app nativo', async () => {
      core.listShares.mockResolvedValue({ shares: [] });
      const c = montar();
      await settle();
      await bloqueado(['operator', 'funnel'], 'sudo tailscale set --operator=$USER\nlibere o Funnel');
      const { copyText } = await import('../lib/clipboard');
      botao(m.compartilhar_copiar()).click();
      await settle();
      expect(copyText).toHaveBeenCalledWith('sudo tailscale set --operator=$USER');
      expect(document.body.textContent).toContain(m.compartilhar_operador_dica());
      unmount(c);
    });
  });

  it('revogar um acesso chama a rota com a sessão e o id, e recarrega', async () => {
    core.listShares.mockResolvedValue({ shares: [
      { id: 'a1', device: 'Browser · Linux', created_at: agora() - 60, redeemed_at: agora() - 30, expires_at: agora() + 86000, pending: false },
    ] });
    core.revokeShare.mockResolvedValue({ ok: true });
    const c = montar();
    await settle();
    expect(document.body.textContent).toContain('Browser · Linux');
    botao(m.compartilhar_revogar()).click();
    await settle();
    expect(core.revokeShare).toHaveBeenCalledWith('s1', 'a1', expect.objectContaining({ id: 'srv-a' }));
    expect(core.listShares).toHaveBeenCalledTimes(2);
    unmount(c);
  });

  it('falha ao carregar: mostra o erro e tenta de novo', async () => {
    core.listShares.mockRejectedValueOnce(new Error('caiu')).mockResolvedValue({ shares: [] });
    const c = montar();
    await settle();
    expect(document.body.textContent).toContain(m.compartilhar_erro_lista({ erro: 'caiu' }));
    botao(m.compartilhar_tentar_de_novo()).click();
    await settle();
    expect(document.body.textContent).toContain(m.compartilhar_vazio());
    unmount(c);
  });
});
