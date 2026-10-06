<script lang="ts">
  // Tela temporária: sai na parte 7, junto com o Python.
  import * as m from '../../paraglide/messages';
  import { getMigrationStatus, fmtBytes, type MigrationStatus, type MigrationProcess } from '@hangar/core';

  const EVERY_MS = 3000;

  let status = $state<MigrationStatus | null>(null);
  let erro = $state('');
  let carregando = $state(true);

  async function load() {
    try {
      status = await getMigrationStatus();
      erro = '';
    } catch (e) {
      erro = e instanceof Error ? e.message : String(e);
    } finally {
      carregando = false;
    }
  }

  $effect(() => {
    load();
    const id = setInterval(load, EVERY_MS);
    return () => clearInterval(id);
  });

  const AREA: Record<string, () => string> = {
    history: m.migration_area_history, list: m.migration_area_list, send: m.migration_area_send,
    session: m.migration_area_session, terminal: m.migration_area_terminal, workspace: m.migration_area_workspace,
    worktrees: m.migration_area_worktrees, costs: m.migration_area_costs, quotas: m.migration_area_quotas,
    codex: m.migration_area_codex, providers: m.migration_area_providers, accounts: m.migration_area_accounts,
    guests: m.migration_area_guests, pairing: m.migration_area_pairing, mcp: m.migration_area_mcp,
    push: m.migration_area_push, update: m.migration_area_update, uploads: m.migration_area_uploads,
    dictation: m.migration_area_dictation, static: m.migration_area_static, other: m.migration_area_other,
  };
  const PRIVATE: Record<string, () => string> = {
    send_headless: m.migration_private_send_headless, send_terminal: m.migration_private_send_terminal,
    terminal_observe: m.migration_private_terminal_observe, workspace_bridge: m.migration_private_workspace_bridge,
    list_bridge: m.migration_private_list_bridge,
  };
  const REASON: Record<string, () => string> = {
    sem_binario: m.migration_reason_sem_binario, desligado: m.migration_reason_desligado,
    reload: m.migration_reason_reload, protocolo: m.migration_reason_protocolo,
    sem_resposta: m.migration_reason_sem_resposta, endereco_privado: m.migration_reason_endereco_privado,
    quedas: m.migration_reason_quedas, erro: m.migration_reason_erro,
  };
  const MODE: Record<string, () => string> = {
    pending: m.migration_mode_pending, rust: m.migration_mode_rust, python: m.migration_mode_python,
  };

  function owner(routes: { rust: boolean }[]): string {
    const rust = routes.filter((r) => r.rust).length;
    return rust === 0 ? m.migration_owner_python() : rust === routes.length ? m.migration_owner_rust() : m.migration_owner_mixed();
  }
  function ownerClass(routes: { rust: boolean }[]): string {
    const rust = routes.filter((r) => r.rust).length;
    return rust === 0 ? 'py' : rust === routes.length ? 'rs' : 'mix';
  }
  const cpu = (p: { cpu_percent: number | null }) => p.cpu_percent === null ? m.migration_cpu_wait() : `${p.cpu_percent.toFixed(1)}%`;
  const py = $derived(status?.python ?? null);
  const procs = $derived.by(() => {
    const p = py?.processes;
    const rows: { label: string; proc: MigrationProcess | { rss_bytes: number; cpu_percent: number | null } | null }[] = [
      { label: m.migration_proc_python(), proc: p?.python ?? null },
      { label: m.migration_proc_rust(), proc: p?.rust ?? null },
    ];
    if (p?.cano) rows.push({ label: m.migration_proc_cano({ n: String(p.cano.count) }), proc: p.cano.count ? p.cano : null });
    return rows;
  });
</script>

