<script lang="ts">
  // Diff no desenho do terminal do Claude Code: uma numeração (a do arquivo, quando o resultado
  // trouxe o patch), a linha inteira com fundo e só o pedaço trocado mais forte.
  import * as m from '../paraglide/messages';
  import type { NumberedDiff, NumberedLine } from '@hangar/core';
  import { highlightCodeLines, type DiffToken } from '../lib/highlightLazy';
  import { betweenHunks, piecesOf } from '../lib/terminalDiff';

  interface Props { diff: NumberedDiff; path: string; limit?: number | null }
  let { diff, path, limit = null }: Props = $props();

  const lines = $derived(diff.hunks.flat());
  // Trecho de cada linha: a quebra entre trechos aparece onde ele muda, e só as linhas contam no corte.
  const hunkOf = $derived(diff.hunks.flatMap((h, n) => h.map(() => n)));
  let all = $state(false);
  const shown = $derived(limit !== null && !all ? lines.slice(0, limit) : lines);
  const hidden = $derived(lines.length - shown.length);
  const numOf = (l: NumberedLine) => l.newNum ?? l.oldNum ?? 0;
  const width = $derived(String(lines.reduce((mx, l) => Math.max(mx, numOf(l)), 1)).length);

  // Realce por linha, na ordem de `shown` (só as desenhadas). A removida fica sem cor de sintaxe, como no terminal.
  let tokens = $state<(DiffToken[] | null)[]>([]);
  $effect(() => {
    const ls = shown;
    let alive = true;
    tokens = [];
    highlightCodeLines(ls.map((l) => (l.op === 'del' ? '' : l.text)), path)
      .then((t) => { if (alive && t) tokens = t; })
      .catch((err) => console.warn('[hl] realce do diff falhou', err));
    return () => { alive = false; };
  });
</script>

<div class="td" style:--num-w="{width}ch">
  {#each shown as l, i (i)}
    {#if i > 0 && hunkOf[i] !== hunkOf[i - 1]}
      {@const sep = betweenHunks(diff.paths, hunkOf[i])}
      {#if sep.kind === 'file'}
        <div class="sep" title={sep.path}>{sep.name}</div>
      {:else}
        <div class="sep" aria-hidden="true">{'\u2026'}</div>
      {/if}
    {/if}
    <div class="row" class:add={l.op === 'add'} class:del={l.op === 'del'}>
      <span class="num">{numOf(l)}</span><span class="sig" aria-hidden="true">{l.op === 'add' ? '+' : l.op === 'del' ? '-' : ''}</span><span class="sr-only">{l.op === 'add' ? '+' : l.op === 'del' ? '-' : ' '}</span><code>{#each piecesOf(l.text, l.op === 'del' ? null : tokens[i] ?? null, l.spans) as p, pi (pi)}<span class:mark={p.mark} style={p.color ? `color: ${p.color}` : undefined}>{p.content}</span>{/each}</code>
    </div>
  {/each}
  {#if hidden > 0 || all}
    <button type="button" class="more" aria-expanded={all} onclick={(e) => { e.stopPropagation(); all = !all; }}>{all ? m.term_recolher() : m.term_mais_linhas({ n: hidden })}</button>
  {/if}
</div>

<style>
  /* relative: o .sr-only de cada linha é absoluto e não pode vazar para a rolagem da conversa. */
  .td { position: relative; display: flex; flex-direction: column; min-width: 0; font-family: var(--font-mono); font-size: var(--text-xs); line-height: 1.55; }
  .row { display: grid; grid-template-columns: var(--num-w) 2ch minmax(0, 1fr); color: var(--text-primary); }
  .sep { color: var(--text-muted); }
  .num { text-align: right; color: var(--text-muted); opacity: 0.7; user-select: none; }
  .sig { text-align: center; user-select: none; }
  code { min-width: 0; white-space: pre-wrap; overflow-wrap: anywhere; font: inherit; }
  .row.add { background: color-mix(in srgb, var(--success) 14%, transparent); }
  .row.del { background: color-mix(in srgb, var(--error) 14%, transparent); }
  .row.add .num, .row.add .sig { color: var(--success); opacity: 1; }
  .row.del .num, .row.del .sig { color: var(--error); opacity: 1; }
  .row.add .mark { background: color-mix(in srgb, var(--success) 30%, transparent); }
  .row.del .mark { background: color-mix(in srgb, var(--error) 30%, transparent); }
  .more { align-self: flex-start; justify-content: flex-start; min-height: 32px; padding: 0; border: 0; background: none; font: inherit; color: var(--text-muted); cursor: pointer; }
</style>
