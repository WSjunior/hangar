import { memo, useMemo, type ReactNode } from 'react';
import { ActionSheetIOS, Platform, Pressable, Text, View } from 'react-native';
import { MenuView, type NativeActionEvent } from '@react-native-menu/menu';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import * as Haptics from 'expo-haptics';
import { cwdParts, isOrq, loopBadge, providerName, relativeTime, rotuloEstado, untrackedReason, worktreeLabel, type AggSession, type State } from '@hangar/core';
import { Chip, type Tone } from '../../ui/Chip';
import { HangarMark } from '../../ui/HangarMark';
import { Icon } from '../../ui/Icon';
import { superficie } from '../../theme/superficie';
import { useWorktreeStatus } from '../worktrees/worktreeStatus';
import * as m from '../../paraglide/messages';

// LOOP_TONE_COLOR do core é CSS var (`var(--accent)`) — não serve em RN; o tom vira `Chip tone`.
const TOM_DO_LOOP: Record<'ok' | 'warn' | 'attention' | 'muted', Tone> = {
  ok: 'success',
  warn: 'warning',
  attention: 'error',
  muted: 'neutral',
};

// A marca é a única pista visual de estado na linha: lê a cor da pílula do tema, que acompanha
// tema claro/escuro e o acento escolhido na Aparência.
const PILL_DO_ESTADO: Record<State, 'working' | 'idle' | 'input' | 'dead'> = {
  working: 'working',
  idle: 'idle',
  awaiting_input: 'input',
  dead: 'dead',
};

interface Props {
  session: AggSession;
  mostrarServidor: boolean; // false quando a lista já está agrupada por servidor
  // recebem a sessão: a lista passa a mesma função a todas as linhas, e o memo segura o render
  onPress: (s: AggSession) => void;
  onGit: (s: AggSession) => void;
  onExcluir: (s: AggSession) => void;
  onRenomear: (s: AggSession) => void;
  onResume: (s: AggSession) => void;
  // Ausente em servidor de convite: o backend recusa worktrees ao convidado, e o chip é só texto.
  onWorktree?: (s: AggSession, path: string) => void;
}

// O iOS já vibra ao abrir o menu de contexto; o PopupMenu do Android não.
const vibrarAoAbrir =
  Platform.OS === 'android' ? () => void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium).catch(() => {}) : undefined;

