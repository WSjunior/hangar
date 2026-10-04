import { abbrevNum, basename, rateLabel, type SessionInfo, type StatsEvent, type StatusFields } from '@hangar/core';
import * as m from '../paraglide/messages';

const fmtDur = (ms: number) =>
  ms < 60_000 ? `${Math.round(ms / 1000)}s` : `${Math.floor(ms / 60_000)}m${Math.round((ms % 60_000) / 1000)}s`;

// Mesma ordem da faixa da PWA; campo que não veio não vira parte vazia.
export function linhaStats(s: StatsEvent): string[] {
  const p = [
    s.turns === 1 ? m.stats_turnos_1() : m.stats_turnos({ n: s.turns }),
    s.steps === 1 ? m.stats_chamadas_1() : m.stats_chamadas({ n: s.steps }),
  ];
  if (s.llm_ms) {
    p.push(m.stats_llm({ t: fmtDur(s.llm_ms) }));
    if (s.tool_ms) p.push(m.stats_tools({ t: fmtDur(s.tool_ms) }));
  }
  if (s.tok_s) p.push(m.stats_toks({ n: Math.round(s.tok_s) }));
  if (s.tok_s_now) p.push(m.stats_toks_now({ n: rateLabel(s.tok_s_now, s.tok_s_exact) }));
  if (s.tok_s_recent) p.push(m.stats_toks_recent({ n: rateLabel(s.tok_s_recent, s.tok_s_exact) }));
  if (s.ttft_ms) p.push(m.stats_ttft({ t: fmtDur(s.ttft_ms) }));
  if (s.cache_pct != null) p.push(m.stats_cache({ n: s.cache_pct }));
  p.push(m.stats_io({ i: abbrevNum(s.in_tok), o: abbrevNum(s.out_tok) }));
  return p;
}

export type UsageWindow = { key: '5h' | '7d' | '30d'; label: string; pct: number; reset?: string };

const known = (n: unknown): n is number => typeof n === 'number' && isFinite(n);

// Janelas de cota da conta, na ordem da folha de Uso da PWA; só entra o que a statusline trouxe.
export function usageWindows(f: StatusFields | null): UsageWindow[] {
  if (!f) return [];
  const out: UsageWindow[] = [];
  if (known(f.fiveHourPct)) out.push({ key: '5h', label: m.uso_janela_5h(), pct: f.fiveHourPct, reset: f.fiveHourReset });
  if (known(f.weeklyPct)) out.push({ key: '7d', label: m.uso_janela_7d(), pct: f.weeklyPct, reset: f.weeklyReset });
  if (known(f.monthlyPct)) out.push({ key: '30d', label: m.uso_janela_30d(), pct: f.monthlyPct, reset: f.monthlyReset });
  return out;
}

// Rótulo do botão único do composer: "Opus5.5·1M · high". Sem modelo lido, cai no nome do campo.
export function settingsLabel(model: string | null | undefined, effort: string | null | undefined): string {
  if (!model) return m.composer_modelo();
  return effort ? `${spacedModel(model)} · ${effort}` : spacedModel(model);
}

// "Opus5.5·1M" → "Opus 5.5 · 1M": a statusline cola tudo; no chip do composer o nome tem de ler
// como no app de PC. Duas letras antes do número: "o3" é nome, não "o 3".
export function spacedModel(model: string): string {
  return model.replace(/([A-Za-z]{2})(\d)/g, '$1 $2').replace(/\s*·\s*/g, ' · ').trim();
}

export type ComposerStatus = {
  folder?: string;
  branch?: string;
  added?: number;
  removed?: number;
  time?: string;
  ctxPct?: number;
  quota?: UsageWindow;
  cost?: string;
};

type GitSession = Pick<SessionInfo, 'cwd' | 'branch' | 'git_added' | 'git_removed'>;

// Linha de status embaixo do composer (a do app de PC): o que a statusline trouxe, e da lista o
// que ela não traz (pasta, branch e o diff). Campo ausente não vira parte vazia.
export function composerStatus(f: StatusFields | null, session: GitSession | null): ComposerStatus {
  const out: ComposerStatus = {};
  const folder = f?.repo ?? (session?.cwd ? basename(session.cwd) : undefined);
  if (folder) out.folder = folder;
  const branch = f?.branch ?? session?.branch ?? undefined;
  if (branch) out.branch = branch;
  const added = session?.git_added;
  const removed = session?.git_removed;
  if (known(added) && added > 0) out.added = added;
  if (known(removed) && removed > 0) out.removed = removed;
  if (f?.sessionTime) out.time = f.sessionTime;
  const ctx = f?.ctxPct;
  if (known(ctx)) out.ctxPct = ctx;
  const quota = usageWindows(f)[0];
  if (quota) out.quota = quota;
  const cost = f?.costUsd;
  if (known(cost)) out.cost = `$${cost.toFixed(2)}`;
  return out;
}
