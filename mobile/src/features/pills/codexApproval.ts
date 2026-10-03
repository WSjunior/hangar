import { useCallback, useEffect, useState } from 'react';
import { getCodexPermissions, setCodexPermission } from '@hangar/core';
import * as m from '../../paraglide/messages';

export interface ApprovalMode {
  name: string;
  desc?: string;
}

// Nome e descrição vêm do picker vivo do Codex, em inglês: são dado do agente, não interface.
// Só o rótulo curto do segmento é traduzido, para caber na linha.
export function approvalShortLabel(mode: string): string {
  if (mode === 'Ask for approval') return m.permissao_codex_curta_perguntar();
  if (mode === 'Approve for me') return m.permissao_codex_curta_auto();
  if (mode === 'Full Access') return m.permissao_codex_curta_total();
  return mode;
}

// Nível de aprovação do Codex. Sem terminal o modo está no sidecar e pode ser lido a qualquer
// hora; com terminal, ler dirige o `/permissions` no pane — só quando a pessoa pede.
export function useCodexApproval({ name, headless, enabled }: { name: string; headless: boolean; enabled: boolean }) {
  const [current, setCurrent] = useState<string | null>(null);
  const [modes, setModes] = useState<ApprovalMode[] | null>(null);
  const [loading, setLoading] = useState(false);
  const [applying, setApplying] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [notice, setNotice] = useState<string | null>(null);

  useEffect(() => {
    setCurrent(null);
    setModes(null);
    if (!enabled || !headless) return;
    let alive = true;
    getCodexPermissions(name).then((res) => { if (alive) setCurrent(res.current); }).catch(() => {});
    return () => { alive = false; };
  }, [name, headless, enabled]);

  const load = useCallback(async () => {
    if (!enabled) return;
    setLoading(true);
    setError(null);
    setNotice(null);
    try {
      const res = await getCodexPermissions(name);
      setCurrent(res.current);
      setModes(res.modes.map((mo) => ({ name: mo.nome, desc: mo.desc })));
    } catch (e) {
      setError(e instanceof Error ? e.message : m.comum_falha_aplicar());
    } finally {
      setLoading(false);
    }
  }, [name, enabled]);

  // Diz se a troca pegou: o painel só fecha no sucesso.
  const select = useCallback(async (mode: string): Promise<boolean> => {
    if (applying || mode === current) return false;
    setApplying(true);
    setNotice(null);
    try {
      const res = await setCodexPermission(name, mode);
      setCurrent(res.current);
      return true;
    } catch (e) {
      setNotice(e instanceof Error ? e.message : m.comum_falha_aplicar());
      // A troca pode ter chegado e falhado na volta: relê o que ficou valendo.
      getCodexPermissions(name).then((res) => setCurrent(res.current)).catch(() => {});
      return false;
    } finally {
      setApplying(false);
    }
  }, [applying, current, name]);

  return { current, modes, loading, applying, error, notice, load, select };
}
