<script lang="ts">
  // Exportar e importar os atalhos. Quem tira as credenciais é o backend (/api/shortcuts/export):
  // o arquivo sai com `⟦SEGREDO:<nome>⟧` no lugar de cada senha, e a importação pede o valor de
  // cada marcador antes de gravar. Marcador em branco fica: o atalho é salvo, mas não roda.
  import { onDestroy } from 'svelte';
  import { exportShortcuts, importShortcuts, type Shortcut, type ShortcutExport, type ShortcutImportResult } from '@hangar/core';
  import ModalDialog from './ModalDialog.svelte';
  import { reloadShortcuts } from '../lib/shortcuts.svelte';
  import { listServers, getActiveId, type Server } from '../lib/auth';
  import * as m from '../paraglide/messages';

  interface Props {
    serverId?: string | null;
    // Compacto: um "⋯" com menu (cabeçalho da seção Ações). Senão, dois botões (tela de config).
    compact?: boolean;
    onDone?: () => void;
  }
  let { serverId = null, compact = false, onDone }: Props = $props();

  let menuOpen = $state(false);
  let menuEl: HTMLElement | undefined = $state();

  function closeMenuOutside(e: PointerEvent) {
    if (menuOpen && menuEl && !menuEl.contains(e.target as Node)) menuOpen = false;
  }
  let input = $state<HTMLInputElement | null>(null);
  let note = $state<{ text: string; error: boolean } | null>(null);
  let noteTimer: ReturnType<typeof setTimeout> | undefined;
  let pending = $state<{ data: unknown; preview: ShortcutImportResult; target: Server } | null>(null);
  let secrets = $state<Record<string, Record<string, string>>>({});
  let applying = $state(false);
  let importError = $state('');
  let exportOpen = $state(false);
  let exportLoading = $state(false);
  let exportBusy = $state(false);
  let exportError = $state('');
  let exportSource = $state<Server | null>(null);
  let choices = $state<ShortcutExport | null>(null);
  let selected = $state<string[]>([]);
  let warnings = $state<string[]>([]);
  let generation = 0;
  const missingScriptSecrets = $derived(pending?.preview.placeholders.some((p) =>
    p.id.startsWith('script:') && p.names.some((name) => !secrets[p.id]?.[name])) ?? false);

  onDestroy(() => { generation++; clearTimeout(noteTimer); });

  function server(): Server {
    const target = listServers().find((s) => s.id === (serverId ?? getActiveId()));
    if (!target) throw new Error(m.compare_servidor_nao_encontrado());
    return target;
  }

  function show(text: string, error = false) {
    clearTimeout(noteTimer);
    note = { text, error };
    noteTimer = setTimeout(() => (note = null), 8000);
  }

  function msg(e: unknown) { return e instanceof Error ? e.message : String(e); }

  function closeExport() { generation++; exportOpen = false; exportBusy = false; }

  function shortcutLabel(item: Shortcut): string {
    if (item.type !== 'internal') return item.label;
    return { terminal: m.ctx_terminal, modo: m.atalhos_interno_modo, navegador: m.ctx_navegador,
      anexos: m.ctx_anexos, rodar: m.ctx_rodar, externo: m.native_shortcuts_native_externo }[item.action]();
  }

  function fileStatus(status: 'create' | 'replace' | 'same'): string {
    return status === 'replace' ? m.shortcut_file_replace() : status === 'same' ? m.shortcut_file_same() : m.shortcut_file_create();
  }

  async function chooseExport() {
    menuOpen = false;
    const mine = ++generation;
    exportOpen = true;
    exportLoading = true;
    exportError = '';
    choices = null;
    exportSource = null;
    selected = [];
    warnings = [];
    try {
      exportSource = server();
      const result = await exportShortcuts(exportSource, { includeScripts: false });
      if (result.version !== 2) throw new Error(m.shortcut_transfer_update_required());
      if (mine === generation) choices = result;
    } catch (e) {
      if (mine === generation) exportError = msg(e);
    } finally {
      if (mine === generation) exportLoading = false;
    }
  }

  async function doExport() {
    if (!exportSource || !selected.length || exportBusy) return;
    const mine = generation;
    exportBusy = true;
    exportError = '';
    try {
      const r = await exportShortcuts(exportSource, { ids: [...selected], includeScripts: true });
      if (mine !== generation) return;
      if (r.version !== 2 || !Array.isArray(r.scripts)) throw new Error(m.shortcut_transfer_update_required());
      const blob = new Blob([JSON.stringify({ version: r.version, shortcuts: r.shortcuts,
        scripts: r.scripts ?? [], warnings: r.warnings ?? [] }, null, 2) + '\n'],
                            { type: 'application/json' });
      const a = document.createElement('a');
      a.href = URL.createObjectURL(blob);
      a.download = 'hangar-atalhos.json';
      a.click();
      setTimeout(() => URL.revokeObjectURL(a.href), 1000);
      warnings = r.warnings ?? [];
      exportOpen = false;
      show(r.removed ? m.atalhos_exportado_segredos({ n: r.removed }) : m.atalhos_exportado());
    } catch (e) {
      if (mine === generation) exportError = msg(e);
    } finally {
      if (mine === generation) exportBusy = false;
    }
  }

  function pickFile() {
    menuOpen = false;
    input?.click();
  }

  async function onFile(e: Event) {
    const el = e.currentTarget as HTMLInputElement;
    const file = el.files?.[0];
    el.value = '';
    if (!file) return;
    const mine = ++generation;
    importError = '';
    let data: unknown;
    let target: Server;
    try {
      target = server();
      if (file.size > 4 * 1024 * 1024) throw new Error(m.shortcut_import_too_large());
      data = JSON.parse(await file.text());
    } catch (err) {
      show(m.atalhos_importar_invalido({ msg: msg(err) }), true);
      return;
    }
    try {
      const preview = await importShortcuts({ data }, target);
      if (mine !== generation) return;
      if (data && typeof data === 'object' && 'version' in data && data.version === 2 && !Array.isArray(preview.files)) {
        throw new Error(m.shortcut_transfer_update_required());
      }
      secrets = Object.fromEntries(preview.placeholders.map((p) => [p.id, Object.fromEntries(p.names.map((n) => [n, '']))]));
      pending = { data, preview, target };
    } catch (err) {
      show(m.atalhos_importar_invalido({ msg: msg(err) }), true);
    }
  }

  async function apply() {
    if (!pending || applying) return;
    const draft = pending;
    applying = true;
    importError = '';
    try {
      await importShortcuts({ data: draft.data, apply: true, secrets }, draft.target);
      pending = null;
      secrets = {};
      try { await reloadShortcuts(draft.target.id); }
      catch (err) { show(m.shortcut_import_reload_failed({ error: msg(err) }), true); onDone?.(); return; }
      show(m.atalhos_importado());
      onDone?.();
    } catch (err) {
      importError = msg(err);
    } finally {
      applying = false;
    }
  }
