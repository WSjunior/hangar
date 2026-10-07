<script lang="ts">
  import { onMount, untrack } from 'svelte';
  import * as m from '../paraglide/messages';
  import { baseOf, fileAuthHeader, frameHeight, pageFetchState, pageUrls, reservedHeight, themeVariables,
    type HtmlPageRef, type PageFetchState } from '@hangar/core';
  import { useSessionServer } from '../lib/sessionServer';
  import { getRouteBaseUrl } from '../lib/auth';
  import { useFollowTail } from '../lib/followTail';

  let { page, sessionName }: { page: HtmlPageRef; sessionName: string } = $props();
  const sessionServer = useSessionServer();
  const followTail = useFollowTail();

  let status = $state<PageFetchState>('loading');
  let doc = $state('');
  let reported = $state<number | null>(null);
  let width = $state(0);
  let scheme = $state<'dark' | 'light'>(readScheme());
  let frame: HTMLIFrameElement | undefined = $state();
  let box: HTMLDivElement | undefined = $state();

  const height = $derived(width ? frameHeight(page, width, reported) : reservedHeight(page, 728));

  function readScheme(): 'dark' | 'light' {
    return document.documentElement.dataset.theme === 'light' ? 'light' : 'dark';
  }

  // Nome da variável na página → token do app.
  const MAP: Record<string, string> = {
    '--foreground': '--text-primary', '--muted-foreground': '--text-muted', '--surface': '--bg-surface',
    '--border': '--border-default', '--accent': '--accent', '--accent-foreground': '--text-inverse',
    '--danger': '--error', '--warning': '--warning', '--success': '--success', '--code-background': '--bg-base',
    '--chart-1': '--chart-1', '--chart-2': '--chart-2', '--chart-3': '--chart-3', '--chart-4': '--chart-4',
    '--font-sans': '--font-ui', '--font-mono': '--font-mono',
  };

  function vars() {
    const cs = getComputedStyle(document.documentElement);
    return themeVariables((name) => cs.getPropertyValue(MAP[name] ?? name));
  }

  // A tag vai partida: escrita inteira dentro do <script>, o pré-processador do Svelte a toma por um bloco de estilo.
  const TAG = 'style';
  const THEME_BLOCK = new RegExp(`(<${TAG} id="hangar-theme" data-base="([^"]*)">)[\\s\\S]*?(</${TAG}>)`);

  function themed(html: string) {
    const css = `:root{color-scheme:${scheme};${Object.entries(vars()).map(([k, v]) => `${k}:${v}`).join(';')}}`;
    // Troca só o conteúdo do bloco de tema injetado pelo servidor; o data-base fica para as trocas seguintes.
    return html.replace(THEME_BLOCK,
      (_all, open: string, base: string, close: string) => `${open}${css}${base.replaceAll('&quot;', '"')}${close}`);
  }

  let seq = 0;

  async function load() {
    // Resposta de um pedido antigo (nova tentativa no meio) não sobrescreve a do mais novo.
    const mine = ++seq;
    status = 'loading';
    const server = sessionServer();
    try {
      const r = await fetch(pageUrls(server ? baseOf(server) : getRouteBaseUrl(), sessionName, page.id).raw,
        { headers: fileAuthHeader(server) });
      // O 404 da página vencida traz o código; sem ele (convidado, rota ausente) é erro.
      const code = r.status === 404 ? await r.json().then((b) => b?.detail?.code, () => null) : null;
      const next = pageFetchState(r.status, code);
      // Tema próprio: a página fica como foi desenhada, sem as cores do app.
      const html = next === 'ready' ? await r.text() : null;
      if (mine !== seq) return;
      if (html != null) doc = page.ownTheme ? html : themed(html);
      status = next;
    } catch {
      if (mine === seq) status = pageFetchState('network');
    }
  }

  function onMessage(e: MessageEvent) {
    if (!frame || e.source !== frame.contentWindow) return;
    const d = e.data;
    if (d?.method === 'ui/notifications/size-changed' && Number.isFinite(d.params?.height)) {
      reported = d.params.height;
    } else if (d?.method === 'ui/open-link' && typeof d.params?.url === 'string' && /^https?:/i.test(d.params.url)
      // Só com gesto real: a página não abre aba sozinha.
      && (navigator as Navigator & { userActivation?: { isActive: boolean } }).userActivation?.isActive) {
      window.open(d.params.url, '_blank', 'noopener,noreferrer');
    }
  }

  $effect(() => {
    void height;
    // Só a altura dispara; o estado de rolagem da lista que a função lê não entra como dependência.
    untrack(followTail);
  });

  $effect(() => {
    // Tema do app mudou: a página recebe as variáveis novas sem recarregar.
    const theme = scheme;
    if (page.ownTheme) return;
    frame?.contentWindow?.postMessage({ jsonrpc: '2.0', method: 'ui/notifications/host-context-changed',
      params: { theme, styles: { variables: vars() } } }, '*');
  });

  onMount(() => {
    // Site de verdade só vive no app desktop; aqui o cartão só abre o endereço.
    if (!page.url) void load();
    const ro = new ResizeObserver(([e]) => (width = e.contentRect.width));
    if (box) ro.observe(box);
    const mo = new MutationObserver(() => (scheme = readScheme()));
    mo.observe(document.documentElement, { attributes: true, attributeFilter: ['data-theme'] });
    return () => { ro.disconnect(); mo.disconnect(); };
  });
