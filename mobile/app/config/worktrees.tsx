import { useCallback, useEffect, useRef, useState } from 'react';
import { Text } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { basename, deleteMergedWorktreesForServer, fetchWorktreesForServer, getWorktreesForServer, type WorktreeRepo } from '@hangar/core';
import { Pagina } from '../../src/features/config/Pagina';
import { PageHeader, Pill } from '../../src/features/config/PageHeader';
import { SectionCard } from '../../src/features/config/SectionCard';
import { SettingsRow } from '../../src/features/config/SettingsRow';
import { InfoNotice } from '../../src/features/config/InfoNotice';
import { WorktreeSheet } from '../../src/features/worktrees/WorktreeSheet';
import { putWorktreeStatus } from '../../src/features/worktrees/worktreeStatus';
import { toast } from '../../src/ui/Toast';
import { useServers } from '../../src/stores/servers';
import * as m from '../../src/paraglide/messages';

export default function Worktrees() {
  const server = useServers((s) => s.active());
  // A leitura segue o id: o store troca o objeto do servidor a cada atualização da lista, e isso
  // não pode reler nem zerar a tela.
  const serverRef = useRef(server);
  serverRef.current = server;
  const serverId = server?.id ?? null;
  const geracao = useRef(0);
  const [repos, setRepos] = useState<WorktreeRepo[] | null>(null);
  const [erro, setErro] = useState('');
  const [atualizando, setAtualizando] = useState(false);
  const [aberta, setAberta] = useState<string | null>(null);

  const carregar = useCallback(async (): Promise<WorktreeRepo[] | null> => {
    const alvo = serverRef.current;
    if (!alvo) return null;
    const g = geracao.current;
    try {
      const r = await getWorktreesForServer(alvo);
      if (g !== geracao.current) return null;
      r.forEach((repo) => repo.worktrees.forEach((w) => putWorktreeStatus(alvo.id, w)));
      setRepos(r);
      setErro('');
      return r;
    } catch (e) {
      if (g === geracao.current) setErro(e instanceof Error ? e.message : String(e));
      return null;
    }
  }, []);

  useEffect(() => {
    const g = ++geracao.current;
    setRepos(null); setErro(''); setAberta(null); setAtualizando(false);
    const alvo = serverRef.current;
    if (!alvo) return;
    void (async () => {
      // Mostra o que já está no disco e só então busca o remoto, que pode demorar.
      const lidos = await carregar();
      if (!lidos || g !== geracao.current) return;
      setAtualizando(true);
      await Promise.allSettled(lidos.map((r) => fetchWorktreesForServer(alvo, r.repo)));
      if (g !== geracao.current) return;
      setAtualizando(false);
      await carregar();
    })();
  }, [serverId, carregar]);

  const apagarJuntadas = async (repo: string) => {
    const alvo = serverRef.current;
    if (!alvo) return;
    try {
      await deleteMergedWorktreesForServer(alvo, repo);
    } catch (e) {
      toast.erro(e instanceof Error ? e.message : String(e));
    }
    await carregar();
  };

  return (
    <Pagina>
      <PageHeader title={m.worktrees_titulo()} subtitle={atualizando ? m.worktrees_atualizando() : undefined} />
      {!server ? <InfoNotice text={m.maquinas_vazio()} /> : null}
      {server && repos === null && !erro ? <Text style={styles.muted}>{m.comum_carregando()}</Text> : null}
      {erro ? <Text accessibilityRole="alert" style={styles.erro}>{m.worktrees_erro({ motivo: erro })}</Text> : null}
      {repos && repos.length === 0 ? <InfoNotice text={m.worktrees_vazio()} /> : null}
      {repos?.map((r) => {
        const limpas = r.worktrees.filter((w) => w.merged && !w.dirty && !w.ignored.length && !w.sessions.length);
        return (
          <SectionCard
            key={r.repo}
            icon="GitBranch"
            title={basename(r.repo)}
            extra={limpas.length ? (
              <Pill icon="Trash2" label={m.worktree_apagar_juntadas({ n: limpas.length })} onPress={() => void apagarJuntadas(r.repo)} />
            ) : undefined}
          >
            {r.worktrees.map((w) => (
              <SettingsRow
                key={w.path}
                icon="GitBranch"
                title={basename(w.path)}
                description={`${w.branch ?? ''} ← ${w.base ?? ''} · ${w.merged ? m.worktree_juntada() : m.worktree_nao_juntada({ n: w.ahead })}`}
                onPress={() => setAberta(w.path)}
              />
            ))}
          </SectionCard>
        );
      })}
      <WorktreeSheet server={server} path={aberta} onClose={() => setAberta(null)} onDeleted={() => void carregar()} />
    </Pagina>
  );
}

const styles = StyleSheet.create((theme) => ({
  muted: { fontSize: theme.base.text.sm, color: theme.tokens.text.muted, paddingHorizontal: 4 },
  erro: { fontSize: theme.base.text.sm, color: theme.tokens.status.error, paddingHorizontal: 4 },
}));
