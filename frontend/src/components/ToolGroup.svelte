<script lang="ts">
  import type { ChatEvent } from '@hangar/core';
  import * as m from '../paraglide/messages';
  import { summarizeToolInput, summarizeToolResult, toolGroupCounts, splitTerminalRun, terminalFoldTitle, toolGroupLabel, toolGroupTitulo, toolPhase } from '@hangar/core';
  import { toolLook } from '../lib/toolLook.svelte';
  import ToolCard from './ToolCard.svelte';
  import ToolGlyph from './ToolGlyph.svelte';
  import HangarTrail, { type PassoHangar } from './HangarTrail.svelte';
  import { lerComandoHangar } from '../lib/hangarCmd';

  interface Props {
    tools: ChatEvent[];
    // mesmo wrapper de toolResults do MessageList (Map incremental): tool_use_id -> tool_result.
    toolResults: { get: (id: string) => ChatEvent | undefined };
    sessionName: string;
    animate?: boolean;   // false = grupo de HISTORICO remontado (paginacao/janela): sem fade
  }
  let { tools, toolResults, sessionName, animate = true }: Props = $props();

  // Colapsado por padrao: o burst vira UMA linha — cabecalho com a contagem. A arvore de uma linha
  // por chamada saiu: 22 Bash viravam 22 linhas de scroll, e o argumento que ela mostrava e o mesmo
  // que o ToolCard ja poe na linha 1. Tap abre os ToolCards completos (cada um com o proprio tap
  // pra saida). Enquanto o burst roda, a chamada VIVA aparece sob o cabecalho — sem ela o painel
  // diria "3 concluidos" e esconderia o que esta acontecendo agora.
  // Na pele 'chips' grupo curto nasce aberto (a lista de trabalho É a leitura); rajada longa continua
  // recolhida pelo mesmo motivo acima.
  // svelte-ignore state_referenced_locally -- decisão da montagem; o grupo que cresce não reabre.
  let expanded = $state(toolLook.look === 'chips' && tools.length <= 5);

  const resultOf = (t: ChatEvent) => toolResults.get(t.tool_use_id ?? '') ?? null;

  // Comandos do hangar deste grupo, na ordem. Só entram os que já têm resultado: sem ele não há o
  // que ler, e um passo "?" na trilha seria pior que a ausência dele.
  const trilha = $derived.by<PassoHangar[]>(() => {
    const passos: PassoHangar[] = [];
    for (const t of tools) {
      if (t.tool_name !== 'Bash') continue;
      const r = resultOf(t);
      if (!r) continue;
      const comando = String((t.tool_input as Record<string, unknown> | null)?.['command'] ?? '');
      const acao = lerComandoHangar(comando, String(r.result ?? ''), toolPhase(r) === 'error');
      if (!acao) continue;
      passos.push({
        acao,
        hora: t.ts ? new Date(t.ts * 1000).toLocaleTimeString(undefined, { hour: '2-digit', minute: '2-digit' }) : null,
      });
    }
    return passos;
  });
  // Do início do primeiro comando ao fim do último — inclui o que rodou ENTRE eles, e é isso mesmo
  // que a trilha mede: quanto tempo a sequência levou.
  const duracaoTrilha = $derived.by(() => {
    const comandos = tools.filter((t) => t.tool_name === 'Bash' && resultOf(t));
    const primeiro = comandos[0]?.ts;
    const ultimo = comandos.length ? resultOf(comandos[comandos.length - 1])?.ts : null;
    return primeiro && ultimo ? Math.max(0, (ultimo - primeiro) * 1000) : null;
  });

  const phases = $derived(tools.map((t) => toolPhase(resultOf(t))));
  const label = $derived(toolGroupLabel(tools.map((t) => t.tool_name)));
  const counts = $derived(toolGroupCounts(phases));
  const anyError = $derived(phases.includes('error'));
  // Todas do mesmo tipo -> o nome ja esta no cabecalho e some de cada filho (o que a arvore do Pi
  // faz); misturadas -> cada filho carrega o proprio nome, senao a linha vira um path sem dono.
  const mixed = $derived(label === m.lista_ferramentas());

  const titulo = $derived(toolGroupTitulo(tools));

  // Pele 'terminal': sem moldura de grupo. Buscas, leituras, MCP e comandos seguidos somam numa linha
  // cinza, como no Claude Code; edição nunca fica escondida.
  const termParts = $derived(toolLook.look === 'terminal' ? splitTerminalRun(tools) : []);
  let openFolds = $state<Record<string, boolean>>({});
  const faseDe = $derived(new Map(tools.map((t, i) => [t, phases[i]])));
  // Na pele 'terminal' uma chamada só também passa pela dobra, como o "Ran 1 shell command" do Claude Code.
  const sozinha = $derived(tools.length === 1 && toolLook.look !== 'terminal');

  // A chamada viva: a ULTIMA pendente (a mais nova), como o "$ …" que o Claude mostra sob o resumo.
  const running = $derived.by(() => {
    for (let i = tools.length - 1; i >= 0; i--) if (phases[i] === 'pending') return { t: tools[i], i };
    return null;
  });
</script>

