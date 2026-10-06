// @vitest-environment happy-dom
import { act, createElement, type ReactNode } from 'react';
import { createRoot } from 'react-dom/client';
import { afterEach, beforeEach, describe, expect, it, vi } from 'vitest';
import type { FirstConversationAttempt, MotivoFim } from '@hangar/core';

(globalThis as typeof globalThis & { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

vi.mock('react-native', async (original) => ({
  ...await original<object>(),
  AppState: { currentState: 'active', addEventListener: () => ({ remove: () => {} }) },
  AccessibilityInfo: { sendAccessibilityEvent: vi.fn() },
  Pressable: ({ accessibilityState, accessibilityRole, accessibilityLabel, accessibilityHint, onPress, children, disabled, ref, style }: {
    accessibilityState?: { disabled?: boolean; busy?: boolean; expanded?: boolean }; accessibilityRole?: string;
    accessibilityLabel?: string; onPress?: () => void; children: ReactNode; disabled?: boolean;
    accessibilityHint?: string; ref?: import('react').Ref<HTMLButtonElement>; style?: unknown;
  }) => createElement('button', {
    ref, role: accessibilityRole, 'aria-label': accessibilityLabel, title: accessibilityHint, onClick: onPress, disabled,
    style: [typeof style === 'function' ? style({ pressed: false }) : style].flat(Infinity)
      .reduce<Record<string, unknown>>((all, part) => (part && typeof part === 'object' ? { ...all, ...part } : all), {}),
    'aria-disabled': accessibilityState?.disabled, 'aria-busy': accessibilityState?.busy,
    'aria-expanded': accessibilityState?.expanded,
  }, children),
}));

const focusNavigation = vi.hoisted(() => ({
  listeners: new Map<string, Set<(event: { data: { closing: boolean } }) => void>>(),
  focused: true,
}));
const nativeNavigation = {
  isFocused: () => focusNavigation.focused,
  addListener: (name: string, listener: (event: { data: { closing: boolean } }) => void) => {
    const listeners = focusNavigation.listeners.get(name) ?? new Set();
    listeners.add(listener);
    focusNavigation.listeners.set(name, listeners);
    return () => { listeners.delete(listener); };
  },
};
const routerPush = vi.hoisted(() => vi.fn());
const navigation = vi.hoisted(() => ({ back: vi.fn(), replace: vi.fn(), canGoBack: true }));
const route = vi.hoisted(() => ({ params: { server: 's1', name: 'sess' }, segments: ['s'] }));
const realFirstInput = vi.hoisted(() => ({ enabled: false, send: vi.fn(), history: vi.fn(), transcribe: vi.fn(),
  upload: vi.fn(async () => ({ path: '/up/sess/ditado-1.m4a' })) }));
const voiceInput = vi.hoisted(() => ({ onFim: null as null | ((file: File, reason: MotivoFim, uri: string) => Promise<void>) }));
vi.mock('expo-router', () => ({
  useNavigation: () => nativeNavigation,
  useRouter: () => ({ push: routerPush, back: navigation.back, replace: navigation.replace, canGoBack: () => navigation.canGoBack }),
  useLocalSearchParams: () => route.params, useSegments: () => route.segments,
}));
vi.mock('@hangar/core', async (original) => ({
  ...await original<typeof import('@hangar/core')>(),
  fetchSessionsForServer: async () => [{ name: 'sess', provider: 'claude' }],
  sendInputForServer: realFirstInput.send,
  getHistory: realFirstInput.history,
  transcribeUploadedForServer: realFirstInput.transcribe,
  // O ditado sobe o áudio por aqui (`audioOnly`) antes de transcrever.
  uploadFileForServer: realFirstInput.upload,
}));
vi.mock('../stores/servers', () => {
  const state = { ready: true, servers: [{ id: 's1' }, { id: 's2' }], ensureActive: () => true };
  return { useServers: Object.assign((select: (s: typeof state) => unknown) => select(state), { getState: () => state }) };
});
vi.mock('../ui/Screen', () => ({ Screen: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
// A gaveta puxa o gesture-handler nativo, que o vitest não carrega; aqui só o conteúdo importa.
vi.mock('../features/sessions/SessionsDrawer', () => ({ SessionsDrawer: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
vi.mock('../features/sessions/ServerSheet', () => ({ ServerSheet: () => null }));
vi.mock('react-native-keyboard-controller', () => ({ KeyboardAvoidingView: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
vi.mock('./LoopChip', () => ({ LoopChip: () => null }));
vi.mock('./OrqFooter', () => ({ OrqFooter: () => null }));
vi.mock('./TuiPill', () => ({ TuiPill: () => null }));
vi.mock('./RecarregarPill', () => ({ RecarregarPill: () => null }));
vi.mock('./PendingPlan', () => ({ PendingPlan: () => null }));
vi.mock('./SessionProblem', () => ({ SessionProblem: () => null }));
vi.mock('./SessionPickerSheet', () => ({ SessionPickerSheet: () => null }));
vi.mock('../features/create/CreateSessionSheet', () => ({ CreateSessionSheet: () => createElement('div', { 'data-create': true }) }));
vi.mock('../ui/Icon', () => ({ Icon: () => null }));
vi.mock('../ui/HangarMark', () => ({ HangarMark: () => null }));
const sheetEvents = vi.hoisted(() => ({ presented: undefined as (() => void) | undefined, dismissed: undefined as (() => void) | undefined }));
vi.mock('../ui/Sheet', () => ({ Sheet: ({ children, onDidPresent, onDismiss }: { children: ReactNode; onDidPresent?: () => void; onDismiss?: () => void }) => {
  sheetEvents.presented = onDidPresent;
  sheetEvents.dismissed = onDismiss;
  return createElement('div', null, children);
} }));
const panelEvents = vi.hoisted(() => ({ dismissed: undefined as (() => void) | undefined }));
vi.mock('../ui/AnchoredPanel', () => ({ AnchoredPanel: ({ open, onDismissed, children }: { open: boolean; onDismissed?: () => void; children: ReactNode }) => {
  panelEvents.dismissed = onDismissed;
  return open ? createElement('div', null, children) : null;
} }));
// A folha de anexo também é um Sheet; fora daqui para o `sheetEvents` seguir preso à folha de opções.
vi.mock('../ui/AttachSheet', () => ({ AttachSheet: () => null }));
vi.mock('../features/usage/HomeUsageCard', () => ({ HomeUsageCard: () => null, TextTabs: () => null }));
vi.mock('../features/sessions/StatePill', () => ({ StatePill: () => null }));
vi.mock('./ContextRing', () => ({ ContextRing: () => null }));
// Cada chave devolve o próprio nome, como nos outros testes de componente do app.
vi.mock('../paraglide/messages', () => Object.fromEntries(
  ('arq_aba askq_sua_resposta bastao_dossie_sub bastao_dossie_titulo chat_voltar_sessoes codex_limites_titulo ctx_anexos ctx_atividade ctx_grupo ctx_limites ctx_repositorio ctx_terminal modo_so_ociosa more_fotos_videos_arquivos more_tarefas_agentes navbar_mais_acoes par_titulo recarregar_sessao recarregar_sessao_detalhe sessao_trocar_de term_titulo '
    + 'askq_enviando board_arquivo board_imagem board_remover_anexo codex_orientar composer_anexar_arquivo composer_desfazer_limpeza composer_ditado_limpo composer_enviando_cancelar composer_enviar_mensagem composer_fila_acao composer_fila_aria composer_fila_contagem composer_gravando_audio composer_gravar_audio composer_mandando_grupo composer_mandar_grupo composer_mandar_tambem composer_mensagem composer_parar composer_parar_gravacao composer_pro_grupo composer_pros_dois composer_sessao_trabalhando composer_transcrevendo_audio composer_transcrever_de_novo')
    .concat(' composer_mic_style_hint composer_dictation_style composer_session_settings composer_session_settings_hint ditado_estilo_titulo uso_aria codex_orientar_ajuda')
    .concat(' composer_interromper_claude composer_interromper_msg composer_interromper comum_cancelar')
    .concat(' permissao_pedido comum_cancelar msg_aria_mensagens chat_plan_proposto composer_falha_envio nova_conversa_envio_incerto nova_conversa_resultado_salvar_erro nova_conversa_salvar_erro')
    .concat(' askq_enviando board_falha_envio board_falha_upload chat_chegou_mas chat_envio_incerto chat_nao_chegou_em chat_servidor_removido codex_orientar_recebido codex_orientar_sem_envio composer_ditado_anterior composer_ditado_aplicado composer_ditado_indisponivel composer_ditado_interrompido composer_ditado_recuperavel composer_draft_read_again composer_draft_recover_attach_busy composer_falha_gravacao composer_falha_transcricao composer_fila_erro composer_sem_acesso_fotos composer_sem_acesso_mic composer_submission_check composer_submission_rejected composer_submission_sending composer_transcrever_de_novo composer_transcricao_vazia composer_aguarde_transcricao')
    .concat(' draft_read_error draft_invalid draft_write_error draft_clear_error composer_draft_previous composer_draft_recover composer_draft_discard composer_draft_read_again')
    .concat(' sessao_nova nova_conversa_placeholder nova_conversa_sem_destino nova_conversa_opcoes nova_conversa_opcoes_fechar nova_conversa_destino_hint nova_conversa_config_hint nova_conversa_enviar criar_criando')
    .concat(' composer_mensagem_para comandos_titulo native_new_chat_title native_empty_chat_hint')
    .concat(' uso_titulo uso_vazio uso_secao_cota uso_secao_conversa uso_secao_numeros uso_statusline uso_custo uso_tempo_sessao uso_linha_projeto uso_reset composer_modelo ctx_contexto stats_faixa_aria')
    // Telas do redesign (Nova conversa, folha de anexo, lista de tarefas, uso na inicial).
    .concat(' ask_perguntas chat_abrir_terminal_codex chat_carregando_historico chat_codex_buffering chat_historico_antigo chat_sem_historico_anterior chat_sem_thread_codex chat_sem_thread_codex_hint chat_sessao_encerrada chat_sse_recusado chat_sse_tentar composer_adicionar_ao_chat composer_adicionar_arquivos composer_camera composer_fotos composer_sem_acesso_camera comum_falha_envio_opcao comum_voltar criar_retomar ctx_anexos_da_sessao')
    .concat(' home_usage_30d home_usage_7d home_usage_active_days home_usage_activity home_usage_all home_usage_cost home_usage_day home_usage_empty home_usage_load_failed home_usage_method home_usage_model_count home_usage_models home_usage_overview home_usage_partial home_usage_period_unsupported home_usage_sessions home_usage_today home_usage_tokens home_usage_top_model home_usage_warming home_usage_warming_timeout lista_tentar_novamente')
    .concat(' native_action_uncertain native_ask_fallback native_close native_dictation_active native_dictation_cancel native_dictation_level native_loading native_new_chat_folder native_tasks_all_done native_tasks_completed native_tasks_expand native_tasks_in_progress native_tasks_left native_tasks_left_1 native_tasks_minimize native_tasks_pending native_tasks_untitled native_thinking native_tools_failed native_tools_failed_1 native_tree_thoughts')
    .concat(' new_conversation_no_received_messages new_conversation_received_messages notice_compacted notice_hook_prompt notice_interrupted notice_skill_loaded nova_conversa_abrir nova_conversa_adotar nova_conversa_conferir nova_conversa_descartar nova_conversa_guardada nova_conversa_reenviar')
    .concat(' native_working_line pensamento_vivo tool_executando_ha estado_em_execucao atividade_subagente tool_abrir_agente tool_fase_escrevendo')
    .concat(' stats_cache stats_chamadas stats_chamadas_1 stats_io stats_llm stats_toks stats_toks_now stats_toks_recent stats_tools stats_ttft stats_turnos stats_turnos_1 sync_retry uso_janela_30d uso_janela_5h uso_janela_7d').split(' ').map((k) => [k, () => k]),
));

// Rascunho em memória no lugar do MMKV; cada teste começa sem nada guardado.
const storage = vi.hoisted(() => ({ memory: new Map<string, string>(), failSet: false, failRemove: false }));
vi.mock('react-native-mmkv', () => ({
  createMMKV: () => ({
    getString: (key: string) => storage.memory.get(key),
    set: (key: string, value: string) => {
      if (storage.failSet) throw new Error('storage unavailable');
      storage.memory.set(key, value);
    },
    remove: (key: string) => {
      if (storage.failRemove) throw new Error('storage unavailable');
      storage.memory.delete(key);
    },
    getNumber: () => undefined, getBoolean: () => undefined, contains: (key: string) => storage.memory.has(key),
  }),
}));
beforeEach(() => { storage.memory.clear(); storage.failSet = false; storage.failRemove = false; sessionsState.rows = []; realFirstInput.enabled = false; });

const sessionsState = vi.hoisted(() => ({ rows: [] as { serverId: string; name: string; jsonl?: string | null }[], byServerRecord: {} }));

// Composer isolado: sem picker, pills, ditado nem store real — só o que decide o botão Parar.
const composerChat = vi.hoisted(() => ({ state: 'idle' as string, send: vi.fn(async (_text: string) => {}) }));
vi.mock('expo-image-picker', () => ({}));
vi.mock('@react-native-menu/menu', () => ({ MenuView: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
vi.mock('expo-document-picker', () => ({}));
vi.mock('./draftAttachments', () => ({
  retainDraftAttachment: vi.fn(async (attachment: import('../stores/drafts').DraftAttachment) => attachment),
  removeDraftAttachment: vi.fn(),
}));
vi.mock('../ui/Glass', () => ({ Glass: ({ children }: { children: ReactNode }) => createElement('div', null, children) }));
vi.mock('../ui/MultilineInput', () => ({ MultilineInput: ({ value, onChangeText, accessibilityLabel, ref }: {
  value: string; onChangeText: (text: string) => void; accessibilityLabel?: string; ref?: import('react').Ref<HTMLTextAreaElement>;
}) => createElement('textarea', { ref, 'aria-label': accessibilityLabel, value, readOnly: true, onInput: (e: { currentTarget: { value: string } }) => onChangeText(e.currentTarget.value) }),
}));
vi.mock('../features/pills/PillMenu', () => ({ PillMenu: () => null }));
vi.mock('../features/ditado/EstiloPill', () => ({ DictationStyleMenu: () => null, useDictationStyleLabel: () => 'estilo' }));
vi.mock('./SessionSettings', () => ({ SessionSettingsButton: () => createElement('button', { 'aria-label': 'composer_session_settings' }) }));
vi.mock('../features/ditado/useDitado', () => ({ useDitado: (callbacks: { onFim: typeof voiceInput.onFim }) => {
  voiceInput.onFim = callbacks.onFim;
  return { gravando: false, rms: 0, iniciar: () => {}, parar: () => {} };
} }));
vi.mock('../features/ditado/ditadoEstiloStore', () => ({ useDitadoEstiloStore: { getState: () => ({ pronto: false }) } }));
vi.mock('./CommandSheet', () => ({
  CommandSheet: () => null,
  useSessionCommands: () => ({ commands: [], error: '', retry: () => {} }),
}));
vi.mock('./SlashSuggest', () => ({ SlashSuggest: () => null }));
vi.mock('./SideQuestionSheet', () => ({ SideQuestionSheet: () => null }));
vi.mock('../stores/sessions', () => ({
  useSessions: Object.assign((sel: (s: unknown) => unknown) => sel(sessionsState), { getState: () => sessionsState }),
}));
vi.mock('../stores/chat', async (original) => {
  const actual = await original<typeof import('../stores/chat')>();
  const snap = () => ({ pending: [], events: [], stateEvent: { state: composerChat.state } });
  const use = Object.assign((sel: (s: unknown) => unknown) => sel(snap()), {
    getState: snap, setState: () => {}, subscribe: () => () => {},
  });
  return { ...actual,
    chatStore: (serverId: string, name: string) => realFirstInput.enabled ? actual.chatStore(serverId, name)
      : { use, send: composerChat.send, retain: () => {}, release: () => {}, retry: () => {} },
    filaCount: (...args: Parameters<typeof actual.filaCount>) => realFirstInput.enabled ? actual.filaCount(...args) : 0,
    isSubmitting: (serverId: string, name: string) => realFirstInput.enabled && actual.isSubmitting(serverId, name),
  };
});

const firstInput = vi.hoisted(() => ({
  attempt: null as FirstConversationAttempt | null,
  send: vi.fn<(serverId: string, id: string) => Promise<void>>(),
  confirm: vi.fn(),
}));
vi.mock('../stores/newConversation', async (original) => {
  const actual = await original<typeof import('../stores/newConversation')>();
  return { ...actual,
    useNewConversation: Object.assign((select: (state: ReturnType<typeof actual.useNewConversation.getState>) => unknown) => realFirstInput.enabled ? actual.useNewConversation(select) : select({
      attempts: firstInput.attempt ? { s1: firstInput.attempt } : {}, issues: {}, busy: {},
    }), { getState: () => realFirstInput.enabled ? actual.useNewConversation.getState()
      : { attempts: firstInput.attempt ? { s1: firstInput.attempt } : {}, issues: {}, busy: {} } }),
    readFirstInput: (serverId: string, name: string) => realFirstInput.enabled ? actual.readFirstInput(serverId, name) : firstInput.attempt?.serverId === serverId
      && firstInput.attempt.sessionName === name ? firstInput.attempt : null,
    confirmFirstInput: (id: string) => realFirstInput.enabled ? actual.confirmFirstInput(id) : firstInput.confirm(id),
    sendFirstInput: (serverId: string, id: string) => realFirstInput.enabled ? actual.sendFirstInput(serverId, id) : firstInput.send(serverId, id),
    restoreAttempt: (serverId: string) => realFirstInput.enabled ? actual.restoreAttempt(serverId) : null,
  };
});

// Lista isolada: a bolha só registra o texto recebido, pra provar o que chega nela.
const bubbleTexts = vi.hoisted(() => [] as string[]);
vi.mock('./AssistantBubble', () => ({ AssistantBubble: ({ text }: { text: string }) => { bubbleTexts.push(text); return null; } }));
vi.mock('./UserBubble', () => ({ UserBubble: () => null }));
// Anexos da bolha puxam módulos nativos (gesture-handler, expo-audio, expo-file-system) que o node não carrega.
vi.mock('./ImageThumb', () => ({ ImageThumb: () => null }));
vi.mock('./AudioChip', () => ({ AudioChip: () => null }));
vi.mock('../features/attachments/DocumentViewer', () => ({ DocumentViewer: () => null }));
vi.mock('../features/attachments/mediaCache', () => ({ canShareFile: false, shareFile: async () => {} }));
vi.mock('../ui/Toast', () => ({ toast: { ok: () => {}, erro: () => {} } }));
vi.mock('./PreviewBubble', () => ({ PreviewBubble: () => null }));
vi.mock('./ThinkingBlock', () => ({ ThinkingBlock: () => null }));
vi.mock('./tools/ToolCard', () => ({ ToolCard: () => null }));
vi.mock('./tools/ToolGroup', () => ({ ToolGroup: () => null }));
vi.mock('./tools/ToolDetailSheet', () => ({ ToolDetailSheet: () => null }));
vi.mock('../stores/aparencia', () => ({ useAparencia: (sel: (s: unknown) => unknown) => sel({ pensamentoTools: 'busca' }) }));
vi.mock('@legendapp/list/react-native', () => ({
  LegendList: ({ data, renderItem, keyExtractor, ListFooterComponent }: {
    data: unknown[]; renderItem: (a: { item: unknown }) => ReactNode; keyExtractor: (i: unknown) => string; ListFooterComponent?: ReactNode;
  }) => createElement('div', null, ...data.map((item) => createElement('div', { key: keyExtractor(item) }, renderItem({ item }))), ListFooterComponent),
}));

vi.mock('react-native-enriched-markdown', () => ({
  EnrichedMarkdownText: ({ markdown }: { markdown: string }) => createElement('pre', null, markdown),
}));
vi.mock('./TableChart', () => ({ TableChart: () => null }));
vi.mock('./tableChartPref', () => ({ getTableChartPref: () => 'table', setTableChartPref: () => {} }));
vi.mock('./BubbleActions', () => ({ BubbleActions: () => null, pararTts: () => {} }));
vi.mock('./ArquivoChip', () => ({
  ArquivoChip: ({ caminho, onPress }: { caminho: string; onPress: () => void }) =>
    createElement('button', { onClick: onPress, 'aria-label': caminho }, caminho),
}));

import { ChatHeader } from './ChatHeader';
import { MessageList } from './MessageList';
import { MoreSheet } from './MoreSheet';
import { Composer } from './Composer';
import { OptionButtons } from './OptionButtons';
import ChatScreen from '../../app/s/[server]/[name]/index';
import CreateRoute from '../../app/create';
import { NewConversation } from '../features/create/NewConversation';
import { _resetNewConversationForTests, recoverAttempt, restoreAttempt, useNewConversation } from '../stores/newConversation';
import { AccessibilityInfo, Alert } from 'react-native';
import { retainDraftAttachment } from './draftAttachments';

async function render(el: ReturnType<typeof createElement>) {
  const container = document.createElement('div');
  const root = createRoot(container);
  await act(async () => root.render(el));
  return { container, root };
}
const button = (container: HTMLElement, label: string) => [...container.querySelectorAll('button, [role="button"]')]
  .find((el) => el.textContent === label || el.getAttribute('aria-label') === label) as HTMLElement | undefined;

const header = { name: 'g1-orq', state: null, onBack: () => {}, onMore: () => {}, onTitlePress: () => {} };
const sheet = { open: true, onClose: () => {}, serverId: 's1', name: 'g1-orq' };

it.each([0, 1])('opção %s e cancelar compartilham trava, mantendo índice 1-based e liberando no erro', async (selected) => {
  let fail!: (reason: Error) => void;
  const onSelect = vi.fn(() => new Promise<void>((_, reject) => { fail = reject; }));
  const onCancel = vi.fn(async () => {});
  const { container, root } = await render(createElement(OptionButtons, {
    question: 'permission', options: ['Yes', 'No'], onSelect, onCancel,
  }));
  const buttons = container.querySelectorAll<HTMLButtonElement>('button');
  act(() => { buttons[selected].click(); buttons[1 - selected].click(); buttons[2].click(); });
  expect(onSelect).toHaveBeenCalledExactlyOnceWith(selected + 1);
  expect(onCancel).not.toHaveBeenCalled();
  expect([...buttons].every((b) => b.disabled)).toBe(true);
  await act(async () => fail(new Error('recusado')));
  await act(async () => buttons[2].click());
  expect(onCancel).toHaveBeenCalledTimes(1);
  act(() => root.unmount());
});

describe('terminal escondido', () => {
  it('cabeçalho não tem botão Terminal: ele mora só no "⋯"', async () => {
    const { container, root } = await render(createElement(ChatHeader, header));
    expect(container.querySelector('[aria-label="term_titulo"]')).toBeNull();
    expect(container.querySelector('[aria-label="navbar_mais_acoes"]')).not.toBeNull();
    act(() => root.unmount());
  });

  it('"⋯" do orquestrador não lista Terminal, resposta, anexos nem grupo', async () => {
    const { container, root } = await render(createElement(MoreSheet, { ...sheet, orq: true }));
    expect(container.querySelector('[aria-label="term_titulo"]')).toBeNull();
    expect(container.querySelector('[aria-label="par_titulo"]')).toBeNull();
    expect(container.querySelector('[aria-label="askq_sua_resposta"]')).toBeNull();
    expect(container.querySelector('[aria-label="ctx_anexos"]')).toBeNull();
    expect(container.querySelector('[aria-label="arq_aba"]')).not.toBeNull();
    act(() => root.unmount());
  });

  it('"⋯" de sessão comum continua listando Terminal, resposta e anexos', async () => {
    const { container, root } = await render(createElement(MoreSheet, { ...sheet, temPergunta: true }));
    expect(container.querySelector('[aria-label="term_titulo"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="askq_sua_resposta"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="ctx_anexos"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="par_titulo"]')).not.toBeNull();
    act(() => root.unmount());
  });

  it('"⋯" sem pergunta pendente não oferece "Sua resposta"; sem pane não oferece Terminal', async () => {
    const { container, root } = await render(createElement(MoreSheet, { ...sheet, semTerminal: true }));
    expect(container.querySelector('[aria-label="askq_sua_resposta"]')).toBeNull();
    expect(container.querySelector('[aria-label="term_titulo"]')).toBeNull();
    expect(container.querySelector('[aria-label="ctx_anexos"]')).not.toBeNull();
    act(() => root.unmount());
  });
});

// A tela de verdade (CreateSessionSheet) entrega as pílulas; aqui bastam botões que chamam os ganchos.
let folderOpener: HTMLButtonElement | null = null;
const newConversationProps = (server: object) => ({
  server: server as never,
  destination: null, destinationPending: false, providerLabel: 'Codex',
  folderPanel: () => createElement('div', null, 'painel-pastas'),
  topPills: (openFolder: (opener: never) => void) => createElement('button', {
    ref: (el: HTMLButtonElement | null) => { folderOpener = el; },
    onClick: () => openFolder(folderOpener as never),
  }, 'repo'),
  bottomPills: null,
  providerChip: (openOptions: () => void) => createElement('button', { onClick: openOptions }, 'Codex'),
  resume: null, notices: null, options: null,
  body: { cwd: '/repo', provider: 'codex' as const }, blocked: false,
});

it('opções e pasta da Nova conversa recebem foco e devolvem ao seletor que abriu, preservando o rascunho', async () => {
  firstInput.attempt = null;
  vi.mocked(AccessibilityInfo.sendAccessibilityEvent).mockClear();
  const { container, root } = await render(createElement(NewConversation, newConversationProps({ id: 's1' })));
  const field = container.querySelector('textarea')!;
  act(() => {
    field.value = 'primeira mensagem';
    field.dispatchEvent(new Event('input', { bubbles: true }));
  });
  // "Mais opções" vem do menu nativo do chip: a folha foca o Fechar ao abrir.
  act(() => button(container, 'Codex')!.click());
  expect(AccessibilityInfo.sendAccessibilityEvent).not.toHaveBeenCalled();
  act(() => sheetEvents.presented?.());
  const close = button(container, 'nova_conversa_opcoes_fechar')!;
  expect(AccessibilityInfo.sendAccessibilityEvent).toHaveBeenLastCalledWith(close, 'focus');
  act(() => close.click());
  act(() => sheetEvents.dismissed?.());
  expect(AccessibilityInfo.sendAccessibilityEvent).toHaveBeenCalledTimes(1);
  // O painel de pasta devolve o foco à pílula que o abriu.
  const opener = button(container, 'repo')!;
  act(() => opener.click());
  expect(container.textContent).toContain('painel-pastas');
  act(() => panelEvents.dismissed?.());
  expect(AccessibilityInfo.sendAccessibilityEvent).toHaveBeenLastCalledWith(opener, 'focus');
  expect(field.value).toBe('primeira mensagem');
  act(() => root.unmount());
});

describe('Parar no Composer', () => {
  const props = { serverId: 's1', name: 'sess' };

  it('ocioso: sem Parar, envio presente', async () => {
    composerChat.state = 'idle';
    const { container, root } = await render(createElement(Composer, { ...props, onStop: () => {} }));
    expect(container.querySelector('[aria-label="composer_parar"]')).toBeNull();
    expect(container.querySelector('[aria-label="composer_enviar_mensagem"]')).not.toBeNull();
    act(() => root.unmount());
  });

  it('trabalhando sem texto: Parar no lugar do envio, pede confirmação e só então chama onStop', async () => {
    composerChat.state = 'working';
    const onStop = vi.fn();
    const alert = vi.spyOn(Alert, 'alert');
    const { container, root } = await render(createElement(Composer, { ...props, onStop }));
    const parar = container.querySelector<HTMLButtonElement>('[aria-label="composer_parar"]');
    expect(parar).not.toBeNull();
    expect(container.querySelector('[aria-label="composer_enviar_mensagem"]')).toBeNull();
    act(() => parar!.click());
    expect(onStop).not.toHaveBeenCalled();
    const buttons = alert.mock.calls[0][2] as { style?: string; onPress?: () => void }[];
    act(() => buttons.find((b) => b.style === 'destructive')!.onPress!());
    expect(onStop).toHaveBeenCalledTimes(1);
    alert.mockRestore();
    act(() => root.unmount());
  });

  it('interrupção em voo: Parar desabilitado', async () => {
    composerChat.state = 'working';
    const { container, root } = await render(createElement(Composer, { ...props, onStop: () => {}, stopping: true }));
    expect(container.querySelector<HTMLButtonElement>('[aria-label="composer_parar"]')!.disabled).toBe(true);
    expect(container.querySelector('[aria-label="composer_parar"]')!.getAttribute('aria-busy')).toBe('true');
    act(() => root.unmount());
  });

  it('VoiceOver identifica o campo preenchido e o estado de envio; com texto não há Parar', async () => {
    composerChat.state = 'working';
    let finish!: () => void;
    composerChat.send.mockImplementationOnce(() => new Promise<void>((resolve) => { finish = resolve; }));
    const { container, root } = await render(createElement(Composer, { ...props, draft: 'texto', onStop: () => {} }));
    const send = container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!;
    expect(container.querySelector('[aria-label="composer_mensagem"]')).not.toBeNull();
    // Com texto a linha é Orientar + Enviar; o Parar volta quando o campo esvazia.
    expect(container.querySelector('[aria-label="composer_parar"]')).toBeNull();
    expect(send.getAttribute('aria-disabled')).toBe('false');
    act(() => send.click());
    expect(send.getAttribute('aria-disabled')).toBe('true');
    expect(send.getAttribute('aria-busy')).toBe('true');
    await act(async () => finish());
    expect(send.getAttribute('aria-busy')).toBe('false');
    act(() => root.unmount());
  });

  it('foco acessível volta ao campo só depois da folha sair, sem abrir teclado nem repetir no streaming', async () => {
    vi.mocked(AccessibilityInfo.sendAccessibilityEvent).mockClear();
    const { container, root } = await render(createElement(Composer, { ...props, draft: 'rascunho' }));
    const emit = (name: string, closing: boolean) => act(() => {
      focusNavigation.listeners.get(name)?.forEach((listener) => listener({ data: { closing } }));
    });
    emit('transitionEnd', false);
    expect(AccessibilityInfo.sendAccessibilityEvent).not.toHaveBeenCalled();
    focusNavigation.focused = false;
    emit('blur', true);
    emit('transitionEnd', true);
    expect(AccessibilityInfo.sendAccessibilityEvent).not.toHaveBeenCalled();
    focusNavigation.focused = true;
    emit('transitionEnd', false);
    expect(AccessibilityInfo.sendAccessibilityEvent).toHaveBeenCalledExactlyOnceWith(container.querySelector('textarea'), 'focus');
    await act(async () => root.render(createElement(Composer, { ...props, draft: 'rascunho' })));
    emit('transitionEnd', false);
    expect(AccessibilityInfo.sendAccessibilityEvent).toHaveBeenCalledTimes(1);
    act(() => root.unmount());
    expect([...focusNavigation.listeners.values()].every((listeners) => listeners.size === 0)).toBe(true);
  });

  it('sem onStop (quem não pode parar): nada de Parar mesmo trabalhando', async () => {
    composerChat.state = 'working';
    const { container, root } = await render(createElement(Composer, props));
    expect(container.querySelector('[aria-label="composer_parar"]')).toBeNull();
    act(() => root.unmount());
  });
});

describe('linha de botões do Composer', () => {
  it('campo em linha própria; +, microfone com a dica do estilo e um chip só de ajustes', async () => {
    composerChat.state = 'idle';
    const { container, root } = await render(createElement(Composer, { serverId: 's1', name: 'sess' }));
    const field = container.querySelector('[aria-label="composer_mensagem"]')!;
    const settings = container.querySelector('[aria-label="composer_session_settings"]')!;
    expect(settings).not.toBeNull();
    // O campo não divide a linha com os botões.
    expect(field.parentElement!.contains(settings)).toBe(false);
    expect(container.querySelector<HTMLButtonElement>('[aria-label="composer_gravar_audio"]')!.title).toBe('composer_mic_style_hint');
    expect(container.querySelector('[aria-label="composer_adicionar_ao_chat"]')).not.toBeNull();
    // Comandos e estilo do ditado moram no +, não na linha.
    expect(container.querySelector('[aria-label="comandos_titulo"]')).toBeNull();
    act(() => root.unmount());
  });

  it('trabalhando com texto sem terminal: a linha não muda — sem Orientar, com microfone e chip', async () => {
    composerChat.state = 'working';
    const { container, root } = await render(createElement(Composer, { serverId: 's1', name: 'sess', draft: 'texto', headless: true, onStop: () => {} }));
    expect(container.querySelector('[aria-label="codex_orientar"]')).toBeNull();
    expect(container.querySelector('[aria-label="composer_gravar_audio"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="composer_session_settings"]')).not.toBeNull();
    expect(container.querySelector('[aria-label="composer_enviar_mensagem"]')).not.toBeNull();
    act(() => root.unmount());
  });
});

describe('primeiro texto recuperado no Composer', () => {
  const props = { serverId: 's1', name: 'sess', draft: 'primeiro texto', firstInputId: 'attempt-1' };
  beforeEach(() => {
    composerChat.send.mockClear(); firstInput.confirm.mockClear(); firstInput.send.mockReset();
    firstInput.attempt = {
      id: 'attempt-1', serverId: 's1', body: { name: 'sess', cwd: '/repo', provider: 'codex' },
      sessionName: 'sess', text: 'primeiro texto', phase: 'created',
    };
  });

  it('montar/remontar não envia; dois toques explícitos fazem um envio à mesma sessão sem eco local', async () => {
    let done!: () => void;
    firstInput.send.mockImplementation(() => new Promise<void>((resolve) => { done = resolve; }));
    const first = await render(createElement(Composer, props));
    act(() => first.root.unmount());
    const { container, root } = await render(createElement(Composer, props));
    expect(container.querySelector('textarea')!.value).toBe('primeiro texto');
    expect(firstInput.send).not.toHaveBeenCalled();
    const send = container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!;
    act(() => { send.click(); send.click(); });
    expect(firstInput.send).toHaveBeenCalledExactlyOnceWith('s1', 'attempt-1');
    expect(composerChat.send).not.toHaveBeenCalled();
    firstInput.attempt!.phase = 'sent';
    await act(async () => done());
    expect(firstInput.confirm).toHaveBeenCalledWith('attempt-1');
    act(() => root.unmount());
  });

  it('recusa conserva rascunho; resultado incerto não repete POST', async () => {
    firstInput.send.mockResolvedValue(undefined);
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(container.querySelector('textarea')!.value).toBe('primeiro texto');
    expect(firstInput.confirm).not.toHaveBeenCalled();
    firstInput.attempt!.phase = 'send_unknown';
    firstInput.send.mockClear();
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(firstInput.send).not.toHaveBeenCalled();
    expect(composerChat.send).not.toHaveBeenCalled();
    expect(container.querySelector('textarea')!.value).toBe('primeiro texto');
    expect(container.textContent).toContain('nova_conversa_envio_incerto');
    act(() => root.unmount());
  });

  it('ACK confirmado antes de o botão atualizar não repete o texto que ainda aparece', async () => {
    const { container, root } = await render(createElement(Composer, props));
    firstInput.attempt = null;
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(firstInput.send).not.toHaveBeenCalled();
    expect(composerChat.send).not.toHaveBeenCalled();
    expect(container.querySelector('textarea')!.value).toBe('');
    act(() => root.unmount());
  });

  it('ACK tardio preserva nova edição e sent não repete o primeiro input', async () => {
    let done!: () => void;
    firstInput.send.mockImplementation(() => new Promise<void>((resolve) => { done = resolve; }));
    const { container, root } = await render(createElement(Composer, props));
    act(() => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    const field = container.querySelector('textarea')!;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, 'texto novo');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    firstInput.attempt!.phase = 'sent';
    await act(async () => done());
    expect(field.value).toBe('texto novo');
    expect(firstInput.confirm).toHaveBeenCalledWith('attempt-1');
    act(() => root.unmount());
    const reopened = await render(createElement(Composer, props));
    // A edição guardada vence o texto de handoff que a rota ainda carrega; enviá-la é mensagem nova.
    expect(reopened.container.querySelector('textarea')!.value).toBe('texto novo');
    await act(async () => reopened.container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(firstInput.send).toHaveBeenCalledTimes(1);
    expect(composerChat.send).toHaveBeenCalledExactlyOnceWith('texto novo', expect.any(Number));
    act(() => reopened.root.unmount());
  });
});

describe('primeiro envio incerto com os stores reais', () => {
  const props = { serverId: 's1', name: 'recuperada', draft: 'primeiro texto', firstInputId: 'unknown-real' };
  afterEach(() => { vi.useRealTimers(); _resetNewConversationForTests(); });
  beforeEach(() => {
    realFirstInput.enabled = true;
    realFirstInput.send.mockReset().mockResolvedValue(undefined);
    realFirstInput.history.mockReset().mockResolvedValue([]);
    _resetNewConversationForTests();
    storage.memory.set('create.attempt.v1:s1', JSON.stringify({
      id: 'unknown-real', serverId: 's1', body: { name: 'recuperada', cwd: '/repo', provider: 'codex' },
      sessionName: 'recuperada', text: 'primeiro texto', phase: 'send_unknown',
    }));
    storage.memory.set('draft.v1:s1::recuperada', JSON.stringify({
      version: 1, text: 'primeiro texto', revision: 1, transcript: null, attachment: null,
      submission: { text: 'primeiro texto', draftRevision: 1, status: 'unknown' },
    }));
    restoreAttempt('s1');
  });

  it('Recuperar não envia nem descarta; Enviar uma vez remove tentativa e libera Nova conversa', async () => {
    let done!: () => void;
    realFirstInput.send.mockImplementationOnce(() => new Promise<void>((resolve) => { done = resolve; }));
    const { container, root } = await render(createElement(Composer, props));
    const send = container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!;
    await act(async () => send.click());
    expect(realFirstInput.send).not.toHaveBeenCalled();
    expect(container.textContent).toContain('nova_conversa_envio_incerto');
    act(() => button(container, 'composer_draft_recover')!.click());
    expect(realFirstInput.send).not.toHaveBeenCalled();
    expect(useNewConversation.getState().attempts.s1?.phase).toBe('send_unknown');
    expect(send.disabled).toBe(false);
    act(() => { send.click(); send.click(); });
    expect(realFirstInput.send).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 'recuperada', 'primeiro texto');
    expect(useNewConversation.getState().attempts.s1).toBeUndefined();
    expect(storage.memory.has('create.attempt.v1:s1')).toBe(false);
    await act(async () => done());
    expect(container.querySelector('textarea')!.value).toBe('');
    act(() => root.unmount());

    const creation = await render(createElement(NewConversation,
      newConversationProps({ id: 's1', label: 'S1', baseUrl: 'http://s1', token: 'fixture' })));
    act(() => {
      const field = creation.container.querySelector('textarea')!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, 'outra conversa');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    expect((button(creation.container, 'nova_conversa_enviar') as HTMLButtonElement).disabled).toBe(false);
    expect(realFirstInput.send).toHaveBeenCalledTimes(1);
    act(() => creation.root.unmount());
  });

  it('created que fica incerto no próprio toque não repete o POST nem abandona a tentativa', async () => {
    storage.memory.set('create.attempt.v1:s1', JSON.stringify({
      id: props.firstInputId, serverId: 's1', body: { name: props.name, cwd: '/repo', provider: 'codex' },
      sessionName: props.name, text: props.draft, phase: 'created',
    }));
    storage.memory.delete('draft.v1:s1::recuperada');
    _resetNewConversationForTests();
    restoreAttempt('s1');
    realFirstInput.send.mockRejectedValueOnce(new TypeError('offline'));
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(realFirstInput.send).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, props.name, props.draft);
    expect(useNewConversation.getState().attempts.s1?.phase).toBe('send_unknown');
    expect(container.textContent).toContain('nova_conversa_envio_incerto');
    expect(container.querySelector('textarea')!.value).toBe(props.draft);
    act(() => root.unmount());
  });

  it('envio automático após Recuperar não abandona send_unknown nem dispara POST', async () => {
    vi.useFakeTimers();
    realFirstInput.transcribe.mockResolvedValueOnce({ text: 'Execute a tarefa solicitada.', raw: '', aviso: null });
    const { container, root } = await render(createElement(Composer, props));
    act(() => button(container, 'composer_draft_recover')!.click());
    act(() => {
      const field = container.querySelector('textarea')!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, '');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_gravar_audio"]')!.click());
    await act(async () => voiceInput.onFim!(new File(['audio'], 'voice.m4a', { type: 'audio/mp4' }), 'silencio', 'file:///voice.m4a'));
    expect(container.textContent).toContain('composer_enviando_cancelar');
    await act(async () => vi.advanceTimersByTimeAsync(3250));
    expect(realFirstInput.send).not.toHaveBeenCalled();
    expect(useNewConversation.getState().attempts.s1?.phase).toBe('send_unknown');
    expect(storage.memory.has('create.attempt.v1:s1')).toBe(true);
    expect(container.textContent).toContain('nova_conversa_envio_incerto');
    act(() => root.unmount());
  });

  it.each(['inFlight', 'remove'])('abandono impedido por %s conserva tentativa e texto sem POST', async (failure) => {
    const { container, root } = await render(createElement(Composer, props));
    act(() => button(container, 'composer_draft_recover')!.click());
    let finish!: (events: []) => void;
    realFirstInput.history.mockImplementationOnce(() => new Promise<[]>((resolve) => { finish = resolve; }));
    const recovery = failure === 'inFlight' ? recoverAttempt('s1') : null;
    storage.failRemove = failure === 'remove';
    await act(async () => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    expect(realFirstInput.send).not.toHaveBeenCalled();
    expect(useNewConversation.getState().attempts.s1?.phase).toBe('send_unknown');
    expect(storage.memory.has('create.attempt.v1:s1')).toBe(true);
    expect(container.querySelector('textarea')!.value).toBe(props.draft);
    expect(container.textContent).toContain(failure === 'remove' ? 'nova_conversa_salvar_erro' : 'nova_conversa_envio_incerto');
    if (recovery) await act(async () => { finish([]); await recovery; });
    act(() => root.unmount());
  });

  it('ACK da nova ação preserva edição posterior e o destino original', async () => {
    let done!: () => void;
    realFirstInput.send.mockImplementationOnce(() => new Promise<void>((resolve) => { done = resolve; }));
    const { container, root } = await render(createElement(Composer, props));
    act(() => button(container, 'composer_draft_recover')!.click());
    act(() => container.querySelector<HTMLButtonElement>('[aria-label="composer_enviar_mensagem"]')!.click());
    act(() => {
      const field = container.querySelector('textarea')!;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, 'edição durante envio');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    await act(async () => done());
    expect(container.querySelector('textarea')!.value).toBe('edição durante envio');
    expect(JSON.parse(storage.memory.get('draft.v1:s1::recuperada')!).text).toBe('edição durante envio');
    expect(realFirstInput.send).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 'recuperada', 'primeiro texto');
    expect(storage.memory.has('draft.v1:s2::recuperada')).toBe(false);
    act(() => root.unmount());
  });
});

describe('rascunho guardado no Composer', () => {
  const props = { serverId: 's1', name: 'sess' };
  const type = (container: HTMLElement, value: string) => act(() => {
    const field = container.querySelector('textarea')!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, value);
    field.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const stored = (serverId: string, name: string) => {
    const raw = storage.memory.get(`draft.v1:${serverId}::${name}`);
    return raw ? JSON.parse(raw) as { text: string; revision: number; transcript: string | null } : null;
  };

  it.each(['sending', 'unknown', 'rejected'])('reabre upload confirmado com input %s sem reenviar; Recuperar devolve só o texto', async (status) => {
    composerChat.send.mockClear();
    const attachment = {
      uri: 'file:///draft-attachments/1-1.txt', name: 'nota.txt', mime: 'text/plain', kind: 'file',
      uploadedPath: '/uploads/nota.txt', uploadedFor: { serverId: 's1', name: 'sess', transcript: null },
    };
    storage.memory.set('draft.v1:s1::sess', JSON.stringify({
      version: 1, text: 'nota', revision: 1, transcript: null, attachment,
      submission: { text: 'nota — 📎 board_arquivo: /uploads/nota.txt', draftRevision: 1, status },
    }));
    const first = await render(createElement(Composer, props));
    act(() => first.root.unmount());
    const reopened = await render(createElement(Composer, props));
    expect(composerChat.send).not.toHaveBeenCalled();
    expect(reopened.container.textContent).toContain('nota.txt');
    act(() => button(reopened.container, 'composer_draft_recover')!.click());
    expect(reopened.container.querySelector('textarea')!.value).toBe('nota');
    expect(JSON.parse(storage.memory.get('draft.v1:s1::sess')!)).toMatchObject({ attachment, submission: null });
    act(() => reopened.root.unmount());
  });

  it('recuperação não troca anexo atual; depois de removê-lo adota o anterior sem reutilizar upload da sessão morta', async () => {
    const attachment = { uri: 'file:///draft-attachments/2-1.txt', name: 'atual.txt', mime: 'text/plain', kind: 'file' };
    const previous = { ...attachment, uri: 'file:///draft-attachments/1-1.txt', name: 'anterior.txt',
      uploadedPath: '/uploads/anterior.txt', uploadedFor: { serverId: 's1', name: 'sess', transcript: '/old' } };
    storage.memory.set('draft.v1:s1::sess', JSON.stringify({
      version: 1, text: 'atual', revision: 1, transcript: null, attachment, submission: null,
    }));
    storage.memory.set('draft.v1.recoverable:s1::sess', JSON.stringify({
      version: 1, text: 'anterior', revision: 1, transcript: '/old', attachment: previous, submission: null,
    }));
    const { container, root } = await render(createElement(Composer, props));
    act(() => button(container, 'composer_draft_recover')!.click());
    expect(container.textContent).toContain('composer_draft_recover_attach_busy');
    expect(container.querySelector('textarea')!.value).toBe('atual');
    expect(JSON.parse(storage.memory.get('draft.v1:s1::sess')!).attachment).toEqual(attachment);
    expect(storage.memory.has('draft.v1.recoverable:s1::sess')).toBe(true);
    act(() => container.querySelector<HTMLButtonElement>('[aria-label="board_remover_anexo"]')!.click());
    act(() => button(container, 'composer_draft_recover')!.click());
    expect(container.querySelector('textarea')!.value).toBe('atual\nanterior');
    expect(JSON.parse(storage.memory.get('draft.v1:s1::sess')!).attachment).toEqual({
      uri: previous.uri, name: previous.name, mime: previous.mime, kind: previous.kind,
    });
    expect(storage.memory.has('draft.v1.recoverable:s1::sess')).toBe(false);
    act(() => root.unmount());
  });

  it('cada edição grava na conversa de origem; desmontar não apaga e outro servidor com mesmo nome não recebe', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/a.jsonl' }];
    const first = await render(createElement(Composer, props));
    type(first.container, 'o');
    type(first.container, 'oi');
    expect(stored('s1', 'sess')).toMatchObject({ text: 'oi', revision: 2, transcript: '/t/a.jsonl' });
    act(() => first.root.unmount());
    expect(stored('s1', 'sess')?.text).toBe('oi');

    const other = await render(createElement(Composer, { serverId: 's2', name: 'sess' }));
    expect(other.container.querySelector('textarea')!.value).toBe('');
    act(() => other.root.unmount());
    const reopened = await render(createElement(Composer, props));
    expect(reopened.container.querySelector('textarea')!.value).toBe('oi');
    act(() => reopened.root.unmount());
  });

  it('texto devolvido pelo cancelar das opções é adotado e guardado na mesma conversa', async () => {
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => root.render(createElement(Composer, { ...props, draft: 'resposta cancelada' })));
    expect(container.querySelector('textarea')!.value).toBe('resposta cancelada');
    expect(stored('s1', 'sess')?.text).toBe('resposta cancelada');
    act(() => root.unmount());
  });

  it('texto devolvido pelo Parar entra antes do digitado, e o mesmo texto devolvido de novo entra de novo', async () => {
    const adopted = vi.fn();
    const { container, root } = await render(createElement(Composer, { ...props, onReturnedAdopted: adopted }));
    type(container, 'digitado');
    await act(async () => root.render(createElement(Composer, { ...props, returned: { text: 'pendente' }, onReturnedAdopted: adopted })));
    expect(container.querySelector('textarea')!.value).toBe('pendente\n\ndigitado');
    await act(async () => root.render(createElement(Composer, { ...props, returned: { text: 'pendente' }, onReturnedAdopted: adopted })));
    expect(container.querySelector('textarea')!.value).toBe('pendente\n\npendente\n\ndigitado');
    expect(adopted).toHaveBeenCalledTimes(2);
    expect(stored('s1', 'sess')?.text).toBe('pendente\n\npendente\n\ndigitado');
    act(() => root.unmount());
  });

  it('primeiro jsonl associa o rascunho provisório sem tratar como sessão recriada', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: null }];
    const { container, root } = await render(createElement(Composer, props));
    type(container, 'antes do transcript');
    expect(stored('s1', 'sess')?.transcript).toBeNull();
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/first.jsonl' }];
    await act(async () => root.render(createElement(Composer, props)));
    expect(container.querySelector('textarea')!.value).toBe('antes do transcript');
    expect(stored('s1', 'sess')?.transcript).toBe('/t/first.jsonl');
    expect(container.textContent).not.toContain('composer_draft_previous');
    act(() => root.unmount());
  });

  it('sessão recriada não recebe o texto antigo; ele sobrevive a texto novo e reabertura até recuperar', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/old.jsonl' }];
    const old = await render(createElement(Composer, props));
    type(old.container, 'texto da sessão morta');
    act(() => old.root.unmount());

    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/new.jsonl' }];
    const recreated = await render(createElement(Composer, props));
    expect(recreated.container.querySelector('textarea')!.value).toBe('');
    expect(recreated.container.textContent).toContain('composer_draft_previous');
    type(recreated.container, 'texto novo');
    expect(stored('s1', 'sess')).toMatchObject({ text: 'texto novo', transcript: '/t/new.jsonl' });
    act(() => recreated.root.unmount());

    const reopened = await render(createElement(Composer, props));
    expect(reopened.container.querySelector('textarea')!.value).toBe('texto novo');
    act(() => button(reopened.container, 'composer_draft_recover')!.click());
    expect(reopened.container.querySelector('textarea')!.value).toBe('texto novo\ntexto da sessão morta');
    expect(stored('s1', 'sess')?.text).toBe('texto novo\ntexto da sessão morta');
    expect(storage.memory.has('draft.v1.recoverable:s1::sess')).toBe(false);
    expect(reopened.container.textContent).not.toContain('composer_draft_previous');
    act(() => reopened.root.unmount());
  });

  it('sessão recriada enquanto aberta tira o texto antigo do campo e oferece recuperar; descartar não reoferece', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/old.jsonl' }];
    const { container, root } = await render(createElement(Composer, props));
    type(container, 'antigo');
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/new.jsonl' }];
    await act(async () => root.render(createElement(Composer, props)));
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(container.textContent).toContain('composer_draft_previous');
    act(() => button(container, 'composer_draft_discard')!.click());
    expect(container.textContent).not.toContain('composer_draft_previous');
    act(() => root.unmount());
    const reopened = await render(createElement(Composer, props));
    expect(reopened.container.textContent).not.toContain('composer_draft_previous');
    act(() => reopened.root.unmount());
  });

  it('"Ler de novo" após falha na recriação não traz o texto morto ao campo e Recuperar o devolve uma vez', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/old.jsonl' }];
    const { container, root } = await render(createElement(Composer, props));
    type(container, 'hello');
    storage.failSet = true;
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/new.jsonl' }];
    await act(async () => root.render(createElement(Composer, props)));
    expect(container.textContent).toContain('draft_write_error');
    storage.failSet = false;
    act(() => button(container, 'composer_draft_read_again')!.click());
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(container.textContent).toContain('composer_draft_previous');
    expect(stored('s1', 'sess')?.text).not.toBe('hello');
    act(() => button(container, 'composer_draft_recover')!.click());
    expect(container.querySelector('textarea')!.value).toBe('hello');
    act(() => root.unmount());
  });

  it('falha ao gravar aparece como aviso e o texto continua no campo', async () => {
    const { container, root } = await render(createElement(Composer, props));
    storage.failSet = true;
    type(container, 'sem espaço');
    expect(container.querySelector('textarea')!.value).toBe('sem espaço');
    expect(container.textContent).toContain('draft_write_error');
    act(() => root.unmount());
  });

  it('rascunho em formato inválido avisa sem travar e a edição seguinte grava por cima', async () => {
    storage.memory.set('draft.v1:s1::sess', '{"text":');
    const { container, root } = await render(createElement(Composer, props));
    expect(container.textContent).toContain('draft_invalid');
    type(container, 'recomeço');
    expect(stored('s1', 'sess')?.text).toBe('recomeço');
    act(() => root.unmount());
  });
});

describe('handoff nas rotas reais', () => {
  beforeEach(() => {
    route.params = { server: 's1', name: 'sess' }; route.segments = ['s'];
    navigation.back.mockClear(); navigation.replace.mockClear(); navigation.canGoBack = true;
    composerChat.send.mockClear(); firstInput.send.mockClear(); firstInput.confirm.mockClear();
    firstInput.attempt = {
      id: 'attempt-route', serverId: 's1', body: { name: 'sess', cwd: '/repo', provider: 'claude' },
      sessionName: 'sess', text: 'rascunho recuperável', phase: 'created',
    };
  });

  it('chat recebe rascunho sem POST e a troca de servidor não carrega o texto antigo', async () => {
    const { container, root } = await render(createElement(ChatScreen));
    expect(container.querySelector('textarea')!.value).toBe('rascunho recuperável');
    expect(firstInput.send).not.toHaveBeenCalled();
    expect(composerChat.send).not.toHaveBeenCalled();
    route.params = { server: 's2', name: 'sess' };
    await act(async () => root.render(createElement(ChatScreen)));
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(firstInput.confirm).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it('chat confirma sent sem retornar texto ao campo nem enviar novamente', async () => {
    firstInput.attempt!.phase = 'sent';
    const { container, root } = await render(createElement(ChatScreen));
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(firstInput.confirm).toHaveBeenCalledWith('attempt-route');
    expect(firstInput.send).not.toHaveBeenCalled();
    expect(composerChat.send).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it.each([false, true])('ACK que chega após o handoff limpa somente o snapshot, edição nova: %s', async (edited) => {
    const { container, root } = await render(createElement(ChatScreen));
    const field = container.querySelector('textarea')!;
    if (edited) act(() => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, 'outra mensagem');
      field.dispatchEvent(new Event('input', { bubbles: true }));
    });
    firstInput.attempt = { ...firstInput.attempt!, phase: 'sent' };
    await act(async () => root.render(createElement(ChatScreen)));
    expect(field.value).toBe(edited ? 'outra mensagem' : '');
    expect(composerChat.send).not.toHaveBeenCalled();
    expect(firstInput.send).not.toHaveBeenCalled();
    act(() => root.unmount());
  });

  it('Nova conversa desmonta o formulário ao perder a rota para impedir navegação tardia', async () => {
    route.segments = ['create'];
    const { container, root } = await render(createElement(CreateRoute));
    expect(container.querySelector('[data-create]')).not.toBeNull();
    route.segments = ['s'];
    await act(async () => root.render(createElement(CreateRoute)));
    expect(container.querySelector('[data-create]')).toBeNull();
    act(() => root.unmount());
  });

  it.each([true, false])('Nova conversa tem saída acessível com ou sem tela anterior: %s', async (canGoBack) => {
    route.segments = ['create']; navigation.canGoBack = canGoBack;
    const { container, root } = await render(createElement(CreateRoute));
    const cancel = container.querySelector<HTMLButtonElement>('[aria-label="comum_cancelar"]');
    expect(cancel).not.toBeNull();
    expect(cancel!.style.minHeight).toBe('44px');
    expect(cancel!.style.minWidth).toBe('44px');
    act(() => cancel!.click());
    if (canGoBack) {
      expect(navigation.back).toHaveBeenCalledTimes(1);
      expect(navigation.replace).not.toHaveBeenCalled();
    } else {
      expect(navigation.replace).toHaveBeenCalledExactlyOnceWith('/');
      expect(navigation.back).not.toHaveBeenCalled();
    }
    act(() => root.unmount());
  });
});

describe('plano proposto na lista', () => {
  it('Codex: a bolha recebe o texto original, com os marcadores, pra detectar o plano', async () => {
    bubbleTexts.length = 0;
    const text = 'Antes\n<proposed_plan>\n# Plano\n- passo\n</proposed_plan>';
    const { root } = await render(createElement(MessageList, {
      events: [{ id: 'a1', kind: 'assistant_msg', text }],
      preview: '', olderFailed: '', onLoadOlder: () => {},
      session: { provider: 'codex' } as never,
    } as never));
    expect(bubbleTexts).toEqual([text]);
    act(() => root.unmount());
  });
});

describe('bloco trabalhando no fim da lista', () => {
  it('texto e tempo do rótulo do terminal e subagente rodando grudado no fim', async () => {
    const { container, root } = await render(createElement(MessageList, {
      events: [{ id: 'u1', kind: 'user_msg', text: 'vai', ts: Date.now() / 1000 - 12 },
               { id: 'a1', kind: 'tool_use', tool_name: 'Agent', tool_use_id: 'tu1', ts: Date.now() / 1000 - 5, tool_input: { description: 'Revisar diff' } }],
      preview: '', olderFailed: '', onLoadOlder: () => {},
      stateEvent: { session: 'sess', state: 'working', label: 'Pensando… (3s · esc to interrupt)' },
      agentesRodando: [{ id: 'tu1', kind: 'agent', description: 'Revisar diff', running: true, subagentType: 'Explore' }],
    } as never));
    expect(container.textContent).toContain('Pensando…');
    expect(container.textContent).not.toContain('esc to interrupt');
    // O tempo do terminal vence o contado no app: vale mesmo com o envio fora da janela carregada.
    expect(container.textContent).toContain('3s');
    expect(container.textContent).not.toContain('12 s');
    expect(container.textContent).toContain('Revisar diff');
    expect(container.textContent).toContain('Explore');
    act(() => root.unmount());
  });

  it('raciocínio em voo ocupa o lugar da linha de spinner', async () => {
    const { container, root } = await render(createElement(MessageList, {
      events: [], preview: '', olderFailed: '', onLoadOlder: () => {},
      stateEvent: { session: 'sess', state: 'working', label: null },
      pensamento: 'considerando as opções',
    } as never));
    expect(container.textContent).toContain('pensamento_vivo');
    expect(container.textContent).toContain('considerando as opções');
    expect(container.textContent).not.toContain('native_working_line');
    act(() => root.unmount());
  });
});

it('plano real preserva prosa, links e abertura do arquivo sem tags do protocolo', async () => {
  const { AssistantBubble } = await vi.importActual<typeof import('./AssistantBubble')>('./AssistantBubble');
  routerPush.mockClear();
  const text = 'Prosa antes\n<proposed_plan>\n# Plano Android\n[Documentação](https://docs.example.test/android)\n[Arquivo do plano](/repo/docs/plano.md)\n</proposed_plan>\nProsa depois';
  const { container, root } = await render(createElement(AssistantBubble, { text, sessionName: 'sess', serverId: 'srv' }));
  const markdown = container.querySelector('pre')!.textContent;
  expect(markdown).toContain('Prosa antes');
  expect(markdown).toContain('Prosa depois');
  expect(markdown).toContain('[Documentação](https://docs.example.test/android)');
  expect(markdown).toContain('[Arquivo do plano](/repo/docs/plano.md)');
  expect(markdown).not.toContain('proposed_plan');
  expect(container.textContent).toContain('chat_plan_proposto');
  const chip = container.querySelector<HTMLButtonElement>('[aria-label="/repo/docs/plano.md"]');
  expect(chip).not.toBeNull();
  act(() => chip!.click());
  expect(routerPush).toHaveBeenCalledExactlyOnceWith('/s/srv/sess/files?path=%2Frepo%2Fdocs%2Fplano.md');
  act(() => root.unmount());
});

describe('ditado entregue à conversa de origem', () => {
  const props = { serverId: 's1', name: 'sess' };
  const type = (container: HTMLElement, value: string) => act(() => {
    const field = container.querySelector('textarea')!;
    Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, 'value')!.set!.call(field, value);
    field.dispatchEvent(new Event('input', { bubbles: true }));
  });
  const mic = (c: HTMLElement) => c.querySelector<HTMLButtonElement>('[aria-label="composer_gravar_audio"]')!;
  const audio = { uri: 'file:///doc/draft-attachments/1-1.m4a', name: 'ditado.m4a', mime: 'audio/m4a', kind: 'file' };
  const voiceRecord = (patch: Record<string, unknown> = {}) => ({
    version: 1, id: 'v1', audio, transcript: '/t/a.jsonl', draftRevision: 2, before: 'antes', motivo: 'botao',
    status: 'ready', text: 'ditado', raw: 'ditado cru', issue: '', ...patch,
  });
  const storeDraft = (patch: Record<string, unknown> = {}) => storage.memory.set('draft.v1:s1::sess', JSON.stringify({
    version: 1, text: 'antes', revision: 2, transcript: '/t/a.jsonl', attachment: null, submission: null, ...patch,
  }));
  beforeEach(() => { sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/a.jsonl' }]; });

  it('ditado pronto com o rascunho intacto entra sozinho no fim ao montar, sem botão de recuperar', async () => {
    storeDraft();
    storage.memory.set('draft.v1.dictation:s1::sess', JSON.stringify(voiceRecord()));
    const { container, root } = await render(createElement(Composer, props));
    expect(container.querySelector('textarea')!.value).toBe('antes ditado');
    expect(button(container, 'composer_draft_recover')).toBeUndefined();
    expect(storage.memory.has('draft.v1.dictation:s1::sess')).toBe(false);
    act(() => root.unmount());
  });

  it('rascunho mexido depois da gravação mantém o botão de recuperar e não mexe no campo', async () => {
    storeDraft({ text: 'antes editado', revision: 3 });
    storage.memory.set('draft.v1.dictation:s1::sess', JSON.stringify(voiceRecord()));
    const { container, root } = await render(createElement(Composer, props));
    expect(container.querySelector('textarea')!.value).toBe('antes editado');
    expect(button(container, 'composer_draft_recover')).toBeDefined();
    expect(storage.memory.has('draft.v1.dictation:s1::sess')).toBe(true);
    act(() => root.unmount());
  });

  it('sessão recriada com o mesmo nome não recebe o ditado sozinha', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/new.jsonl' }];
    storeDraft();
    storage.memory.set('draft.v1.dictation:s1::sess', JSON.stringify(voiceRecord()));
    const { container, root } = await render(createElement(Composer, props));
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(container.textContent).toContain('composer_ditado_recuperavel');
    expect(JSON.parse(storage.memory.get('draft.v1.dictation:s1::sess')!).status).toBe('ready');
    act(() => root.unmount());
  });

  it('trocar de conversa logo depois de parar ainda transcreve para a origem, e o texto entra ao voltar', async () => {
    realFirstInput.transcribe.mockClear().mockResolvedValueOnce({ path: '/up/ditado-1.m4a', text: 'ditado', raw: 'ditado', aviso: null });
    const first = await render(createElement(Composer, props));
    type(first.container, 'antes');
    await act(async () => mic(first.container).click());
    await act(async () => {
      const done = voiceInput.onFim!(new File(['a'], 'ditado.m4a', { type: 'audio/m4a' }), 'escondeu', 'file:///cache/ditado.m4a');
      first.root.unmount();
      await done;
    });
    expect(realFirstInput.transcribe).toHaveBeenCalledTimes(1);
    const back = await render(createElement(Composer, props));
    expect(back.container.querySelector('textarea')!.value).toBe('antes ditado');
    expect(button(back.container, 'composer_draft_recover')).toBeUndefined();
    act(() => back.root.unmount());
  });

  it('voltar no meio da transcrição mostra o microfone ocupado; o resultado entra ao voltar de novo', async () => {
    let finish!: (r: { path: string; text: string; raw: string; aviso: null }) => void;
    realFirstInput.transcribe.mockClear().mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const first = await render(createElement(Composer, props));
    type(first.container, 'antes');
    await act(async () => mic(first.container).click());
    await act(async () => { void voiceInput.onFim!(new File(['a'], 'ditado.m4a', { type: 'audio/m4a' }), 'botao', 'file:///cache/ditado.m4a'); });
    act(() => first.root.unmount());
    const back = await render(createElement(Composer, props));
    expect(mic(back.container).getAttribute('aria-busy')).toBe('true');
    expect(button(back.container, 'composer_transcrever_de_novo')).toBeUndefined();
    await act(async () => finish({ path: '/up/ditado-1.m4a', text: 'ditado', raw: 'ditado', aviso: null }));
    act(() => back.root.unmount());
    const again = await render(createElement(Composer, props));
    expect(again.container.querySelector('textarea')!.value).toBe('antes ditado');
    expect(realFirstInput.transcribe).toHaveBeenCalledTimes(1);
    act(() => again.root.unmount());
  });

  it('remontar durante a cópia do áudio: a cópia antiga não grava por cima do ditado novo no ar', async () => {
    let copied!: () => void;
    vi.mocked(retainDraftAttachment).mockImplementationOnce((a) => new Promise((resolve) => { copied = () => resolve(a); }));
    let finish!: (r: { path: string; text: string; raw: string; aviso: null }) => void;
    realFirstInput.transcribe.mockClear().mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    const first = await render(createElement(Composer, props));
    await act(async () => mic(first.container).click());
    await act(async () => { void voiceInput.onFim!(new File(['a'], 'a.m4a', { type: 'audio/m4a' }), 'escondeu', 'file:///cache/a.m4a'); });
    act(() => first.root.unmount());
    const second = await render(createElement(Composer, props));
    await act(async () => mic(second.container).click());
    await act(async () => { void voiceInput.onFim!(new File(['b'], 'b.m4a', { type: 'audio/m4a' }), 'botao', 'file:///cache/b.m4a'); });
    await act(async () => copied());
    expect(JSON.parse(storage.memory.get('draft.v1.dictation:s1::sess')!).audio.uri).toBe('file:///cache/b.m4a');
    expect(realFirstInput.transcribe).toHaveBeenCalledTimes(1);
    await act(async () => finish({ path: '/up/sess/b.m4a', text: 'b', raw: 'b', aviso: null }));
    act(() => second.root.unmount());
  });

  it('microfone de outra montagem não abre enquanto o POST da conversa está no ar', async () => {
    let finish!: (r: { path: string; text: string; raw: string; aviso: null }) => void;
    realFirstInput.transcribe.mockClear().mockImplementationOnce(() => new Promise((resolve) => { finish = resolve; }));
    // A segunda montagem nasce antes do ditado e não relê o armazenamento (o mock do chat não avisa).
    const second = await render(createElement(Composer, props));
    const first = await render(createElement(Composer, props));
    await act(async () => mic(first.container).click());
    await act(async () => { void voiceInput.onFim!(new File(['a'], 'a.m4a', { type: 'audio/m4a' }), 'botao', 'file:///cache/a.m4a'); });
    await act(async () => mic(second.container).click());
    expect(second.container.textContent).toContain('composer_aguarde_transcricao');
    await act(async () => finish({ path: '/up/sess/a.m4a', text: 'a', raw: 'a', aviso: null }));
    act(() => { first.root.unmount(); second.root.unmount(); });
  });

  it('"Transcrever de novo" usa o arquivo já enviado pelo nome, sem subir outra cópia', async () => {
    realFirstInput.upload.mockClear();
    realFirstInput.transcribe.mockClear().mockResolvedValueOnce({ path: '/up/sess/ditado-1.m4a', text: 'de novo', raw: 'de novo', aviso: null });
    storeDraft({ text: '', revision: 1 });
    storage.memory.set('draft.v1.dictation:s1::sess', JSON.stringify(voiceRecord({
      status: 'failed', text: '', raw: '', issue: '502: groq', draftRevision: 1, before: '', serverPath: '/up/sess/ditado-1.m4a',
    })));
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => button(container, 'composer_transcrever_de_novo')!.click());
    // Mesma conversa: nome solto, que o convidado também pode usar.
    expect(realFirstInput.transcribe).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 'sess', 'ditado-1.m4a', { limpar: true, estilo: undefined });
    expect(realFirstInput.upload).not.toHaveBeenCalled();
    expect(container.querySelector('textarea')!.value).toBe('de novo');
    act(() => root.unmount());
  });

  it('falha da transcrição deixa o "de novo" apontando para o arquivo já enviado', async () => {
    realFirstInput.upload.mockClear();
    realFirstInput.transcribe.mockClear().mockRejectedValueOnce(Object.assign(new Error('502: groq'), { status: 502 }));
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => mic(container).click());
    await act(async () => voiceInput.onFim!(new File(['a'], 'ditado.m4a', { type: 'audio/m4a' }), 'botao', 'file:///cache/ditado.m4a'));
    expect(realFirstInput.upload).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 'sess', expect.any(File), { audioOnly: true });
    expect(JSON.parse(storage.memory.get('draft.v1.dictation:s1::sess')!)).toMatchObject({
      status: 'failed', serverPath: '/up/sess/ditado-1.m4a', issue: '502: groq',
    });
    expect(button(container, 'composer_transcrever_de_novo')).toBeDefined();
    act(() => root.unmount());
  });

  it('depois de /clear, o "de novo" pelo caminho absoluto guardado volta com "Recuperar", sem entrar sozinho', async () => {
    sessionsState.rows = [{ serverId: 's1', name: 'sess', jsonl: '/t/clear.jsonl' }];
    realFirstInput.upload.mockClear();
    realFirstInput.transcribe.mockClear().mockResolvedValueOnce({ path: '/up/sess/ditado-1.m4a', text: 'antigo', raw: 'antigo', aviso: null });
    storeDraft({ text: '', revision: 1, transcript: '/t/clear.jsonl' });
    storage.memory.set('draft.v1.dictation:s1::sess', JSON.stringify(voiceRecord({
      status: 'failed', text: '', raw: '', issue: '502: groq', draftRevision: 1, before: '', serverPath: '/up/sess/ditado-1.m4a',
    })));
    const { container, root } = await render(createElement(Composer, props));
    await act(async () => button(container, 'composer_transcrever_de_novo')!.click());
    expect(realFirstInput.transcribe).toHaveBeenCalledExactlyOnceWith({ id: 's1' }, 'sess', '/up/sess/ditado-1.m4a', { limpar: true, estilo: undefined });
    expect(realFirstInput.upload).not.toHaveBeenCalled();
    expect(container.querySelector('textarea')!.value).toBe('');
    expect(container.textContent).toContain('composer_ditado_recuperavel');
    expect(button(container, 'composer_draft_recover')).toBeDefined();
    act(() => root.unmount());
  });
});
