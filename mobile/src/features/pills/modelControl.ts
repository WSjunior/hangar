import { useCallback, useMemo, useRef, useState } from 'react';
import { parseStatusLine, getModelOptions, getPiModels, getKimiModels, getCodexModels, setModelEffort, setEngineModel, setPiModel, setKimiModel, setCodexModel } from '@hangar/core';
import type { CodexModelsResponse } from '@hangar/core';
import { chatStore } from '../../stores/chat';
import * as m from '../../paraglide/messages';
import type { PillMenuItem } from './PillMenu';
import { pillLabels, type Chosen } from './pills';
import { spacedModel } from '../../chat/usage';

interface Args {
  serverId: string;
  name: string;
  provider: string | null;
  chosen: Chosen;
  onChosen: (next: Chosen) => void;
  // Fecha a folha de ajustes depois de trocar o modelo.
  close: () => void;
}

// A statusline escreve "Opus5.5·1M" e a lista do Claude, "Opus 5.5": compara sem espaço, pontuação
// e o sufixo de contexto, senão o modelo atual nunca aparece marcado.
const semEnfeite = (s: string) => s.split('·')[0].toLowerCase().replace(/[^a-z0-9]/g, '');
function mesmoModelo(nome: string, statusModel: string | null | undefined): boolean {
  return !!statusModel && semEnfeite(nome) === semEnfeite(statusModel);
}

type Catalog =
  | { kind: 'codex'; data: CodexModelsResponse }
  | { kind: 'claude' | 'engine'; names: Map<string, string> }
  | { kind: 'other' };

export function useModelControl({ serverId, name, provider, chosen, onChosen, close }: Args) {
  const chat = chatStore(serverId, name);
  const statusLine = chat.use((s) => s.statusLine);
  const statusFields = useMemo(() => parseStatusLine(statusLine), [statusLine]);

  const isCodex = provider === 'codex';
  // omp é o fork do Pi: mesmo seletor, mesmos endpoints.
  const isPi = provider === 'pi' || provider === 'omp';
  const isKimi = provider === 'kimi';

  const [tempError, setTempError] = useState<string | null>(null);
  const [loading, setLoading] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);
  const [items, setItems] = useState<PillMenuItem[]>([]);
  const catalog = useRef<Catalog>({ kind: 'other' });

  const flash = useCallback((msg: string) => {
    setTempError(msg);
    setTimeout(() => setTempError(null), 8000);
  }, []);

  const model = pillLabels(statusFields, chosen).model;
  const display = tempError ?? (model ? spacedModel(model) : m.composer_modelo());

  const load = useCallback(async () => {
    setLoading(true);
    setError(null);
    setNotice(null);
    const current = chosen.model ?? statusFields?.model;
    try {
      if (isCodex) {
        const res = await getCodexModels(name);
        catalog.current = { kind: 'codex', data: res };
        setItems(res.models.map((mo) => ({
          id: mo.model, label: mo.displayName ?? mo.model, hint: mo.description ?? undefined,
          selected: mo.model === (chosen.model ?? res.current.model),
        })));
      } else if (isPi) {
        const res = await getPiModels(name);
        setItems(res.models.map((mo) => ({
          label: mo.name ?? mo.id, hint: `${mo.provider}/${mo.id}`, selected: (mo.name ?? mo.id) === current,
        })));
      } else if (isKimi) {
        const res = await getKimiModels(name);
        setItems(res.models.map((mo) => ({ id: mo.alias, label: mo.name, hint: mo.alias, selected: mo.name === current })));
      } else {
        const res = await getModelOptions(name);
        // Conta Claude: id longo (`claude-*`) é versão antiga presa no catálogo; o picker só aceita
        // os apelidos vivos (default/opus/...), então as antigas nem aparecem.
        const models = res.kind === 'claude' && !res.engine
          ? res.models.filter((mo) => !mo.id.startsWith('claude-'))
          : res.models;
        catalog.current = { kind: res.kind, names: new Map(models.map((mo) => [mo.id, mo.name ?? mo.id])) };
        setItems(models.map((mo) => ({
          id: mo.id, label: mo.name ?? mo.id, hint: mo.desc ?? undefined,
          selected: chosen.model ? (mo.name ?? mo.id) === chosen.model || mo.id === chosen.model : mesmoModelo(mo.name ?? mo.id, current),
        })));
      }
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [name, isCodex, isPi, isKimi, chosen.model, statusFields?.model]);

  const select = useCallback(
    async (it: PillMenuItem) => {
      const id = it.id ?? it.label;
      let closedEarly = false;
      setNotice(null);
      try {
        const cat = catalog.current;
        if (isCodex && cat.kind === 'codex') {
          // Trocar de modelo leva o nível padrão do NOVO: o antigo pode nem existir na lista dele.
          const md = cat.data.models.find((x) => x.model === id);
          const effort = id === cat.data.current.model
            ? (cat.data.current.effort ?? md?.defaultEffort ?? null)
            : (md?.defaultEffort ?? md?.efforts[0]?.value ?? null);
          await setCodexModel(name, id, effort);
          onChosen({ model: id, effort });
          close();
        } else if (isPi) {
          const alias = it.hint ?? it.label;
          const slash = alias.indexOf('/');
          const res = await setPiModel(name, { provider: slash >= 0 ? alias.slice(0, slash) : undefined, model: slash >= 0 ? alias.slice(slash + 1) : alias });
          onChosen({ ...chosen, model: res.current?.name ?? res.current?.id ?? it.label, effort: res.thinking ?? chosen.effort });
          close();
        } else if (isKimi) {
          const res = await setKimiModel(name, { model: id });
          onChosen({ ...chosen, model: res.current?.name ?? it.label });
          close();
        } else if (cat.kind === 'engine') {
          // Sessão de motor: o id do provedor é o rótulo, e a troca vale só nesta sessão.
          const res = await setEngineModel(name, { model: id, effort: chosen.effort ?? statusFields?.effort ?? undefined });
          onChosen({ ...chosen, model: res.model });
          if (res.effort_error) {
            setNotice(m.modelo_trocado_esforco_nao({ erro: res.effort_error }));
            return;
          }
          close();
        } else {
          // Fecha antes da resposta: a troca pode abrir uma confirmação na conversa, que a folha taparia.
          closedEarly = true;
          close();
          const res = await setModelEffort(name, { model: id, scope: 'session' });
          if (res?.pending_confirm) return;
          const label = cat.kind === 'claude' ? cat.names.get(id) : undefined;
          onChosen({
            ...chosen,
            model: id === 'default' ? null : label?.replace(/\s*\(1M context\)/i, '·1M').trim() || id.charAt(0).toUpperCase() + id.slice(1),
          });
        }
      } catch (e) {
        const status = (e as { status?: number }).status;
        const msg = e instanceof Error ? e.message : String(e);
        if (status === 409 || closedEarly) {
          close();
          flash(msg || m.composer_sessao_trabalhando());
        } else {
          setNotice(msg);
        }
      }
    },
    [name, isCodex, isPi, isKimi, chosen, onChosen, statusFields?.effort, flash, close],
  );

  return { model, display, failed: tempError !== null, items, loading, error, notice, load, select };
}
