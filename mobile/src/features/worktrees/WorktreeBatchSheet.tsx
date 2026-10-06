import { useEffect, useState } from 'react';
import { Pressable, Switch, Text, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { basename, deleteMergedWorktreesForServer, fmtBytes, worktreeReady, type Server, type WorktreeStatus } from '@hangar/core';
import { Sheet } from '../../ui/Sheet';
import * as m from '../../paraglide/messages';
import { dropWorktreeStatus } from './worktreeStatus';

export type WorktreeBatch = { repo: string; deletable: WorktreeStatus[]; blocked: WorktreeStatus[] };
type Props = { server: Server | null; batch: WorktreeBatch | null; onClose: () => void; onDeleted: (leftOut: string[]) => void };

/** Confirmação do lote de mescladas: as prontas entram sempre; as com alteração não commitada só se a pessoa marcar. */
export function WorktreeBatchSheet({ server, batch: aberto, onClose, onDeleted }: Props) {
  const [apagando, setApagando] = useState(false);
  const [erro, setErro] = useState('');
  const [marcadas, setMarcadas] = useState<Set<string>>(new Set());
  // O último lote continua desenhado enquanto a folha anima a saída.
  const [batch, setBatch] = useState(aberto);
  useEffect(() => {
    if (aberto) { setBatch(aberto); setErro(''); setMarcadas(new Set()); }
  }, [aberto]);

  const prontas = batch?.deletable.filter(worktreeReady) ?? [];
  const comArquivos = batch?.deletable.filter((w) => !worktreeReady(w)) ?? [];
  const escolhidas = [...prontas, ...comArquivos.filter((w) => marcadas.has(w.path))];
  const n = escolhidas.length;
  const libera = escolhidas.reduce((a, w) => a + (w.size ?? 0), 0);

  const alternar = (path: string) => setMarcadas((s) => {
    const novo = new Set(s);
    if (!novo.delete(path)) novo.add(path);
    return novo;
  });

  async function apagar() {
    if (!server || !batch || apagando || !n) return;
    setApagando(true); setErro('');
    try {
      // O core marca como "perde arquivos" só as que esta tela mostrou perdendo.
      const removidas = await deleteMergedWorktreesForServer(server, batch.repo, escolhidas);
      removidas.forEach((p) => dropWorktreeStatus(server.id, p));
      onDeleted(escolhidas.filter((w) => !removidas.includes(w.path)).map((w) => basename(w.path)));
      onClose();
    } catch (e) {
      setErro(e instanceof Error ? e.message : String(e));
      onDeleted([]);   // algumas podem ter saído antes do erro: a lista recarrega
    } finally {
      setApagando(false);
    }
  }

  const perdas = (w: WorktreeStatus) => (
    <>
      {w.dirty ? <Text style={styles.arquivo}>{m.worktree_nao_commitados({ n: w.dirty })}</Text> : null}
      {w.ignored.map((f) => <Text key={f} style={styles.arquivo}>{f}</Text>)}
    </>
  );

  return (
    <Sheet open={!!aberto} sizes={['auto']} onDismiss={onClose}>
      <View style={styles.inner}>
        <Text style={styles.title}>{m.worktree_lote_titulo({ n })}</Text>
        {libera ? <Text style={styles.ok}>{m.worktree_libera({ tamanho: fmtBytes(libera) })}</Text> : null}
        {prontas.map((w) => (
          <View key={w.path} style={styles.item}>
            <Text style={styles.nome}>{basename(w.path)}</Text>
            {w.ignored.length ? (
              <>
                <Text style={styles.aviso}>{m.worktree_apagar_perde()}</Text>
                {perdas(w)}
              </>
            ) : <Text style={styles.muted}>{m.worktree_lote_nada_perde()}</Text>}
          </View>
        ))}
        {comArquivos.length ? (
          <>
            <Text style={styles.aviso}>{m.worktree_lote_com_arquivos()}</Text>
            {comArquivos.map((w) => (
              <View key={w.path} style={styles.marcavel}>
                <View style={[styles.item, styles.flex]}>
                  <Text style={styles.nome}>{basename(w.path)}</Text>
                  <Text style={styles.aviso}>{m.worktree_apagar_perde()}</Text>
                  {perdas(w)}
                </View>
                <Switch value={marcadas.has(w.path)} onValueChange={() => alternar(w.path)} disabled={apagando}
                  accessibilityLabel={basename(w.path)} style={styles.switch} />
              </View>
            ))}
          </>
        ) : null}
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
          <Pressable onPress={onClose} disabled={apagando} accessibilityRole="button"
            accessibilityState={{ disabled: apagando }} style={styles.cancelar}>
            <Text style={styles.text}>{m.comum_cancelar()}</Text>
          </Pressable>
          <Pressable onPress={apagar} disabled={apagando || !n} accessibilityRole="button" accessibilityLabel={m.worktree_lote_apagar_n({ n })}
            accessibilityState={{ busy: apagando, disabled: apagando || !n }} style={[styles.botao, !n && styles.desligado]}>
            <Text style={styles.botaoTexto}>{m.worktree_lote_apagar_n({ n })}</Text>
          </Pressable>
        </View>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create((theme) => ({
  inner: { padding: theme.base.space[4], gap: theme.base.space[2] },
  title: { fontSize: theme.base.text.lg, fontWeight: '600', color: theme.tokens.text.primary },
  flex: { flex: 1, minWidth: 0 },
  item: { gap: 2 },
  marcavel: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[2], minHeight: 44 },
  switch: { minHeight: 44 },
  nome: { fontWeight: '600', color: theme.tokens.text.primary },
  text: { color: theme.tokens.text.primary },
  muted: { color: theme.tokens.text.secondary },
  aviso: { color: theme.tokens.status.warning },
  ok: { color: theme.tokens.status.success },
  arquivo: { color: theme.tokens.text.secondary, fontFamily: theme.base.fontMono, fontSize: theme.base.text.sm, paddingLeft: theme.base.space[3] },
  erro: { color: theme.tokens.status.error },
  acoes: { flexDirection: 'row', justifyContent: 'flex-end', gap: theme.base.space[2], marginTop: theme.base.space[2] },
  cancelar: { minHeight: 44, paddingHorizontal: theme.base.space[4], justifyContent: 'center', borderRadius: theme.base.radius.md,
              borderWidth: 1, borderColor: theme.tokens.border.default },
  botao: { minHeight: 44, paddingHorizontal: theme.base.space[4], justifyContent: 'center',
           borderRadius: theme.base.radius.md, backgroundColor: theme.tokens.status.error },
  desligado: { opacity: 0.4 },
  botaoTexto: { color: '#fff', fontWeight: '600' },
}));
