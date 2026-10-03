import { useEffect, useState } from 'react';
import { Pressable, Switch, Text, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { basename, deleteWorktreeForServer, getWorktreeForServer, type Server, type WorktreeStatus } from '@hangar/core';
import { Sheet } from '../../ui/Sheet';
import * as m from '../../paraglide/messages';
import { dropWorktreeStatus, putWorktreeStatus } from './worktreeStatus';

type Props = { server: Server | null; path: string | null; onClose: () => void; onDeleted?: () => void };

export function WorktreeSheet({ server, path, onClose, onDeleted }: Props) {
  const [st, setSt] = useState<WorktreeStatus | null>(null);
  const [erro, setErro] = useState('');
  const [apagando, setApagando] = useState(false);
  const [apagarBranch, setApagarBranch] = useState(false);
  const serverId = server?.id;

  // Chave pelo id: a lista de servidores troca de objeto a cada atualização do store, e isso não
  // pode zerar a folha aberta.
  useEffect(() => {
    if (!server || !path) return;
    let vivo = true;
    setSt(null); setErro(''); setApagarBranch(false);
    getWorktreeForServer(server, path)
      .then((r) => { if (vivo) { setSt(r); putWorktreeStatus(server.id, r); } })
      .catch((e: unknown) => { if (vivo) setErro(e instanceof Error ? e.message : String(e)); });
    return () => { vivo = false; };
  }, [serverId, path]);

  const perde = st ? [...(st.dirty ? [m.worktree_nao_commitados({ n: st.dirty })] : []), ...st.ignored] : [];

  async function apagar() {
    if (!server || !st) return;
    setApagando(true); setErro('');
    try {
      await deleteWorktreeForServer(server, { repo: st.repo, path: st.path, confirm: perde.length > 0, delete_branch: apagarBranch });
      dropWorktreeStatus(server.id, st.path);
      onDeleted?.();
      onClose();
    } catch (e) {
      setErro(e instanceof Error ? e.message : String(e));
    } finally {
      setApagando(false);
    }
  }

  return (
    <Sheet open={!!path} sizes={['auto']} onDismiss={onClose}>
      <View style={styles.inner}>
        <Text style={styles.title}>{path ? basename(path) : ''}</Text>
        {!st && !erro ? <Text style={styles.muted} accessibilityLiveRegion="polite">{m.comum_carregando()}</Text> : null}
        {!st && erro ? <Text accessibilityRole="alert" style={styles.erro}>{m.worktrees_erro({ motivo: erro })}</Text> : null}
        {st ? (
          <>
            <Text style={styles.text}>{st.branch} ← {st.base}</Text>
            <Text style={styles.text}>{st.merged ? m.worktree_juntada() : m.worktree_nao_juntada({ n: st.ahead })}</Text>
            {st.closed ? <Text style={styles.muted}>{m.worktree_conversas_fechadas({ n: st.closed })}</Text> : null}
            {st.sessions.length ? (
              <Text style={styles.aviso}>{m.worktree_bloqueada({ nomes: st.sessions.join(', ') })}</Text>
            ) : (
              <>
                {perde.length ? <Text style={styles.aviso}>{m.worktree_apagar_perde()} {perde.join(', ')}</Text> : null}
                <Text style={styles.muted}>{m.worktree_apagar_conversas({ branch: st.base ?? '' })}</Text>
                {st.merged
                  ? <Text style={styles.muted}>{m.worktree_apagar_branch_juntada({ branch: st.branch ?? '', base: st.base ?? '' })}</Text>
                  : (
                    <>
                      <Text style={styles.muted}>{m.worktree_apagar_branch_fica({ branch: st.branch ?? '' })}</Text>
                      <View style={styles.linha}>
                        <Switch value={apagarBranch} onValueChange={setApagarBranch} accessibilityLabel={m.worktree_apagar_branch_tambem({ n: st.ahead })} />
                        <Text style={styles.text}>{m.worktree_apagar_branch_tambem({ n: st.ahead })}</Text>
                      </View>
                    </>
                  )}
                {erro ? <Text accessibilityRole="alert" style={styles.erro}>{erro}</Text> : null}
                <Pressable onPress={apagar} disabled={apagando} accessibilityRole="button" accessibilityLabel={m.worktree_apagar()}
                  accessibilityState={{ busy: apagando, disabled: apagando }} style={styles.botao}>
                  <Text style={styles.botaoTexto}>{m.worktree_apagar()}</Text>
                </Pressable>
              </>
            )}
          </>
        ) : null}
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create((theme) => ({
  inner: { padding: theme.base.space[4], gap: theme.base.space[2] },
  title: { fontSize: theme.base.text.lg, fontWeight: '600', color: theme.tokens.text.primary },
  text: { color: theme.tokens.text.primary },
  muted: { color: theme.tokens.text.secondary },
  aviso: { color: theme.tokens.status.warning },
  erro: { color: theme.tokens.status.error },
  linha: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[2] },
  botao: { alignSelf: 'flex-end', minHeight: 44, paddingHorizontal: theme.base.space[4], justifyContent: 'center',
           borderRadius: theme.base.radius.md, backgroundColor: theme.tokens.status.error },
  botaoTexto: { color: '#fff', fontWeight: '600' },
}));
