<script lang="ts">
  import { onDestroy, untrack } from 'svelte';
  import { Aquecendo, fetchCostsForServer, type CostReport, type DimBucket, type Server } from '@hangar/core';
  import { dec, tok, money2 } from '../lib/fmt';
  import { moeda } from '../lib/moeda.svelte';
  import * as m from '../paraglide/messages';

  // Resumo de uso da máquina na tela inicial (porte de `home_usage.rs`). Recolhido por padrão: uma
  // linha com o total, para o compositor continuar visível com o teclado aberto.
  let { server }: { server: Server } = $props();

  type Period = 'all' | '30d' | '7d' | '1d';
  const PERIODS: { key: Period; label: () => string }[] = [
    { key: 'all', label: m.home_usage_all },
    { key: '30d', label: m.home_usage_30d },
    { key: '7d', label: m.home_usage_7d },
    { key: '1d', label: m.home_usage_1d },
  ];
  const WARM_MS = 3000;
  const WARM_TRIES = 100;
  const STALE_MS = 60_000;

  const raw = (b: DimBucket) => b.input + b.output + b.cache_write + b.cache_read;

  let period = $state<Period>('all');
  let tab = $state<'overview' | 'models'>('overview');
  let expanded = $state(false);
  let loading = $state(false);
  let warming = $state<{ read: number; total: number } | null>(null);
  let error = $state('');
  let report = $state<Partial<CostReport> | null>(null);
  let loadedAt = 0;
  let seq = 0;
  let timer: ReturnType<typeof setTimeout> | undefined;

  const totals = $derived(report?.totals);
  const models = $derived(
    (report?.by_model ?? []).filter((b) => raw(b) > 0).sort((a, b) => raw(b) - raw(a) || a.key.localeCompare(b.key)),
  );
  const empty = $derived(!!totals && totals.sessions === 0 && raw(totals) === 0);
  const activeDays = $derived((report?.by_day ?? []).filter((b) => raw(b) > 0).length);
  const partial = $derived((report?.sem_tarifa ?? []).length > 0);
  const maxModel = $derived(Math.max(1, ...models.map(raw)));
  // Semanas de segunda a domingo; a data vem do histórico, sem deslocar fuso.
  const day = (d: number) => new Date(d).toISOString().slice(0, 10);
  const calendar = $derived.by(() => {
    const byDay = new Map<string, number>();
    for (const b of report?.by_day ?? []) byDay.set(b.key, (byDay.get(b.key) ?? 0) + raw(b));
    const keys = [...byDay.keys()].sort();
    if (!keys.length) return null;
    const start = Date.parse(`${keys[0]}T00:00:00Z`);
    const end = Date.parse(`${keys[keys.length - 1]}T00:00:00Z`);
    if (Number.isNaN(start) || Number.isNaN(end)) return null;
    const first = start - ((new Date(start).getUTCDay() + 6) % 7) * 86400000;
    const weeks = Math.floor((end - first) / 86400000 / 7) + 1;
    const max = Math.max(1, ...byDay.values());
    const br = (k: string) => k.split('-').reverse().join('/');
    return {
      title: m.home_usage_activity({ start: br(keys[0]), end: br(keys[keys.length - 1]) }),
      weeks: Array.from({ length: weeks }, (_, w) => Array.from({ length: 7 }, (_, d) => {
        const t = first + (w * 7 + d) * 86400000;
        const k = day(t);
        const v = byDay.get(k) ?? 0;
        return {
          k, v, visible: t >= start && t <= end, level: v > 0 ? 0.3 + 0.7 * (v / max) : 0,
          tip: m.home_usage_day({ date: br(k), tokens: dec(v, 0) }),
        };
      })),
    };
  });
  const cost = (n: number) => money2(n, moeda.cur, report?.usd_brl ?? null);

  function stop() {
    seq++;
    clearTimeout(timer);
  }

  async function load() {
    stop();
    const mine = seq;
    const srv = server;
    const per = period;
    loading = true;
    error = '';
    warming = null;
    for (let tries = 0; ; tries++) {
      try {
        const r = await fetchCostsForServer(srv, per, false, true);
        if (mine !== seq) return;
        loadedAt = Date.now();
        if (r.applied?.period !== per) error = m.home_usage_period_unsupported();
        else if (!r.totals) error = m.home_usage_load_failed();
        else report = r;
        break;
      } catch (e) {
        if (mine !== seq) return;
        if (e instanceof Aquecendo) {
          if (tries >= WARM_TRIES) { loadedAt = Date.now(); error = m.home_usage_warming_timeout(); break; }
          warming = { read: e.lidos, total: e.total };
          await new Promise<void>((ok) => { timer = setTimeout(ok, WARM_MS); });
          if (mine !== seq) return;
          continue;
        }
        loadedAt = Date.now();
        error = m.home_usage_load_failed();
        break;
      }
    }
    warming = null;
    loading = false;
  }

  // Troca de máquina ou de período relê; a leitura anterior não pode mais escrever.
  let lastKey = '';
  $effect(() => {
    const key = `${server.id}|${period}`;
    if (key === lastKey) return;
    lastKey = key;
    report = null;
    untrack(() => void load());
  });

  function onVisible() {
    if (document.visibilityState === 'visible' && !loading && Date.now() - loadedAt >= STALE_MS) void load();
  }
  document.addEventListener('visibilitychange', onVisible);
  onDestroy(() => {
    stop();
    document.removeEventListener('visibilitychange', onVisible);
  });

  const summaryLine = $derived(
    totals && !empty ? `${tok(raw(totals))} · ${cost(totals.cost)}` : loading ? m.native_loading() : '',
  );