</script>

<svelte:window onmessage={onMessage} />

{#if page.url}
  <div class="site">
    <span class="site-title">{page.title}</span>
    <span class="site-url">{page.url}</span>
    <button type="button" class="link" onclick={() => window.open(page.url, '_blank', 'noopener,noreferrer')}>
      {m.page_open_browser()}</button>
  </div>
{:else}
<!-- A altura reservada só vale para a página (e enquanto ela carrega); aviso de erro fica do tamanho do texto. -->
<div class="page" bind:this={box} style:height={status === 'ready' || status === 'loading' ? `${height}px` : undefined}
  aria-busy={status === 'loading'}>
  {#if status === 'ready'}
    <!-- Sem allow-same-origin: a página roda em origem opaca, longe do token e do localStorage do app.
         Sem allow-popups: link só abre pela ponte ui/open-link, com gesto do leitor. -->
    <iframe bind:this={frame} title={page.title} srcdoc={doc} sandbox="allow-scripts"
      referrerpolicy="no-referrer" style:color-scheme={page.ownTheme ? 'normal' : scheme}></iframe>
  {:else if status === 'loading'}
    <p class="note">{m.page_loading({ title: page.title })}</p>
  {:else if status === 'expired'}
    <p class="note">{m.page_expired()}</p>
  {:else}
    <p class="note" role="alert">{m.page_error({ title: page.title })}
      <button type="button" class="link" onclick={load}>{m.page_retry()}</button></p>
  {/if}
</div>
{/if}

<style>
  .page { position: relative; width: 100%; margin-bottom: var(--space-1); background: transparent; }
  iframe { display: block; width: 100%; height: 100%; border: 0; background: transparent; }
  .site { display: flex; flex-direction: column; align-items: flex-start; gap: 2px; padding: 8px 0; margin-bottom: var(--space-1); }
  .site-title { color: var(--text-primary); font-size: var(--text-sm); font-weight: 600; }
  .site-url { color: var(--text-muted); font-size: var(--text-xs); overflow-wrap: anywhere; }
  .site .link { margin-left: 0; }
  .note { margin: 0; padding: 12px 0; color: var(--text-muted); font-size: var(--text-sm); }
  .link {
    min-height: 0; min-width: 0; margin-left: 8px; padding: 0; border: 0; background: none;
    font: inherit; color: var(--accent); cursor: pointer;
  }
</style>
