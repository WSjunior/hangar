import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Pressable, Text, View, useWindowDimensions, type NativeScrollEvent, type NativeSyntheticEvent } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { LegendList, type LegendListRef } from '@legendapp/list/react-native';
import { Icon } from '../ui/Icon';
import { superficie } from '../theme/superficie';
import { UserBubble } from './UserBubble';
import { AssistantBubble } from './AssistantBubble';
import { PreviewBubble } from './PreviewBubble';
import { ToolGroup } from './tools/ToolGroup';
import { ToolCard } from './tools/ToolCard';
import { AgentesRodando, PensamentoVivo, WorkingLine } from './LiveWork';
import { turnStart } from './liveWork';
import { ToolDetailSheet, type ToolDetailHandle } from './tools/ToolDetailSheet';
import { foldConversation, type ConversationRow } from './tools/fold';
import { HtmlPageCard } from './tools/HtmlPageCard';
import { agruparConversa, entraNoPensamento, foldTasks, hexParaRgb, htmlPageFromResult, planDisplayText, type AgentRun, type ChatEvent, type SessionInfo, type StateEvent } from '@hangar/core';
import { useAparencia } from '../stores/aparencia';
import { TaskList } from './TaskList';
import type { PendingMsg } from './pending';
import * as m from '../paraglide/messages';

// Lista de bolhas do chat. A janela de render da PWA (WINDOW=120 eventos montados) aqui é
// a virtualização nativa do LegendList: ele só monta o visível + buffer, então o store
// pode manter os events completos e a lista continua barata — mesmo objetivo, menos código.
//
// onStartReached dispara loadOlder (busca o histórico anterior sob demanda); os avisos
// olderFailed ficam no rodapé da lista (réguas: falha não some calada).

interface Props {
  events: ChatEvent[];
  preview: string;
  previewMd?: boolean; // prévia do agente = markdown (default true = comportamento antigo)
  previewFull?: boolean;
  session?: SessionInfo | null;
  olderFailed: '' | 'failed' | 'unjoinable';
  onLoadOlder: () => void;
  pending?: PendingMsg[];
  optionsSlot?: ReactNode;
  sessionName?: string;
  serverId?: string;
  // Altura da caixa que flutua por cima do fim da lista: o último item rola até ficar acima dela.
  bottomInset?: number;
  // Bloco "trabalhando" do fim (ausente = lista só de leitura, como a do subagente).
  stateEvent?: StateEvent | null;
  turnSeen?: number | null;
  pensamento?: string;
  ferramenta?: { nome: string; input: Record<string, unknown> } | null;
  // Subagentes rodando de verdade, pelo fold de atividade: o cartão deles sai do meio da conversa
  // e fica grudado no fim até acabarem.
  agentesRodando?: AgentRun[];
  onAbrirAgentes?: () => void;
}

const SEM_AGENTES: AgentRun[] = [];

// Bolha sem texto não vira item nenhum, salvo a de imagem colada no terminal (só `image_count`).
// O tool_result é descartado pelo agruparConversa (entra na linha do tool_use pareado).
function visivel(ev: ChatEvent): boolean {
  if (ev.kind === 'user_msg') return !!ev.text || !!ev.image_count;
  if (ev.kind === 'assistant_msg') return !!ev.text;
  return true;
}

const SEM_TAREFAS: ReturnType<typeof foldTasks> = [];
const ehTask = (ev: ChatEvent) => ev.kind === 'tool_use' && (ev.tool_name === 'TaskCreate' || ev.tool_name === 'TaskUpdate');

// Com a Lista de tarefas ligada, as chamadas de tarefa saem da conversa e o bloco entra ONDE a
// última delas aconteceu: logo depois da linha que tem o evento anterior a ela (como o web).
function comTarefas(rows: ConversationRow[], eventos: ChatEvent[]): ConversationRow[] {
  let ancora: string | null = null;
  let tem = false;
  for (let i = eventos.length - 1; i >= 0 && !tem; i--) {
    if (!ehTask(eventos[i])) continue;
    tem = true;
    for (let j = i - 1; j >= 0; j--) {
      if (!ehTask(eventos[j]) && eventos[j].kind !== 'tool_result') { ancora = eventos[j].id; break; }
    }
  }
  if (!tem) return rows;
  const pos = ancora === null ? -1
    : rows.findIndex((r) => (r.type === 'event' || r.type === 'page' ? r.ev.id === ancora : r.type === 'fold' && r.parts.some((p) => p.id === ancora)));
  const out = [...rows];
  out.splice(pos + 1, 0, { type: 'tasks', id: 'tasks' });
  return out;
}