</script>

<section class="usage" aria-label={m.home_usage_overview()}>
  <button type="button" class="head" aria-expanded={expanded} onclick={() => (expanded = !expanded)}>
    <span class="lbl">{m.home_usage_tokens()}</span>
    <span class="val">{summaryLine || '—'}</span>
    <svg class="chev" class:open={expanded} width="14" height="14" viewBox="0 0 24 24" fill="none" stroke="currentColor"
      stroke-width="2" stroke-linecap="round" stroke-linejoin="round" aria-hidden="true"><polyline points="6 9 12 15 18 9" /></svg>
  </button>

  {#if expanded}
    <div class="body">
      <div class="bar">
        <div class="seg" role="group" aria-label={m.home_usage_overview()}>
          <button type="button" class:on={tab === 'overview'} aria-pressed={tab === 'overview'}
            onclick={() => (tab = 'overview')}>{m.home_usage_overview()}</button>
          <button type="button" class:on={tab === 'models'} aria-pressed={tab === 'models'}
            onclick={() => (tab = 'models')}>{m.home_usage_models()}</button>
        </div>
        <div class="seg" role="group" aria-label={m.home_usage_all()}>
          {#each PERIODS as p (p.key)}
            <button type="button" class:on={period === p.key} aria-pressed={period === p.key}
              onclick={() => (period = p.key)}>{p.label()}</button>
          {/each}
        </div>
      </div>

      {#if loading}
        <p class="note" role="status">
          {warming ? m.home_usage_warming({ read: String(warming.read), total: String(warming.total) }) : m.native_loading()}
        </p>
      {:else if error}
        <p class="note warn" role="alert">{error}</p>
        <button type="button" class="retry" onclick={() => load()}>{m.sync_retry()}</button>
      {:else if empty || !totals}
        <p class="note">{m.home_usage_empty()}</p>
      {:else if tab === 'models'}
        {#if models.length === 0}
          <p class="note">{m.home_usage_empty()}</p>
        {:else}
          <ul class="models">
            {#each models as b (b.key)}
              <li>
                <div class="mrow"><span class="mname">{b.label ?? b.key}</span><span>{tok(raw(b))}</span></div>
                <div class="track"><div class="fill" style:width="{(raw(b) / maxModel) * 100}%"></div></div>
              </li>
            {/each}
          </ul>
        {/if}
      {:else}
        <dl class="grid">
          <div><dt>{m.home_usage_sessions()}</dt><dd>{dec(totals.sessions, 0)}</dd></div>
          <div><dt>{m.home_usage_tokens()}</dt><dd>{tok(raw(totals))}</dd></div>
          <div><dt>{m.home_usage_cost()}</dt><dd>{cost(totals.cost)}</dd></div>
          <div><dt>{m.home_usage_active_days()}</dt><dd>{activeDays}</dd></div>
          <div><dt>{m.home_usage_model_count()}</dt><dd>{models.length}</dd></div>
          <div><dt>{m.home_usage_top_model()}</dt><dd>{models[0] ? (models[0].label ?? models[0].key) : '—'}</dd></div>
        </dl>
        {#if calendar}
          <p class="note">{calendar.title}</p>
          <div class="cal">
            {#each calendar.weeks as week, wi (wi)}
              <div class="week">
                {#each week as c (c.k)}
                  <span class="cell" class:hidden={!c.visible} title={c.tip} aria-label={c.visible ? c.tip : undefined}
                    aria-hidden={c.visible ? undefined : 'true'}
                    style:background={c.v > 0 ? `color-mix(in srgb, var(--accent) ${Math.round(c.level * 100)}%, transparent)` : undefined}></span>
                {/each}
              </div>
            {/each}
          </div>
        {/if}
      {/if}

      {#if !loading && !error && !empty && totals}
        <p class="note" class:warn={partial}>{partial ? m.home_usage_partial() : m.home_usage_method()}</p>
      {/if}
    </div>
  {/if}
</section>

<style>
  .usage { width: 100%; max-width: 420px; margin: 0 auto var(--space-3); }
  .head {
    width: 100%; min-height: 36px; display: flex; align-items: center; justify-content: center; gap: 8px;
    padding: 4px 10px; border: 0; border-radius: var(--radius-full); background: transparent;
    color: var(--text-secondary); font-size: var(--text-sm); cursor: pointer;
  }
  .head:active { background: var(--fill-subtle); }
  .lbl { color: var(--text-muted); }
  .val { font-variant-numeric: tabular-nums; color: var(--text-primary); }
  .chev { flex: none; transition: transform 0.15s; }
  .chev.open { transform: rotate(180deg); }
  .cal { display: flex; gap: 3px; overflow-x: auto; padding-bottom: 2px; }
  .week { display: flex; flex-direction: column; gap: 3px; }
  .cell { width: 11px; height: 11px; flex: none; border-radius: 2px; background: var(--fill-subtle); }
  .cell.hidden { visibility: hidden; }
  .body {
    min-height: min(260px, 50dvh); max-height: min(50dvh, 340px); overflow-y: auto;
    display: flex; flex-direction: column; gap: var(--space-2); padding: var(--space-3);
    border: 1px solid var(--border-subtle, var(--fill-subtle)); border-radius: var(--radius-lg);
  }
  .bar { display: flex; flex-wrap: wrap; justify-content: space-between; gap: var(--space-2); }
  .seg { display: flex; gap: 2px; }
  .seg button {
    min-height: 32px; padding: 4px 10px; border: 0; border-radius: var(--radius-full); background: transparent;
    color: var(--text-secondary); font-size: var(--text-xs); cursor: pointer;
  }
  .seg button.on { background: var(--accent-dim); color: var(--text-primary); }
  .note { margin: 0; font-size: var(--text-xs); color: var(--text-muted); }
  .note.warn { color: var(--warning, var(--text-secondary)); }
  .retry { align-self: flex-start; border: 0; background: transparent; color: var(--accent); font-size: var(--text-sm); cursor: pointer; min-height: 32px; }
  .grid { margin: 0; display: grid; grid-template-columns: repeat(3, minmax(0, 1fr)); gap: var(--space-2); }
  .grid div { min-width: 0; }
  dt { font-size: var(--text-xs); color: var(--text-muted); }
  dd { margin: 0; font-size: var(--text-sm); font-weight: var(--fw-medium, 500); overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .models { list-style: none; margin: 0; padding: 0; max-height: 200px; overflow-y: auto; display: flex; flex-direction: column; gap: var(--space-2); }
  .mrow { display: flex; justify-content: space-between; gap: 8px; font-size: var(--text-sm); }
  .mname { min-width: 0; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }
  .track { height: 4px; border-radius: var(--radius-full); background: var(--fill-subtle); }
  .fill { height: 100%; border-radius: var(--radius-full); background: var(--accent); }
</style>
