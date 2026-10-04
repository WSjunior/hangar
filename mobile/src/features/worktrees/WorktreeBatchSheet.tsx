import { useState } from 'react';
import { Pressable, Text, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { basename, deleteMergedWorktreesForServer, type Server, type WorktreeStatus } from '@hangar/core';
import { Sheet } from '../../ui/Sheet';
import * as m from '../../paraglide/messages';
import { dropWorktreeStatus } from './worktreeStatus';

export type WorktreeBatch = { repo: string; deletable: WorktreeStatus[]; blocked: WorktreeStatus[] };
type Props = { server: Server | null; batch: WorktreeBatch | null; onClose: () => void; onDeleted: (leftOut: string[]) => void };

/** Confirmação do lote de mescladas: mostra o que cada uma perde e apaga só as que a pessoa viu. */
export function WorktreeBatchSheet({ server, batch, onClose, onDeleted }: Props) {
  const [apagando, setApagando] = useState(false);
  const [erro, setErro] = useState('');

  async function apagar() {
    if (!server || !batch || apagando) return;
    setApagando(true); setErro('');
    try {
      const removidas = await deleteMergedWorktreesForServer(server, batch.repo,
        { paths: batch.deletable.map((w) => w.path), confirm: true });
      removidas.forEach((p) => dropWorktreeStatus(server.id, p));
      onDeleted(batch.deletable.filter((w) => !removidas.includes(w.path)).map((w) => basename(w.path)));
      onClose();
    } catch (e) {
      setErro(e instanceof Error ? e.message : String(e));
    } finally {
      setApagando(false);
    }
  }

  const n = batch?.deletable.length ?? 0;
  return (
    <Sheet open={!!batch} sizes={['auto']} onDismiss={onClose}>
      <View style={styles.inner}>
        <Text style={styles.title}>{m.worktree_lote_titulo({ n })}</Text>
        {batch?.deletable.map((w) => (
          <View key={w.path} style={styles.item}>
            <Text style={styles.nome}>{basename(w.path)}</Text>
            {w.dirty || w.ignored.length ? (
              <>
                <Text style={styles.aviso}>{m.worktree_apagar_perde()}</Text>
                {w.dirty ? <Text style={styles.arquivo}>{m.worktree_nao_commitados({ n: w.dirty })}</Text> : null}
                {w.ignored.map((f) => <Text key={f} style={styles.arquivo}>{f}</Text>)}
              </>
            ) : <Text style={styles.muted}>{m.worktree_lote_nada_perde()}</Text>}
          </View>
        ))}
        {batch?.blocked.length ? (
          <>
            <Text style={styles.aviso}>{m.worktree_lote_ficam()}</Text>
            {batch.blocked.map((w) => (
              <View key={w.path} style={styles.item}>
                <Text style={styles.nome}>{basename(w.path)}</Text>
                <Text style={styles.muted}>
                  {w.sessions.length ? m.worktree_sessao_aberta({ nomes: w.sessions.join(', ') }) : m.worktree_lote_leitura_falhou()}
                </Text>
              </View>
            ))}
          </>
        ) : null}
        {erro ? <Text accessibilityRole="alert" style={styles.erro}>{erro}</Text> : null}
        <View style={styles.acoes}>
          <Pressable onPress={onClose} accessibilityRole="button" style={styles.cancelar}>
            <Text style={styles.text}>{m.comum_cancelar()}</Text>
          </Pressable>
          <Pressable onPress={apagar} disabled={apagando} accessibilityRole="button" accessibilityLabel={m.worktree_lote_confirmar()}
            accessibilityState={{ busy: apagando, disabled: apagando }} style={styles.botao}>
            <Text style={styles.botaoTexto}>{m.worktree_lote_confirmar()}</Text>
          </Pressable>
        </View>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create((theme) => ({
  inner: { padding: theme.base.space[4], gap: theme.base.space[2] },
  title: { fontSize: theme.base.text.lg, fontWeight: '600', color: theme.tokens.text.primary },
  item: { gap: 2 },
  nome: { fontWeight: '600', color: theme.tokens.text.primary },
  text: { color: theme.tokens.text.primary },
  muted: { color: theme.tokens.text.secondary },
  aviso: { color: theme.tokens.status.warning },
  arquivo: { color: theme.tokens.text.secondary, fontFamily: theme.base.fontMono, fontSize: theme.base.text.sm, paddingLeft: theme.base.space[3] },
  erro: { color: theme.tokens.status.error },
  acoes: { flexDirection: 'row', justifyContent: 'flex-end', gap: theme.base.space[2], marginTop: theme.base.space[2] },
  cancelar: { minHeight: 44, paddingHorizontal: theme.base.space[4], justifyContent: 'center', borderRadius: theme.base.radius.md,
              borderWidth: 1, borderColor: theme.tokens.border.default },
  botao: { minHeight: 44, paddingHorizontal: theme.base.space[4], justifyContent: 'center',
           borderRadius: theme.base.radius.md, backgroundColor: theme.tokens.status.error },
  botaoTexto: { color: '#fff', fontWeight: '600' },
}));