<!-- Uma ferramenta so nao e grupo: "Executou 1 ferramenta ›" esconderia a query atras de um tap a
     mais. Desenha o bloco do ToolCard direto (a regra de agrupar vive no MessageList, mas o guarda
     fica aqui pra valer pra qualquer chamador). -->
{#if sozinha}
  <ToolCard event={tools[0]} result={resultOf(tools[0])} {sessionName} {animate} />
{:else if toolLook.look === 'terminal'}
  <div class="tg-term">
    {#each termParts as part (part.kind === 'fold' ? `f-${part.tools[0].id}` : part.tool.id)}
      {#if part.kind === 'fold'}
        {@const key = part.tools[0].id}
        {@const viva = part.tools.findLast((t) => faseDe.get(t) === 'pending') ?? null}
        <button type="button" class="tg-fold" aria-expanded={!!openFolds[key]}
                onclick={() => (openFolds[key] = !openFolds[key])}>{terminalFoldTitle(part.tools, viva !== null)}</button>
        {#if openFolds[key]}
          {#each part.tools as t (t.id)}
            <ToolCard event={t} result={resultOf(t)} {sessionName} {animate} />
          {/each}
        {:else}
          <!-- Recolhida, a linha mostra o que roda agora e a saída de cada falha, como o Claude Code. -->
          {#if viva}
            <div class="tg-fold-sub"><span aria-hidden="true">⎿</span><span class="tg-fold-txt">{summarizeToolInput(viva.tool_name, viva.tool_input)}</span></div>
          {/if}
          {#each part.tools.filter((t) => faseDe.get(t) === 'error') as t (t.id)}
            <div class="tg-fold-sub tg-fold-erro"><span aria-hidden="true">⎿</span><span class="tg-fold-txt">{summarizeToolResult(resultOf(t), t.tool_name)}</span></div>
          {/each}
        {/if}
      {:else}
        <ToolCard event={part.tool} result={resultOf(part.tool)} {sessionName} {animate} />
      {/if}
    {/each}
  </div>
{:else}
<!-- Rajada de comandos do hangar: a trilha resume a sequência ANTES do grupo, e o grupo continua
     ali com os cartões um a um. Ela só aparece com 2+ comandos lidos — com um só o cartão já conta
     a história inteira. -->
{#if trilha.length > 1}
  <HangarTrail passos={trilha} total={duracaoTrilha} />
{/if}
<div class="tg" class:noanim={!animate}>
  <div
    class="tg-head"
    class:tg-head--error={anyError}
    role="button"
    tabindex="0"
    aria-expanded={expanded}
    onclick={() => (expanded = !expanded)}
    onkeydown={(e) => { if (e.key === 'Enter' || e.key === ' ') { e.preventDefault(); expanded = !expanded; } }}
  >
    {#if toolLook.look === 'chips'}
      <!-- Cabeçalho da pele 'chips': ícone da família da última chamada + o título do grupo + quantas
           chamadas; o erro pinta o título, no lugar da bolinha da pele clássica. -->
      <span class="tg-fam"><ToolGlyph tool={tools[tools.length - 1].tool_name} /></span>
      <span class="tg-titulo">{titulo}</span>
      <span class="tg-n">· {m.tool_n_chamadas({ n: tools.length })}</span>
      <svg class="tg-chevron" class:open={expanded} width="12" height="12" viewBox="0 0 24 24"
           fill="none" stroke="currentColor" stroke-width="2.2" stroke-linecap="round"
           stroke-linejoin="round" aria-hidden="true"><path d="M6 9l6 6 6-6" /></svg>
    {:else}
      <span class="tg-dot" class:pending={phases.includes('pending')}
            data-phase={anyError ? 'error' : phases.includes('pending') ? 'pending' : 'done'} aria-hidden="true"></span>
      <span class="tg-label">{label}:</span>
      <span class="tg-counts">{counts}</span>
      <span class="tg-hint">
        <span class="sep" aria-hidden="true">•</span>
        <span class="coarse">{expanded ? m.tool_toque_ocultar() : m.tool_toque_ver()}</span><span
              class="fine">{expanded ? m.tool_clique_ocultar() : m.tool_clique_ver()}</span>
      </span>
    {/if}
  </div>

  {#if expanded && toolLook.look === 'chips'}
    <!-- Cada linha já traz o +/− da própria edição, então a faixa de arquivos do fim saiu. -->
    <div class="tg-body tg-body--chips">
      {#each tools as t, i (t.id)}
        <ToolCard event={t} result={resultOf(t)} {sessionName} animate={false} emGrupo ultimo={i === tools.length - 1} />
      {/each}
    </div>
  {:else if expanded}
    <div class="tg-body">
      {#each tools as t (t.id)}
        <ToolCard event={t} result={resultOf(t)} {sessionName} animate={false} />
      {/each}
    </div>
  {:else if running && toolLook.look === 'chips'}
    <div class="tg-body tg-body--chips">
      <ToolCard event={running.t} result={null} {sessionName} animate={false} emGrupo ultimo />
    </div>
  {:else if running}
    <!-- Uma linha so: a chamada em curso. O "└" e CSS (tronco + bracinho), nao box-drawing — em
         fonte de sistema o glifo cai em fallback e desalinha da bolinha. -->
    <div class="tg-tree">
      <div class="tg-child">
        <span class="tg-dot tg-dot--child pending" data-phase="pending" aria-hidden="true"></span>
        {#if mixed}<span class="tg-cname">{running.t.tool_name ?? 'Tool'}</span>{/if}
        <span class="tg-arg">{summarizeToolInput(running.t.tool_name, running.t.tool_input)}</span>
      </div>
    </div>
  {/if}

</div>
{/if}

<style>
  .tg-term { display: flex; flex-direction: column; gap: var(--space-1); min-width: 0; }
  .tg-fold {
    justify-content: flex-start; min-height: 24px; padding: 0 0 0 14px; border: 0; background: transparent;
    font-family: var(--font-mono); font-size: var(--text-xs); line-height: 1.55;
    color: var(--text-muted); text-align: left; cursor: pointer;
  }
  .tg-fold-sub {
    display: flex; gap: 8px; min-width: 0; padding-left: 14px;
    font-family: var(--font-mono); font-size: var(--text-xs); line-height: 1.55; color: var(--text-muted);
  }
  .tg-fold-txt { min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; }
  .tg-fold-erro { color: var(--error); }
  .tg { margin-bottom: var(--space-1); animation: bubble-in 180ms ease-out both; }
  .tg.noanim { animation: none; }

  .tg-head {
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    padding: var(--space-1) 0;
    font-size: var(--text-xs);
    line-height: 1.5;
    color: var(--text-muted);
    cursor: pointer;
  }

  /* Mesma bolinha do ToolCard (mesmas cores de estado) — no cabecalho ela resume o grupo. */
  .tg-dot {
    flex-shrink: 0;
    width: 6px;
    height: 6px;
    border-radius: 50%;
    align-self: center;
    background: var(--success);
  }
  .tg-dot[data-phase='pending'] { background: var(--accent); }
  .tg-dot[data-phase='error']   { background: var(--error); }
  .tg-dot.pending { animation: pulse-scale 1.2s ease-in-out infinite; }

  /* Cabeçalho da pele 'chips'. */
  .tg-chevron {
    flex-shrink: 0;
    align-self: center;
    color: var(--text-muted);
    transition: transform 200ms var(--ease-out);
    transform: rotate(-90deg);
  }
  .tg-chevron.open { transform: rotate(0deg); }
  .tg-fam { display: inline-flex; flex-shrink: 0; align-self: center; width: 16px; justify-content: center; color: var(--text-muted); }
  .tg-titulo {
    min-width: 0;
    font-size: 13px;
    color: var(--text-secondary);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }
  .tg-head:hover .tg-titulo { color: var(--text-primary); }
  .tg-head--error .tg-titulo { color: var(--error); }
  .tg-n { flex-shrink: 0; font-size: 12px; color: var(--text-muted); font-variant-numeric: tabular-nums; }
  .tg-head .tg-chevron { margin-left: auto; }

  .tg-label { flex-shrink: 0; font-weight: 600; color: var(--text-secondary); }
  .tg-counts { flex-shrink: 0; }
  .tg-head--error .tg-counts { color: var(--error); }

  .tg-hint {
    flex-shrink: 1000;
    min-width: 0;
    white-space: nowrap;
    overflow: hidden;
    opacity: 0.7;
  }
  .tg-hint .sep { margin-right: 4px; }
  .fine { display: inline; }
  .coarse { display: none; }
  @media (pointer: coarse) {
    .fine { display: none; }
    .coarse { display: inline; }
  }

  /* Arvore colapsada. */
  .tg-tree { padding-bottom: var(--space-1); }

  .tg-child {
    position: relative;
    display: flex;
    align-items: baseline;
    gap: 6px;
    min-width: 0;
    padding-left: 14px;
    font-size: var(--text-xs);
    line-height: 1.6;
  }
  /* Tronco vertical que para na metade e vira pra direita = "└". */
  .tg-child::before {
    content: '';
    position: absolute;
    left: 2px;
    top: 0;
    bottom: 50%;
    width: 6px;
    border-left: 1px solid var(--border-default);
    border-bottom: 1px solid var(--border-default);
    border-bottom-left-radius: 3px;
  }

  .tg-dot--child { width: 5px; height: 5px; }

  .tg-cname { flex-shrink: 0; font-weight: 600; color: var(--text-secondary); }

  .tg-arg {
    min-width: 0;
    font-family: var(--font-mono);
    color: var(--text-muted);
    white-space: nowrap;
    overflow: hidden;
    text-overflow: ellipsis;
  }

  /* Corpo expandido: os ToolCards individuais, recuados sob o tronco do grupo. */
  .tg-body { padding-left: var(--space-3); border-left: 1px solid var(--border-subtle); margin-left: 2px; }

  /* Na pele 'chips' o tronco é das próprias linhas (conector em L no ToolCard). */
  .tg-body--chips {
    display: flex;
    flex-direction: column;
    padding-left: 0;
    margin-left: 0;
    border-left: none;
  }
</style>
