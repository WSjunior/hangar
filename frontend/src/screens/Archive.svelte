<script lang="ts">
  import NavBar from '../components/NavBar.svelte';
import { intlLocale } from '../lib/locale';
  import MessageList from '../components/MessageList.svelte';
  import Select from '../components/Select.svelte';
  import * as m from '../paraglide/messages';
  import {
    getArchiveFolder, getArchiveHistory, archiveImageUrl, resumeArchivedConversation, sendInputForServer,
    getEngines, listClaudeConfigs, getCodexAccountsForServer, takeEntry,
    type ArchiveFolder, type ArchiveEntry, type Motor, type ConfigDirInfo, type CodexAccount,
  } from '@hangar/core';
  import NewChatComposer from '../components/newchat/NewChatComposer.svelte';
  import AccountPill from '../components/newchat/AccountPill.svelte';
  import { arquivo, clienteQuery } from '../lib/queries';
  import { keyboardInset } from '../lib/keyboardInset';
  import type { ChatEvent } from '@hangar/core';
  import { selectServer, listOwnServers, getActiveId, serverColor } from '../lib/auth';
  import ProviderGlyph from '../components/icons/ProviderGlyph.svelte';
  import { basename, providerName } from '@hangar/core';
  import { draftStorageKey, parseStoredDraft, readMigrating } from '../lib/dictationStore.svelte';

  interface Props {
    onBack: () => void;
    // Deep-link vindo da busca (feature #10): abre direto uma conversa arquivada de um servidor
    // especifico, sem passar pela navegacao pasta-a-pasta.
    deepLink?: { serverId: string; project: string; sessionId: string; eventId?: string | null } | null;
  }
  let { onBack, deepLink = null }: Props = $props();

  // Navegacao pasta-primeiro (3 niveis, estado interno): pastas -> conversas da pasta -> leitor.
  let loading = $state(true);
  let error = $state('');
  let folders = $state<ArchiveFolder[]>([]);
  let folder = $state<ArchiveFolder | null>(null);
  let entries = $state<ArchiveEntry[]>([]);
  let loadingEntries = $state(false);
  let selected = $state<ArchiveEntry | null>(null);
  let events = $state<ChatEvent[]>([]);
  let loadingChat = $state(false);
  let resuming = $state(false);
  let resumeError = $state('');
  let draft = $state('');
  let showEngine = $state(false);
  // Motor pro resume (Task 5, item 1 do review): o pane original morreu, entao o app NAO sabe qual
  // motor rodava a conversa -- so o nome do modelo fica no transcript, nao qual dos motores do
  // usuario o produziu. '' = conta Anthropic (default de hoje). Falha ao listar -> sem seletor,
  // resume segue sem motor (nao pode travar o resume).
  let engine = $state('');
  let motores = $state<Record<string, Motor>>({});
  // Guard de corrida (mesmo padrao do cfgSeq em CreateSessionSheet): Archive navega multi-servidor,
  // e dois toques rapidos em conversas de servidores DIFERENTES deixam getEngines() em voo -- sem
  // isto a resposta do servidor ERRADO aterrissa por cima da certa e oferece motores de outro host.
  let motorSeq = 0;
  // Contas da conversa aberta: a lista do servidor dela, a escolha do usuário (null = não mexeu,
  // vale a dona da conversa) e o aviso de conta sem limite que a pílula devolve.
  let configs = $state<ConfigDirInfo[]>([]);
  let codexAccounts = $state<CodexAccount[]>([]);
  let accountsLoading = $state(false);
  let pickedConfig = $state<string | null>(null);
  let accountBlocked = $state<string | null>(null);
  let accountSeq = 0;
  // config_dir null = conta do próprio backend, que a lista marca como ativa.
  const selectedAccount = $derived(selected?.provider === 'codex'
    ? (selected.codex_account ?? null)
    : (pickedConfig ?? selected?.config_dir ?? configs.find((c) => c.active)?.path ?? null));
  // Motor próprio não gasta a conta Claude: a trava não vale.
  const blockedNow = $derived(selected?.provider === 'claude' && !engine ? accountBlocked : null);

  // Servidor DE ONDE navegar o arquivo: apiFetch usa o servidor ATIVO, entao sem um seletor o arquivo
  // so mostrava o servidor ativo e nao dava pra saber/escolher de qual servidor abrir (multi-servidor).
  const servers = listOwnServers();
  let activeServerId = $state(getActiveId());
  function pickServer(id: string) {
    if (id === activeServerId) return;
    motorSeq++;
    selectServer(id);
    activeServerId = id;
    folder = null;      // volta pro nivel de pastas do servidor novo
    selected = null;
    loadingChat = false;
    resuming = false;
    load();
  }

  // Pastas só são buscadas quando a tela precisa delas (entrada pela lista de conversas não precisa).
  let foldersLoaded = false;
  async function load() {
    foldersLoaded = true;
    loading = true;
    error = '';
    try {
      folders = await clienteQuery.fetchQuery(arquivo());
    } catch (e) {
      error = e instanceof Error ? e.message : m.arquivo_carregar_erro();
    } finally {
      loading = false;
    }
  }
  $effect(() => {
    if (deepLink) {
      motorSeq++;
      selected = null;
      loadingChat = false;
      resuming = false;
      // Aponta pro servidor dono ANTES de qualquer fetch (apiFetch le o ativo na hora da chamada),
      // carrega as pastas por baixo (pro "voltar" da conversa cair na lista) e abre a conversa direto.
      selectServer(deepLink.serverId);
      activeServerId = deepLink.serverId;   // mantem o seletor coerente ao voltar da conversa
      const link = deepLink;
      const known = takeEntry(link.serverId, link.project, link.sessionId);
      if (known) {
        foldersLoaded = false;
        loading = false;
        void openConversation(known);
      } else {
        load();
        getArchiveFolder(link.project).then((items) => {
          if (deepLink !== link || activeServerId !== link.serverId) return;
          const matches = items.filter((item) => item.session_id === link.sessionId);
          if (matches.length === 1) void openConversation(matches[0]);
          else error = matches.length > 1 ? m.arquivo_conversa_conta_ambigua() : m.arquivo_conversa_erro();
        }).catch(() => {
          if (deepLink === link && activeServerId === link.serverId) error = m.arquivo_conversa_erro();
        });
      }
    } else {
      load();
    }
  });

  async function openFolder(f: ArchiveFolder) {
    folder = f;
    loadingEntries = true;
    entries = [];
    try {
      entries = await getArchiveFolder(f.project);
    } catch {
      error = m.arquivo_pasta_erro();
      folder = null;
    } finally {
      loadingEntries = false;
    }
  }

  async function openConversation(e: ArchiveEntry) {
    const server = servers.find((s) => s.id === activeServerId);
    selected = e;
    loadingChat = true;
    events = [];
    resumeError = '';
    draft = '';
    showEngine = false;
    engine = '';
    motores = {};
    pickedConfig = null;
    accountBlocked = null;
    configs = [];
    codexAccounts = [];
    const accSeq = ++accountSeq;
    if (e.provider === 'claude' || e.provider === 'codex') {
      accountsLoading = true;
      // Lista de contas sem variante por servidor no Claude: usa o ativo, que já é o desta conversa.
      const accounts = e.provider === 'claude' || !server ? listClaudeConfigs() : getCodexAccountsForServer(server);
      accounts
        .then((list) => {
          if (accSeq !== accountSeq) return;
          if (e.provider === 'claude') configs = list as ConfigDirInfo[];
          else codexAccounts = list as CodexAccount[];
        })
        .catch(() => { /* sem lista: sem pílula, retomar segue na conta de origem */ })
        .finally(() => { if (accSeq === accountSeq) accountsLoading = false; });
    } else accountsLoading = false;
    const seq = ++motorSeq;
    // Best-effort: sem isto o seletor de motor nao aparece, mas retomar continua funcionando.
    getEngines()
      .then((r) => { if (seq === motorSeq) motores = r.motores; })
      .catch(() => { if (seq === motorSeq) motores = {}; });
    try {
      const history = await getArchiveHistory(e.project, e.session_id, undefined, e.config_dir, e.provider, e.codex_account, server);
      if (seq !== motorSeq || selected?.session_id !== e.session_id || activeServerId !== server?.id) return;
      events = history;
    } catch {
      if (seq !== motorSeq || selected?.session_id !== e.session_id || activeServerId !== server?.id) return;
      if (!foldersLoaded) void load();   // zera `error`: o aviso vem depois
      error = m.arquivo_conversa_erro();
      selected = null;
    } finally {
      if (seq === motorSeq) loadingChat = false;
    }
  }

  // Enviar numa conversa fechada = retomar (claude --resume na conta dela) e entregar o texto na
  // sessao nova. O servidor do deep-link e reaplicado ANTES de trocar de tela, pro chat abrir no
  // servidor DONO da conversa.
  async function sendAndResume() {
    const text = draft.trim();
    if (!selected || !text || resuming || accountsLoading || blockedNow) return;
    const entry = selected, server = servers.find((s) => s.id === activeServerId);
    if (!server) return;
    resuming = true;
    resumeError = '';
    let name = '';
    try {
      const info = await resumeArchivedConversation(entry.project, entry.session_id,
                                                    engine || null, pickedConfig ?? entry.config_dir,
                                                    entry.provider, entry.codex_account, server);
      name = info.name;
      await sendInputForServer(server, name, text);
      draft = '';
    } catch (e) {
      if (!name) {
        if (selected !== entry || activeServerId !== server.id) return;
        // Conversa já aberta noutra sessão: o texto vai pro rascunho dela e o chat abre lá.
        const err = e as { status?: number; code?: string; envelope?: { sessao?: unknown; params?: { sessao?: unknown } } };
        const live = err.envelope?.sessao ?? err.envelope?.params?.sessao;
        if (err.status === 409 && err.code === 'erro_conversa_viva' && typeof live === 'string' && live) {
          mergeDraft(server.id, live, text);
          if (deepLink) selectServer(deepLink.serverId);
          window.location.hash = `#/chat/${encodeURIComponent(server.id)}/${encodeURIComponent(live)}`;
          return;
        }
        if (err.status === 409 && err.code === 'erro_conversa_viva') resumeError = m.conversa_ja_aberta_sem_nome();
        else resumeError = e instanceof Error ? e.message : m.arquivo_retomar_erro();
        return;
      }
      console.error('archive: resume ok, send failed', name, e);
      // A sessao ja existe: o texto volta ao campo do chat dela pelo rascunho que o Chat ja restaura.
      try { localStorage.setItem(draftStorageKey(server.id, name), JSON.stringify({ text, jsonl: null })); } catch { /* sem storage */ }
    } finally {
      resuming = false;
    }
    if (!name) return;
    if (deepLink) selectServer(deepLink.serverId);
    window.location.hash = `#/chat/${encodeURIComponent(server.id)}/${encodeURIComponent(name)}`;
  }

  // Conversa aberta por link: "voltar" sai da tela (quem monta decide o destino); pela navegacao de
  // pastas, volta a lista da pasta.
  function backFromConversation() {
    if (deepLink) onBack();
    else {
      selected = null;
      if (!foldersLoaded) void load();
    }
  }

  // Rascunho do chat dela: junta ao que já estava lá em vez de sobrescrever. Mesma forma que o Chat lê.
  function mergeDraft(serverId: string, session: string, text: string) {
    const key = draftStorageKey(serverId, session);
    try {
      const { text: prev, jsonl } = parseStoredDraft(readMigrating(key, `cp-draft:${session}`));
      localStorage.setItem(key, JSON.stringify({ text: prev ? `${prev}\n\n${text}` : text, jsonl }));
    } catch { /* sem storage */ }
  }

  // Nome curto da pasta (ultimo segmento do cwd real; fallback: nome sanitizado do projeto).
  function folderName(f: ArchiveFolder): string {
    if (f.cwd) return basename(f.cwd);
    return f.project;
  }

  function fmtDate(ts: number): string {
    return new Date(ts * 1000).toLocaleString(intlLocale(), {
      day: '2-digit', month: '2-digit', hour: '2-digit', minute: '2-digit',
    });
  }
  const noop = () => {};