export const SessionRow = memo(function SessionRow({ session: s, mostrarServidor, onPress, onGit, onExcluir, onRenomear, onResume, onWorktree }: Props) {
  const { theme } = useUnistyles();
  // O orquestrador não se renomeia nem se fecha: o backend recusa, então a linha nem oferece.
  const orq = isOrq(s);
  // Sem o arrasto, o menu é o único caminho visível pro Git: o orquestrador fica só com ele.
  const acoesMenu = useMemo(
    () => [
      ...(orq ? [] : [{ id: 'rename', title: m.sessao_renomear(), image: Platform.select({ ios: 'pencil', android: 'ic_menu_edit' }) }]),
      ...(s.cwd ? [{ id: 'git', title: 'Git', image: Platform.select({ ios: 'arrow.triangle.branch' }) }] : []),
      ...(orq ? [] : [{
        id: 'delete',
        title: m.sessao_excluir_curto(),
        image: Platform.select({ ios: 'trash', android: 'ic_menu_delete' }),
        attributes: { destructive: true },
      }]),
    ],
    [s.cwd, orq],
  );
  const acao = (nome: string) => {
    if (nome === 'rename') onRenomear(s);
    else if (nome === 'git') onGit(s);
    else if (nome === 'delete') onExcluir(s);
  };
  // Renomear/Git/Excluir moram no toque longo, gesto que o leitor de tela não alcança. Aqui elas viram ações do rotor/menu de acessibilidade da própria linha.
  const acoesA11y = useMemo(
    () => [
      ...(orq ? [] : [{ name: 'rename', label: m.sessao_renomear() }]),
      ...(s.cwd ? [{ name: 'git', label: 'Git' }] : []),
      ...(orq ? [] : [{ name: 'delete', label: m.sessao_excluir_curto() }]),
    ],
    [s.cwd, orq],
  );
  const untracked = s.tracked === false;
  const cwd = cwdParts(s.cwd);
  const loop = loopBadge(s.loop_status, s.loop_iter, s.loop_max);
  const pendingQuestions = s.pending_questions ?? 0;
  const sub = s.question ?? (s.state === 'working' ? s.label : null) ?? null;
  const peers = s.pair_peers ?? [];
  // o rótulo do grupo (ex.: o ticket) diz mais que o nome do par; sem ele, o par ou o tamanho
  const pairLabel = peers.length ? (s.pair_task ?? (peers.length === 1 ? peers[0] : String(peers.length + 1))) : null;
  const meta = [
    orq ? null : providerName(s.provider),
    s.engine,
    s.cwd && cwd.base !== s.name ? cwd.base : null,
    s.branch,
    pairLabel,
  ].filter((p): p is string => !!p);
  const wtPath = s.worktree_path ?? (s.worktree ? s.cwd ?? null : null);
  const wtNome = worktreeLabel(s);
  const juntada = useWorktreeStatus(s.serverId, wtPath)?.merged === true;
  const corEstado = theme.tokens.pill[PILL_DO_ESTADO[s.state]].fg;
  const muted = theme.tokens.text.muted;

  // iOS: o menu de contexto (MenuView) engolia o toque simples da linha dentro da gaveta; a folha de
  // ações nativa abre pelo próprio toque longo e não disputa o toque. Android segue com o PopupMenu.
  const abrirAcoesIOS = () => {
    void Haptics.impactAsync(Haptics.ImpactFeedbackStyle.Medium).catch(() => {});
    ActionSheetIOS.showActionSheetWithOptions(
      {
        title: s.name,
        options: [...acoesMenu.map((a) => a.title), m.comum_cancelar()],
        cancelButtonIndex: acoesMenu.length,
        destructiveButtonIndex: acoesMenu.some((a) => a.id === 'delete') ? acoesMenu.findIndex((a) => a.id === 'delete') : undefined,
      },
      (i) => { if (i < acoesMenu.length) acao(acoesMenu[i].id); },
    );
  };
  const comMenu = (linha: ReactNode) =>
    acoesMenu.length === 0 || Platform.OS === 'ios' ? linha : (
      <MenuView
        shouldOpenOnLongPress
        actions={acoesMenu}
        onPressAction={({ nativeEvent }: NativeActionEvent) => acao(nativeEvent.event)}
        onOpenMenu={vibrarAoAbrir}
      >
        {linha}
      </MenuView>
    );

  return comMenu(
    <Pressable
      onPress={() => onPress(s)}
      // no-op de propósito: soltar o dedo depois do toque longo que abriu o menu não abre a conversa
      onLongPress={acoesMenu.length === 0 ? undefined : Platform.OS === 'ios' ? abrirAcoesIOS : () => {}}
      // kimi sem id é estado NORMAL pré-1º prompt, e codex sem thread ainda está no startup:
      // a tela da conversa espera o vínculo sem herdar transcript de outra sessão
      disabled={untracked && s.provider !== 'kimi' && s.provider !== 'codex'}
      style={({ pressed }) => [styles.row, pressed && { backgroundColor: superficie(theme, 0.6) }]}
      accessibilityRole="button"
      // rótulo composto: um label explícito no pai faz o RN descartar o texto dos filhos, e o
      // estado e a pergunta sumiriam do leitor de tela.
      // Máquina e projeto entram mesmo quando a tela os esconde (lista agrupada, pasta = nome):
      // é o que distingue duas conversas de mesmo nome no leitor de tela.
      accessibilityLabel={`${s.name}, ${rotuloEstado(s.state)}${pendingQuestions > 0 ? `, ${m.ask_perguntas()}: ${pendingQuestions}` : ''}${sub ? `, ${sub}` : ''}, ${providerName(s.provider)}, ${s.serverLabel}${s.cwd ? `, ${cwd.base}` : ''}${s.branch ? `, ${s.branch}` : ''}`}
      accessibilityActions={acoesA11y}
      onAccessibilityAction={({ nativeEvent }) => {
        acao(nativeEvent.actionName);
      }}
    >
      <View style={styles.lead}><HangarMark size={20} color={corEstado} /></View>
      <View style={styles.col}>
        <View style={styles.linha}>
          <Text style={[styles.nome, { color: theme.tokens.text.primary }]} numberOfLines={1}>{s.name}</Text>
          {pendingQuestions > 0 ? <Chip tone="warning">{`? ${pendingQuestions}`}</Chip> : null}
          {orq ? <Chip>{m.orq_row_badge()}</Chip> : null}
          {untracked ? <Chip tone="warning">{m.sessao_sem_id()}</Chip> : null}
          <Text style={[styles.tempo, { color: muted }]} numberOfLines={1}>{relativeTime(s.last_activity)}</Text>
        </View>
        {/* O que a sessão está fazendo: a pergunta (cor de input), a atividade (itálico) ou o estado. */}
        <Text
          style={[
            styles.sub,
            s.question
              ? { color: theme.tokens.pill.input.fg }
              : sub
                ? { color: theme.tokens.text.secondary, fontStyle: 'italic' }
                : { color: muted },
          ]}
          numberOfLines={1}
        >
          {sub ?? rotuloEstado(s.state)}
        </Text>
        {/* Uma linha discreta de contexto, sem ícones: provedor · máquina · pasta · ramo · par.
            Diff fica na tela de Git; a pasta some quando repete o nome da sessão. A pasta da
            worktree vem logo abaixo, num chip próprio que abre a folha dela. */}
        {meta.length || mostrarServidor ? (
          <Text style={[styles.meta, { color: muted }]} numberOfLines={1}>
            {mostrarServidor ? <Text style={{ color: s.serverColor }}>{s.serverLabel}{meta.length ? ' · ' : ''}</Text> : null}
            {meta.join(' · ')}
          </Text>
        ) : null}
        {s.worktree_gone ? (
          <Text style={[styles.meta, { color: muted }]} numberOfLines={1}>{m.worktree_apagada()}</Text>
        ) : wtNome && wtPath && onWorktree ? (
          <Pressable onPress={() => onWorktree(s, wtPath)} accessibilityRole="button"
            accessibilityLabel={`${m.sessao_worktree()}: ${wtNome}`} hitSlop={8} style={styles.wt}>
            <Chip icon="GitBranch" mono>{`${wtNome}${juntada ? ' ✓' : ''}`}</Chip>
          </Pressable>
        ) : wtNome && wtPath ? (
          <View style={styles.wt}><Chip icon="GitBranch" mono>{wtNome}</Chip></View>
        ) : null}
        {s.limited || loop ? (
          <View style={styles.chips}>
            {s.limited ? <Chip tone="warning" icon="Hourglass">{s.limit_reset ?? ''}</Chip> : null}
            {loop ? <Chip tone={TOM_DO_LOOP[loop.tone]}>{loop.label}</Chip> : null}
          </View>
        ) : null}
        {untracked && s.provider !== 'kimi' && s.provider !== 'pi' && s.provider !== 'omp' ? (
          <Pressable onPress={() => onResume(s)} style={styles.resume} accessibilityRole="button" accessibilityLabel={m.sessao_retomar()}>
            <Icon name="RotateCw" size={12} color={theme.tokens.text.secondary} />
            <Text style={[styles.metaTxt, { color: theme.tokens.text.primary }]}>{m.sessao_retomar()}</Text>
          </Pressable>
        ) : untracked ? (
          <Text style={[styles.metaTxt, { color: muted }]}>{untrackedReason(s.provider)}</Text>
        ) : null}
      </View>
    </Pressable>,
  );
});