export function MessageList({
  events,
  preview,
  previewMd = true,
  previewFull = false,
  session,
  olderFailed,
  onLoadOlder,
  pending = [],
  optionsSlot,
  sessionName,
  serverId,
  bottomInset = 0,
  stateEvent = null,
  turnSeen = null,
  pensamento = '',
  ferramenta = null,
  agentesRodando = SEM_AGENTES,
  onAbrirAgentes,
}: Props) {
  const codex = session?.provider === 'codex';
  const visiblePreview = codex ? planDisplayText(preview) : preview;
  const detail = useRef<ToolDetailHandle>(null);
  const pref = useAparencia((s) => s.pensamentoTools);
  const look = useAparencia((s) => s.ferramentas);
  const tarefasLigadas = useAparencia((s) => s.tarefas);
  const folha = useAparencia((s) => (s.leitura === 'sheet' ? s.solidezFolha : null));
  const { theme } = useUnistyles();
  const { width } = useWindowDimensions();
  const results = useMemo(() => {
    const mapa = new Map<string, ChatEvent>();
    for (const e of events) if (e.kind === 'tool_result' && e.tool_use_id) mapa.set(e.tool_use_id, e);
    return mapa;
  }, [events]);
  // As três referências abaixo precisam ser ESTÁVEIS, e isso não é zelo: a prévia do streaming
  // muda a cada token, o que re-renderiza este componente dezenas de vezes por segundo. Com
  // `renderItem` novo a cada render, a virtualização não segura nada — toda linha visível é
  // refeita a cada token, em vez de só o rodapé, que é onde a prévia mora. Os itens são `memo`
  // pelo mesmo motivo; sem as duas metades, qualquer uma sozinha não adianta.
  const resultDe = useCallback((t: ChatEvent) => results.get(t.tool_use_id ?? '') ?? null, [results]);
  // Layout do nativo: o que vem entre duas mensagens (raciocínio, chamadas, grupos) vira UM trecho
  // dobrado. O agrupamento continua o do core; o foldConversation só junta o desenho.
  const tasks = useMemo(
    () => (tarefasLigadas ? foldTasks(events, (id) => results.get(id)) : SEM_TAREFAS),
    [tarefasLigadas, events, results],
  );
  const rodandoIds = useMemo(() => new Set(agentesRodando.map((a) => a.id)), [agentesRodando]);
  const inicioDoAgente = useCallback((id: string) => {
    const ts = events.find((e) => e.kind === 'tool_use' && e.tool_use_id === id)?.ts;
    return ts ? ts * 1000 : null;
  }, [events]);
  const data = useMemo(() => {
    const vis = events.filter((e) => visivel(e) && !(tarefasLigadas && ehTask(e))
      && !(e.kind === 'tool_use' && rodandoIds.has(e.tool_use_id ?? '')));
    // Página só com resultado de sucesso; sem sessão (lista do subagente) não há de onde baixá-la.
    const pageOf = (ev: ChatEvent) => {
      const r = results.get(ev.tool_use_id ?? '');
      return sessionName && r && !r.is_error ? htmlPageFromResult(ev.tool_name, r.result) : null;
    };
    const rows = foldConversation(agruparConversa(vis, { entraNoPensamento: (n) => entraNoPensamento(pref, n) }), look === 'tree', pageOf);
    return tarefasLigadas && tasks.length ? comTarefas(rows, events) : rows;
  }, [events, results, sessionName, pref, look, tarefasLigadas, tasks.length, rodandoIds]);
  const abrirDetalhe = useCallback((ev: ChatEvent) => detail.current?.abrir(ev), []);
  const working = stateEvent?.state === 'working';
  const desde = useMemo(() => (working ? turnStart(events, turnSeen) : null), [working, events, turnSeen]);
  const ferramentaEv = useMemo<ChatEvent | null>(
    () => (ferramenta ? { kind: 'tool_use', id: 'ferramenta-viva', tool_name: ferramenta.nome, tool_input: ferramenta.input } : null),
    [ferramenta],
  );

  const renderItem = useCallback(({ item }: { item: ConversationRow }) => {
    switch (item.type) {
      case 'fold':
        return <ToolGroup parts={item.parts} resultOf={resultDe} onAbrir={abrirDetalhe} look={look} />;
      case 'tasks':
        return <TaskList tasks={tasks} />;
      case 'page':
        return <HtmlPageCard page={item.page} sessionName={sessionName ?? ''} serverId={serverId} />;
      case 'event': {
        const ev = item.ev;
        if (ev.kind === 'user_msg') {
          // fila durável (queued-*) aparece translúcida igual ao eco local até o real chegar
          if (ev.id.startsWith('queued-')) {
            return (
              <View style={styles.pending}>
                <UserBubble text={ev.text ?? ''} sessionName={sessionName} ts={ev.ts} />
              </View>
            );
          }
          return <UserBubble text={ev.text ?? ''} sessionName={sessionName} ts={ev.ts} eventId={ev.id} imageCount={ev.image_count ?? 0} />;
        }
        if (ev.kind === 'assistant_msg') {
          return <AssistantBubble text={ev.text ?? ''} sessionName={sessionName} serverId={serverId} ts={ev.ts} />;
        }
        if (ev.kind === 'notice') {
          // Código conhecido vira frase do idioma da tela; desconhecido mostra o que veio, pra um
          // aviso novo do harness não sumir calado.
          const conhecido = ev.text === 'interrupted' || ev.text === 'turn_aborted';
          const frase = conhecido ? m.notice_interrupted()
            : ev.text === 'compacted' ? m.notice_compacted()
            : ev.text === 'hook_prompt' ? [m.notice_hook_prompt(), ev.hook_error].filter(Boolean).join('\n')
            // ponytail: só a linha; abrir o SKILL.md no app nativo fica pra quando pedirem.
            : ev.text === 'skill_loaded' && ev.skill ? m.notice_skill_loaded({ name: ev.skill.name })
            : ev.text;
          return <Text style={styles.notice}>{frase}</Text>;
        }
        return null;
      }
      default: {
        const nunca: never = item;
        return nunca;
      }
    }
  }, [resultDe, abrirDetalhe, sessionName, serverId, look, tasks]);

  // "Ir pro fim" (como o web): aparece com mais de uma tela entre o que se vê e o fim. Mensagem que
  // chega nesse meio-tempo muda o botão pra aviso, senão a conversa anda sem a pessoa saber.
  const lista = useRef<LegendListRef>(null);
  const [longeDoFim, setLongeDoFim] = useState(false);
  const vistoAte = useRef(events.length);
  useEffect(() => {
    if (!longeDoFim) vistoAte.current = events.length;
  }, [longeDoFim, events.length]);
  const temNovas = longeDoFim && events.length > vistoAte.current;
  const onScroll = useCallback((e: NativeSyntheticEvent<NativeScrollEvent>) => {
    const { contentOffset, contentSize, layoutMeasurement } = e.nativeEvent;
    const falta = contentSize.height - contentOffset.y - layoutMeasurement.height;
    setLongeDoFim(falta > layoutMeasurement.height);
  }, []);
  const irProFim = useCallback(() => {
    setLongeDoFim(false);
    void lista.current?.scrollToEnd({ animated: true });
  }, []);

  // Largura da coluna (Aparência › Texto da conversa): abaixo de 100% a conversa estreita no meio.
  const recuo = ((width - 2 * theme.base.space[4]) * (1 - theme.conversa.coluna)) / 2;
  const elevado = hexParaRgb(theme.tokens.bg.elevated);

  return (
    <View style={styles.area}>
    {/* Leitura Folha: a folha atrás da conversa; a Solidez da folha diz quanto ela tapa o fundo. */}
    {folha !== null && elevado ? (
      <View
        pointerEvents="none"
        style={[styles.folha, { backgroundColor: `rgba(${elevado.join(',')},${folha})`, left: recuo + theme.base.space[2], right: recuo + theme.base.space[2] }]}
      />
    ) : null}
    <LegendList
      ref={lista}
      onScroll={onScroll}
      scrollEventThrottle={100}
      data={data}
      keyExtractor={(i) => i.id}
      renderItem={renderItem}
      recycleItems={false}
      estimatedItemSize={72}
      alignItemsAtEnd
      initialScrollAtEnd
      maintainScrollAtEnd
      maintainVisibleContentPosition
      keyboardShouldPersistTaps="handled"
      // Sem isto o teclado aberto não tinha como fechar no chat.
      keyboardDismissMode="on-drag"
      onStartReached={onLoadOlder}
      onStartReachedThreshold={1}
      contentContainerStyle={[styles.content, recuo > 0 && { paddingHorizontal: theme.base.space[4] + recuo }]}
      // O histórico antigo carrega pelo topo: o aviso da falha fica lá, não perto do composer.
      ListHeaderComponent={
        olderFailed === 'failed' ? (
          <Text style={styles.gap} onPress={onLoadOlder} accessibilityRole="button">
            {m.chat_historico_antigo()}
          </Text>
        ) : olderFailed === 'unjoinable' ? (
          <Text style={styles.gap}>{m.chat_sem_historico_anterior()}</Text>
        ) : null
      }
      ListFooterComponent={
        <View style={styles.footer}>
          {/* Mesma ordem do fim da conversa na PWA: o trabalho em voo, depois o que a pessoa mandou. */}
          {pensamento ? <PensamentoVivo texto={pensamento} /> : null}
          {ferramentaEv ? <ToolCard use={ferramentaEv} result={null} onPress={abrirDetalhe} look={look} escrevendo /> : null}
          {visiblePreview ? <PreviewBubble text={visiblePreview} md={previewMd} full={previewFull} streaming={working} /> : null}
          {working && !pensamento && !ferramenta ? <WorkingLine label={stateEvent?.label} since={desde} /> : null}
          {agentesRodando.length ? <AgentesRodando agentes={agentesRodando} inicio={inicioDoAgente} onAbrir={onAbrirAgentes} /> : null}
          {pending.map((p) => (
            <View key={p.id} style={[styles.pending, p.solid && styles.pendingSolid]}>
              <UserBubble text={p.text} sessionName={sessionName} />
            </View>
          ))}
          {optionsSlot ?? null}
          {bottomInset > 0 ? <View style={{ height: bottomInset }} /> : null}
        </View>
      }
      accessibilityLabel={m.msg_aria_mensagens()}
    />
    {longeDoFim ? (
      <Pressable
        onPress={irProFim}
        style={[styles.toBottom, { bottom: bottomInset + theme.base.space[3] }, temNovas && { borderColor: theme.tokens.accent.base }]}
        accessibilityRole="button"
        accessibilityLabel={temNovas ? m.msg_novas_abaixo() : m.msg_ir_ultima()}
      >
        <Icon name="ChevronDown" size={20} color={temNovas ? theme.tokens.accent.base : theme.tokens.text.secondary} />
        {temNovas ? <View style={[styles.novasDot, { backgroundColor: theme.tokens.accent.base, borderColor: theme.tokens.bg.base }]} /> : null}
      </Pressable>
    ) : null}
    <ToolDetailSheet ref={detail} resultOf={resultDe} sessionName={sessionName} />
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  area: { flex: 1 },
  folha: { position: 'absolute', top: theme.base.space[2], bottom: 0, borderRadius: 16 },
  // Resposta sem bolha ocupa a largura: a margem lateral é o que separa o texto da borda da tela.
  content: {
    paddingHorizontal: theme.base.space[4],
    paddingVertical: theme.base.space[3],
    gap: theme.base.space[3],
  },
  footer: {
    gap: theme.base.space[3],
  },
  gap: {
    fontSize: theme.base.text.xs,
    color: theme.tokens.status.warning,
    textAlign: 'center',
    paddingVertical: theme.base.space[1],
    minHeight: 44,
    lineHeight: 44,
  },
  notice: {
    fontSize: theme.base.text.xs,
    color: theme.tokens.text.muted,
    textAlign: 'center',
    paddingVertical: theme.base.space[1],
  },
  pending: {
    opacity: 0.5,
  },
  pendingSolid: {
    opacity: 1,
  },
  toBottom: {
    position: 'absolute',
    right: theme.base.space[4],
    width: 40,
    height: 40,
    borderRadius: 20,
    borderWidth: 1,
    borderColor: theme.tokens.border.default,
    backgroundColor: superficie(theme, 0.9),
    alignItems: 'center',
    justifyContent: 'center',
  },
  novasDot: {
    position: 'absolute',
    top: -2,
    right: -2,
    width: 10,
    height: 10,
    borderRadius: 5,
    borderWidth: 2,
  },
}));