</script>

{#if selected}
  {@const sel = selected}
  <div class="archive-screen" style="--nav-h: 0px" use:keyboardInset>
    <!-- `preview` (1a msg) só existe no Claude; fora dele o titulo cairia no slice do id e virava
         "session_" pra TODA conversa do Kimi. A ultima msg identifica melhor de qualquer forma. -->
    <NavBar title={sel.preview || sel.ultima || sel.session_id.slice(0, 8)} showBack={true} onBack={backFromConversation} />
    {#if loadingChat}
      <p class="muted">{m.arquivo_carregando()}</p>
    {:else}
      <MessageList
        {events}
        stateEvent={null}
        pending={[]}
        sessionName={''}
        dockH={8}
        onSelectOption={noop}
        onCancel={noop}
        imageUrl={(id, idx) => archiveImageUrl(sel.project, sel.session_id, id, idx)}
        focoId={deepLink?.sessionId === sel.session_id ? (deepLink.eventId ?? null) : null}
      />
    {/if}
    <div class="resume-bar">
      <!-- Motor é do Claude: mandá-lo num resume de Pi/Kimi faria o hangar-engine exportar chave de
           outro provedor pra um CLI que nem lê essas variáveis. -->
      {#if sel.provider === 'claude' && Object.keys(motores).length}
        {#if showEngine}
          <label class="engine-pick">
            <span class="engine-pick-label">{m.comum_motor()}</span>
            <Select ariaLabel={m.comum_motor()} value={engine}
              opcoes={[{ value: '', label: m.criar_claude_sua_conta() },
                       ...Object.entries(motores).map(([nome, motor]) => ({
                         value: nome, label: motor.label ?? nome, hint: motor.model }))]}
              onchange={(v) => (engine = v)} />
          </label>
          <!-- O app nao sabe qual motor rodava esta conversa (o pane original morreu, sem /proc pra
               ler) -- so o nome do modelo fica gravado no transcript. Escolha e sua, nao memoria. -->
          <p class="engine-pick-hint">{m.arquivo_motor_escolha()}</p>
        {/if}
        <button class="engine-opt" type="button" aria-expanded={showEngine}
          onclick={() => (showEngine = !showEngine)}>
          {m.comum_motor()}: {engine ? (motores[engine]?.label ?? engine) : m.criar_claude_sua_conta()}
        </button>
      {/if}
      <NewChatComposer bind:value={draft} placeholder={m.conversa_continuar_placeholder()}
        busy={resuming} blocked={accountsLoading || !!blockedNow}
        note={blockedNow ? { text: blockedNow, warning: true } : resumeError ? { text: resumeError, warning: true } : null}
        onsend={sendAndResume}>
        {#snippet below()}
          <div class="account-row">
            <AccountPill server={activeServerId ?? ''} provider={sel.provider} {configs} {codexAccounts}
              selected={selectedAccount} loading={accountsLoading}
              disabled={resuming || sel.provider === 'codex'} blockExhausted bind:blocked={accountBlocked}
              onchange={(id) => (pickedConfig = id)} />
          </div>
        {/snippet}
      </NewChatComposer>
    </div>
  </div>
{:else if folder}
  {@const f = folder}
  <div class="archive-screen">
    <NavBar title={folderName(f)} showBack={true} onBack={() => (folder = null)} />
    <div class="archive-list">
      {#if f.cwd}<div class="group-label">{f.cwd}</div>{/if}
      {#if loadingEntries}
        <p class="muted">{m.comum_carregando()}</p>
      {:else if entries.length === 0}
        <p class="muted">{m.arquivo_vazio_pasta()}</p>
      {:else}
        {#each entries as e (JSON.stringify([e.provider, e.codex_account ?? e.config_dir, e.session_id]))}
          <button class="row" onclick={() => openConversation(e)}>
            <span class="row-main">
              <!-- A ULTIMA msg identifica a conversa; a 1a nao (todas comecam parecidas). -->
              <span class="row-preview">{e.ultima || e.preview || m.arquivo_sem_mensagens()}</span>
              <span class="row-meta">
                <!-- O separador vai DENTRO da expressao: um " · " solto no template perde o
                     espaco da frente no build e vira "10:11· Conta". -->
                {fmtDate(e.mtime)}{#if e.conta}{` · ${e.conta}`}{/if}{#if e.live}<b class="live">{m.arquivo_ativa()}</b>{/if}
              </span>
              {#if e.provider && e.provider !== 'claude'}
                <!-- Só fora do Claude: ele é a maioria das linhas, e um selo em todas seria ruído. -->
                <span class="prov" title={providerName(e.provider)}>
                  <ProviderGlyph provider={e.provider} size={11} />{providerName(e.provider)}
                </span>
              {/if}
            </span>
            <span class="chev" aria-hidden="true">›</span>
          </button>
        {/each}
      {/if}
    </div>
  </div>
{:else}
  <div class="archive-screen">
    <NavBar title={m.nav_arquivo()} showBack={true} onBack={onBack} />
    {#if servers.length >= 2}
      <!-- Seletor: de qual servidor navegar o arquivo. So aparece com 2+ servidores. -->
      <div class="srv-picker" role="tablist" aria-label={m.arquivo_servidor_aria()}>
        {#each servers as s (s.id)}
          <button
            class="srv-pill" class:on={s.id === activeServerId}
            role="tab" aria-selected={s.id === activeServerId}
            onclick={() => pickServer(s.id)}
          >
            <span class="srv-dot" style="background: {serverColor(s.id)};" aria-hidden="true"></span>
            {s.label}
          </button>
        {/each}
      </div>
    {/if}
    <div class="archive-list">
      {#if loading}
        <p class="muted">{m.comum_carregando()}</p>
      {:else if error}
        <p class="err">{error}</p>
      {:else if folders.length === 0}
        <p class="muted">{m.arquivo_vazio_geral()}</p>
      {:else}
        {#each folders as f (f.project)}
          <button class="row" onclick={() => openFolder(f)}>
            <span class="row-icon" aria-hidden="true">📁</span>
            <span class="row-main">
              <span class="row-preview">{folderName(f)}</span>
              <span class="row-meta">{f.count === 1 ? m.arquivo_conversa_1() : m.arquivo_conversas({ n: f.count })} · {fmtDate(f.mtime)}</span>
            </span>
            <span class="chev" aria-hidden="true">›</span>
          </button>
        {/each}
      {/if}
    </div>
  </div>
{/if}

<style>
  .archive-screen {
    display: flex;
    flex-direction: column;
    height: 100%;
    min-height: 0;
    position: relative;
    background: var(--bg-base);
  }

  .archive-list {
    flex: 1;
    overflow-y: auto;
    /* topo = --navbar-fade: o glass da navbar pinta por cima desse tanto de conteúdo. */
    padding: var(--navbar-fade) var(--space-4) var(--space-8);
    max-width: 700px;
    width: 100%;
    margin: 0 auto;
  }

  /* Seletor de servidor (multi-servidor): de qual servidor navegar o arquivo. */
  .srv-picker {
    display: flex;
    gap: var(--space-2);
    padding: var(--space-2) var(--space-4) 0;
    max-width: 700px;
    width: 100%;
    margin: 0 auto;
    overflow-x: auto;
  }
  .srv-pill {
    display: inline-flex;
    align-items: center;
    gap: 6px;
    flex-shrink: 0;
    padding: 5px 12px;
    border-radius: var(--radius-full);
    font-size: var(--text-xs);
    font-weight: 600;
    color: var(--text-secondary);
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    transition: color 160ms var(--ease-out), background 160ms var(--ease-out), border-color 160ms var(--ease-out);
  }
  .srv-pill.on {
    color: var(--text-primary);
    background: var(--bg-hover);
    border-color: var(--border-default);
  }
  .srv-dot { width: 8px; height: 8px; border-radius: 50%; flex-shrink: 0; }

  .muted { color: var(--text-secondary); padding: var(--space-4); }
  .err { color: var(--error); padding: var(--space-4); }

  .group-label {
    font-size: var(--text-xs);
    font-family: var(--font-mono);
    color: var(--text-muted);
    padding: var(--space-2) var(--space-1);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  .row {
    width: 100%;
    display: flex;
    align-items: center;
    gap: var(--space-3);
    text-align: left;
    background: var(--bg-surface);
    border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md);
    padding: var(--space-3) var(--space-4);
    margin-bottom: var(--space-2);
  }
  .row:active { background: var(--bg-hover); }

  .row-icon { flex-shrink: 0; }

  .row-main {
    flex: 1;
    min-width: 0;
    display: flex;
    flex-direction: column;
    gap: 2px;
  }
  .row-preview {
    font-size: var(--text-sm);
    color: var(--text-primary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .row-meta { font-size: var(--text-xs); color: var(--text-muted); }
  .live { color: var(--success); }
  .prov {
    display: inline-flex;
    align-items: center;
    gap: 4px;
    align-self: flex-start;
    margin-top: 2px;
    padding: 1px 6px;
    border-radius: var(--radius-sm);
    background: var(--surface-raised);
    font-size: var(--text-xs);
    color: var(--text-secondary);
  }
  .chev { color: var(--text-muted); flex-shrink: 0; }

  .resume-bar {
    padding: var(--space-3) var(--space-4) calc(var(--space-3) + env(safe-area-inset-bottom));
    max-width: 700px;
    width: 100%;
    margin: 0 auto;
  }
  .engine-opt {
    font-size: var(--text-xs);
    color: var(--text-secondary);
    padding: var(--space-1) 0;
    margin-bottom: var(--space-2);
  }
  .account-row { display: flex; align-items: center; min-height: 28px; }

  .engine-pick {
    display: flex;
    flex-direction: column;
    gap: var(--space-2);
    margin-bottom: var(--space-3);
  }
  .engine-pick-label {
    font-size: var(--text-sm);
    color: var(--text-secondary);
    font-weight: 500;
  }
  /* :global: o campo é o <button> do Select.svelte. Mantém os 44px de alvo de toque desta tela (o
     padrão do componente é 40) e a fonte de UI em vez da mono. */
  .engine-pick :global(.sel-campo) {
    height: 44px;
    background: var(--bg-surface);
    border-radius: var(--radius-md);
    font-family: var(--font-ui);
  }
  .engine-pick-hint {
    font-size: var(--text-xs);
    color: var(--text-muted);
    margin: 0 0 var(--space-3);
  }
</style>