const styles = StyleSheet.create((theme) => ({
  row: { flexDirection: 'row', alignItems: 'flex-start', gap: 12, minHeight: 56, paddingVertical: 10, paddingHorizontal: 12, borderRadius: theme.base.radius.lg },
  // a marca alinha com a linha do nome, não com o centro do bloco
  lead: { width: 22, alignItems: 'center', paddingTop: 1 },
  col: { flex: 1, gap: 2, minWidth: 0 },
  linha: { flexDirection: 'row', alignItems: 'center', gap: 6, minWidth: 0 },
  nome: { fontSize: theme.base.text.base, lineHeight: 22, fontWeight: '600', flexShrink: 1 },
  tempo: { marginLeft: 'auto', paddingLeft: 4, flexShrink: 0, fontSize: theme.base.text.xs, fontVariant: ['tabular-nums'] },
  sub: { fontSize: theme.base.text.sm, lineHeight: 20 },
  meta: { fontSize: theme.base.text.xs, lineHeight: 18 },
  metaTxt: { fontSize: theme.base.text.xs },
  chips: { flexDirection: 'row', flexWrap: 'wrap', gap: 4, marginTop: 4 },
  wt: { alignSelf: 'flex-start', marginTop: theme.base.space[1] },
  resume: { flexDirection: 'row', alignItems: 'center', gap: 4, marginTop: 4, minHeight: 32, alignSelf: 'flex-start' },
}));