<div class="mig">
  <p class="nota">{m.migration_temporary()}</p>

  {#if carregando && !status}
    <p class="nota" role="status">{m.migration_loading()}</p>
  {:else if erro && !status}
    <p class="nota erro" role="alert">{m.migration_error({ erro })}</p>
    <button class="bt" onclick={load}>{m.migration_retry()}</button>
  {:else if !status}
    <p class="nota">{m.migration_empty()}</p>
  {:else}
    {#if erro}<p class="nota erro" role="alert">{m.migration_error({ erro })}</p>{/if}

    <section>
      <h3>{m.migration_port()}</h3>
      <dl>
        <dt>{m.migration_served()}</dt>
        <dd class:rs={status.served_by === 'rust'} class:py={status.served_by === 'python'}>
          {status.served_by === 'rust' ? m.migration_served_rust() : m.migration_served_python()}
        </dd>
        {#if py}
          <dt>{m.migration_mode()}</dt><dd>{MODE[py.mode]?.() ?? py.mode}</dd>
          {#if py.reason}
            <dt>{m.migration_reason()}</dt><dd>{REASON[py.reason]?.() ?? py.reason} <code>{py.reason}</code></dd>
          {/if}
        {/if}
      </dl>
      {#if status.python_error}<p class="nota erro">{m.migration_python_error({ codigo: status.python_error })}</p>{/if}
    </section>

    <section>
      <h3>{m.migration_versions()}</h3>
      <dl>
        <dt>{m.migration_contract()}</dt>
        <dd>{m.migration_contract_value({ rust: status.rust ? String(status.rust.protocol) : '—', python: py ? String(py.protocol) : '—' })}</dd>
        {#if status.rust}
          <dt>{m.migration_binary()}</dt>
          <dd><code>{status.rust.version} · {status.rust.commit ? status.rust.commit.slice(0, 10) : m.migration_binary_local()}</code></dd>
        {/if}
        {#if py}
          {#if py.binary}<dt></dt><dd><code class="path">{py.binary.path}</code></dd>{/if}
          <dt>{m.migration_checkout()}</dt><dd><code>{py.branch ?? '—'} · {py.version}</code></dd>
          <dt>{m.migration_channel()}</dt><dd>{py.update_branch ?? m.migration_channel_none()}</dd>
        {/if}
      </dl>
    </section>

    <section>
      <h3>{m.migration_usage()}</h3>
      <table>
        <thead><tr><th></th><th>{m.migration_memory()}</th><th>{m.migration_cpu()}</th></tr></thead>
        <tbody>
          {#each procs as row (row.label)}
            <tr>
              <th scope="row">{row.label}</th>
              {#if row.proc}
                <td>{fmtBytes(row.proc.rss_bytes)}</td><td>{cpu(row.proc)}</td>
              {:else}
                <td colspan="2" class="nota">{m.migration_proc_none()}</td>
              {/if}
            </tr>
          {/each}
        </tbody>
      </table>
    </section>

    <section>
      <h3>{m.migration_areas()}</h3>
      {#if status.areas}
        <p class="nota">{m.migration_window({ minutos: String(status.window_minutes ?? 10) })}</p>
        <table>
          <tbody>
            {#each status.areas as a (a.key)}
              <tr>
                <th scope="row">
                  <details>
                    <summary>{AREA[a.key]?.() ?? a.key}</summary>
                    <ul>{#each a.routes as r (r.method + r.path)}<li class:rs={r.rust}><code>{r.method} {r.path}</code></li>{/each}</ul>
                  </details>
                </th>
                <td class={ownerClass(a.routes)}>{owner(a.routes)}</td>
                <td class="num">{a.rust} × {a.python}</td>
              </tr>
            {/each}
          </tbody>
        </table>
        {#if status.private}
          <h4>{m.migration_private()}</h4>
          <table>
            <tbody>
              {#each status.private as p (p.key)}
                <tr><th scope="row">{PRIVATE[p.key]?.() ?? p.key}</th><td class="num">{p.rust}</td></tr>
              {/each}
            </tbody>
          </table>
        {/if}
      {:else}
        <p class="nota">{m.migration_all_python()}</p>
      {/if}
    </section>
  {/if}
</div>

<style>
  .mig { display: flex; flex-direction: column; gap: var(--space-4); padding: var(--space-4); container-type: inline-size; }
  section { display: flex; flex-direction: column; gap: var(--space-2); }
  h3 { margin: 0; font-size: var(--text-sm); font-weight: 600; color: var(--text-primary); }
  h4 { margin: var(--space-2) 0 0; font-size: var(--text-xs); font-weight: 600; color: var(--text-secondary); }
  dl { display: grid; grid-template-columns: max-content 1fr; gap: var(--space-1) var(--space-3); margin: 0; font-size: var(--text-sm); }
  dt { color: var(--text-muted); }
  dd { margin: 0; color: var(--text-primary); min-width: 0; overflow-wrap: anywhere; }
  code { font-family: var(--font-mono); font-size: var(--text-xs); color: var(--text-secondary); }
  .path { word-break: break-all; }
  table { width: 100%; border-collapse: collapse; font-size: var(--text-sm); }
  th, td { text-align: left; padding: var(--space-1) var(--space-2); border-bottom: 1px solid var(--border-subtle); vertical-align: top; }
  thead th { color: var(--text-muted); font-weight: 500; font-size: var(--text-xs); }
  tbody th { font-weight: 400; color: var(--text-primary); }
  .num { font-family: var(--font-mono); font-size: var(--text-xs); white-space: nowrap; text-align: right; }
  .rs { color: var(--success, #5fb878); }
  .py { color: var(--text-secondary); }
  .mix { color: var(--warning, #d9a441); }
  summary { cursor: pointer; }
  ul { margin: var(--space-1) 0 0; padding-left: var(--space-4); }
  li { color: var(--text-muted); }
  li.rs code { color: var(--success, #5fb878); }
  .nota { margin: 0; font-size: var(--text-xs); color: var(--text-muted); }
  .nota.erro { color: var(--erro, #d97070); }
  .bt { align-self: flex-start; border-radius: var(--radius-md); padding: var(--space-2) var(--space-4); font-family: inherit;
        font-size: var(--text-sm); border: 1px solid var(--border-subtle); background: var(--surface-raised); color: var(--text-secondary); }
</style>