</script>

<input bind:this={input} type="file" accept="application/json,.json" class="sr-only" onchange={onFile} tabindex="-1" aria-hidden="true" />

<!-- Captura: o BottomSheet engole o Esc na bolha; aqui ele fecha só o menu, não a sheet. -->
<svelte:window onpointerdown={closeMenuOutside} onkeydowncapture={(e) => {
  if (menuOpen && e.key === 'Escape') { e.stopImmediatePropagation(); e.preventDefault(); menuOpen = false; }
}} />

{#if compact}
  <span class="tr-menu" bind:this={menuEl}>
    <button type="button" class="tr-mais" onclick={() => (menuOpen = !menuOpen)} aria-haspopup="menu"
            aria-expanded={menuOpen} aria-label={m.atalhos_transferir()} title={m.atalhos_transferir()}>⋯</button>
    {#if menuOpen}
      <span class="tr-lista" role="menu">
        <button type="button" role="menuitem" onclick={pickFile}>{m.atalhos_importar()}</button>
        <button type="button" role="menuitem" onclick={() => void chooseExport()}>{m.atalhos_exportar()}</button>
        <span class="tr-escopo">{m.atalhos_transferir_so_globais()}</span>
      </span>
    {/if}
  </span>
{:else}
  <button type="button" class="btn" onclick={pickFile}>{m.atalhos_importar()}</button>
  <button type="button" class="btn" onclick={() => void chooseExport()}>{m.atalhos_exportar()}</button>
  <span class="tr-escopo">{m.atalhos_transferir_so_globais()}</span>
{/if}

{#if note}
  <span class="tr-nota" class:flutua={compact} class:erro={note.error} role={note.error ? 'alert' : 'status'}>{note.text}</span>
{/if}

{#if warnings.length}
  <details class="tr-warnings" open>
    <summary>{m.shortcut_bundle_warnings()}</summary>
    <ul>{#each warnings as warning, index (index)}<li>{warning}</li>{/each}</ul>
  </details>
{/if}

<ModalDialog open={exportOpen} ariaLabel={m.shortcut_export_title()} onClose={closeExport}>
  <div class="tr-dialogo">
    <h2>{m.shortcut_export_title()}</h2>
    <p>{m.shortcut_export_help()}</p>
    {#if exportSource}<p>{m.shortcut_source_server({ nome: exportSource.label })}</p>{/if}
    {#if exportLoading}
      <p role="status">{m.comum_carregando()}</p>
    {:else if choices?.shortcuts.length}
      <div class="tr-selection-actions">
        <button class="btn" type="button" disabled={exportBusy} onclick={() => (selected = choices!.shortcuts.map((item) => item.id))}>{m.shortcut_select_all()}</button>
        <button class="btn" type="button" disabled={exportBusy} onclick={() => (selected = [])}>{m.shortcut_select_none()}</button>
      </div>
      <div class="tr-options">
        {#each choices.shortcuts as item (item.id)}
          <label class="tr-option">
            <input type="checkbox" checked={selected.includes(item.id)} disabled={exportBusy}
              onchange={(e) => { selected = e.currentTarget.checked ? [...selected, item.id] : selected.filter((id) => id !== item.id); }} />
            <span>{shortcutLabel(item)}</span>
          </label>
        {/each}
      </div>
      <p class="tr-ajuda">{m.shortcut_bundle_help()}</p>
    {:else if !exportError}
      <p>{m.shortcut_export_empty()}</p>
    {/if}
    {#if exportError}<p class="tr-error" role="alert">{exportError}</p>{/if}
    <div class="tr-acoes">
      <button class="btn" type="button" onclick={closeExport}>{m.comum_cancelar()}</button>
      <button class="btn primario" type="button" onclick={() => void doExport()} disabled={!selected.length || exportBusy || exportLoading} aria-busy={exportBusy}>
        {m.shortcut_export_selected({ n: selected.length })}
      </button>
    </div>
  </div>
</ModalDialog>

<ModalDialog open={pending !== null} ariaLabel={m.atalhos_importar_titulo()} onClose={() => { if (!applying) pending = null; }}>
  {#if pending}
    <div class="tr-dialogo">
      <h2>{m.atalhos_importar_titulo()}</h2>
      <p>{m.shortcut_source_server({ nome: pending.target.label })}</p>
      <p>{m.atalhos_importar_resumo({ novos: pending.preview.added, substituidos: pending.preview.replaced })}</p>
      {#if pending.preview.files?.length}
        <h3>{m.shortcut_import_files()}</h3>
        {#each pending.preview.files as file (file.path)}
          <details class="tr-file">
            <summary><span>{file.path}</span> · {fileStatus(file.status)}</summary>
            <pre>{file.content}</pre>
          </details>
        {/each}
      {/if}
      {#if pending.preview.warnings?.length}
        <p>{m.shortcut_bundle_warnings()}</p>
        <ul>{#each pending.preview.warnings as warning, index (index)}<li>{warning}</li>{/each}</ul>
      {/if}
      {#if pending.preview.placeholders.length}
        <p class="tr-ajuda">{pending.preview.placeholders.some((p) => p.id.startsWith('script:'))
          ? m.shortcut_script_secrets_required() : m.atalhos_importar_segredos()}</p>
        {#each pending.preview.placeholders as p (p.id)}
          {#each p.names as name (name)}
            <label class="tr-campo">
              <span>{m.atalhos_segredo_campo({ atalho: p.label, nome: name })}</span>
              <input type="password" autocomplete="off" disabled={applying} bind:value={secrets[p.id][name]} />
            </label>
          {/each}
        {/each}
      {/if}
      {#if importError}<p class="tr-error" role="alert">{importError}</p>{/if}
      <div class="tr-acoes">
        <button type="button" class="btn" onclick={() => (pending = null)} disabled={applying}>{m.comum_cancelar()}</button>
        <button type="button" class="btn primario" onclick={() => void apply()} disabled={applying || missingScriptSecrets}>{m.atalhos_importar()}</button>
      </div>
    </div>
  {/if}
</ModalDialog>

<style>
  .sr-only { position: absolute; width: 1px; height: 1px; overflow: hidden; clip: rect(0 0 0 0); white-space: nowrap; }
  .tr-menu { position: relative; display: inline-flex; }
  .tr-mais {
    width: 24px; height: 24px; border: 0; border-radius: var(--radius-sm); background: transparent;
    color: var(--text-muted); cursor: pointer; font-size: 14px; line-height: 1;
  }
  .tr-mais:hover { background: var(--surface-raised); color: var(--text-primary); }
  .tr-lista {
    position: absolute; top: calc(100% + 4px); right: 0; z-index: 20; min-width: 140px;
    display: flex; flex-direction: column; padding: 4px; border: 1px solid var(--border-subtle);
    border-radius: var(--radius-md); background: var(--surface-raised); box-shadow: 0 4px 16px rgba(0, 0, 0, 0.24);
  }
  .tr-lista button {
    text-align: left; padding: 6px 10px; border: 0; border-radius: var(--radius-sm); background: transparent;
    color: var(--text-primary); font-size: var(--text-sm); cursor: pointer;
  }
  .tr-lista button:hover { background: var(--bg-hover); }
  .tr-escopo { font-size: var(--text-xs); color: var(--text-muted); }
  .tr-lista .tr-escopo { padding: 4px 10px 2px; border-top: 1px solid var(--border-subtle); margin-top: 2px; }
  .tr-nota { font-size: var(--text-xs); color: var(--text-muted); }
  /* No cabeçalho da seção não há linha sobrando: o aviso flutua embaixo do "⋯". */
  .tr-nota.flutua {
    position: absolute; right: 0; top: 28px; z-index: 20; width: min(260px, 80vw); padding: 6px 10px;
    border: 1px solid var(--border-subtle); border-radius: var(--radius-md); background: var(--surface-raised);
  }
  .btn {
    padding: 6px 12px; border: 1px solid var(--border-subtle); border-radius: var(--radius-sm);
    background: transparent; color: var(--text-primary); font-size: var(--text-sm); cursor: pointer;
  }
  .btn:hover { background: var(--bg-hover); }
  .btn:disabled { opacity: 0.45; }
  .btn.primario { background: var(--accent); border-color: var(--accent); color: var(--bg-base); }
  .tr-nota.erro { color: var(--error); }
  .tr-dialogo { display: flex; flex-direction: column; gap: var(--space-3); padding: var(--space-4); max-width: 480px; }
  .tr-dialogo h2 { margin: 0; font-size: var(--text-lg); }
  .tr-dialogo p { margin: 0; color: var(--text-secondary); font-size: var(--text-sm); }
  .tr-ajuda { color: var(--text-muted); }
  .tr-campo { display: flex; flex-direction: column; gap: 4px; font-size: var(--text-sm); }
  .tr-campo input {
    padding: 6px 8px; border: 1px solid var(--border-subtle); border-radius: var(--radius-sm);
    background: var(--surface-inset); color: var(--text-primary);
  }
  .tr-acoes { display: flex; justify-content: flex-end; gap: var(--space-2); }
  .tr-dialogo h3 { margin: 0; font-size: var(--text-sm); }
  .tr-options { display: flex; flex-direction: column; gap: var(--space-2); max-height: 35vh; overflow: auto; position: relative; }
  .tr-option { display: flex; align-items: center; gap: var(--space-2); padding: var(--space-2) 0; overflow-wrap: anywhere; }
  .tr-selection-actions { display: flex; flex-wrap: wrap; gap: var(--space-2); }
  .tr-file summary { cursor: pointer; overflow-wrap: anywhere; font-size: var(--text-sm); }
  .tr-file pre { position: relative; max-height: 200px; overflow: auto; white-space: pre-wrap; overflow-wrap: anywhere; font-size: var(--text-xs); }
  .tr-dialogo ul { margin: 0; padding-left: var(--space-4); color: var(--text-secondary); font-size: var(--text-sm); }
  .tr-error { color: var(--error) !important; }
  .tr-warnings { font-size: var(--text-xs); max-width: 100%; overflow-wrap: anywhere; }
</style>
