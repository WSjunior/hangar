import { useCallback, useEffect, useMemo, useRef, useState, type ReactNode } from 'react';
import { Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { parseStatusLine } from '@hangar/core';
import { chatStore } from '../stores/chat';
import { AnchoredPanel } from '../ui/AnchoredPanel';
import { Icon, type IconName } from '../ui/Icon';
import * as m from '../paraglide/messages';
import { useModelControl } from '../features/pills/modelControl';
import { useEffortControl } from '../features/pills/effortControl';
import { usePermissionControl } from '../features/pills/permissionControl';
import { approvalShortLabel, useCodexApproval } from '../features/pills/codexApproval';
import { permissionLabel } from '../features/pills/permissionLabel';
import { reconcileChosen, type Chosen } from '../features/pills/pills';

interface Props {
  serverId: string;
  name: string;
  provider: string | null;
  headless: boolean;
  // `/model` e `/effort` abrem o painel: cada pedido novo leva um número maior.
  openRequest?: { which: 'model' | 'effort'; n: number } | null;
  // Some da linha sem desmontar o painel: com o Orientar na linha não há espaço, e o `/model`
  // digitado ainda precisa abri-lo.
  hidden?: boolean;
}

// O ícone do chip diz o modo; só o Planejar ganha nome e cor, porque muda o que a sessão faz.
const MODE_ICON: Record<string, IconName> = {
  manual: 'Hand', auto: 'Sparkles', acceptEdits: 'Pencil', plan: 'Diamond', bypassPermissions: 'Zap', dontAsk: 'ShieldOff',
};
const APPROVAL_ICON: Record<string, IconName> = { 'Ask for approval': 'Hand', 'Approve for me': 'Sparkles', 'Full Access': 'Zap' };

// Acima disso a lista de modelos ganha busca (LONG_LIST do controls.rs).
const LONG_LIST = 8;

// `effort_label` do controls.rs: nível conhecido traduzido, o resto capitalizado.
const LEVEL: Record<string, () => string> = {
  low: m.reasoning_level_low, medium: m.reasoning_level_medium, high: m.reasoning_level_high,
  xhigh: m.reasoning_level_xhigh, max: m.reasoning_level_max, ultra: m.reasoning_level_ultra,
};
const effortLabel = (lv: string) => LEVEL[lv.toLowerCase()]?.() ?? lv.charAt(0).toUpperCase() + lv.slice(1);

// Um chip só na linha do composer (modo no ícone, modelo e nível no texto) e um popover só com as
// três escolhas, o mesmo cartão do app de PC (render_ctl_panel).
export function SessionSettingsButton({ serverId, name, provider, headless, openRequest, hidden }: Props) {
  const { theme } = useUnistyles();
  const chat = chatStore(serverId, name);
  const statusLine = chat.use((s) => s.statusLine);
  const statusModel = useMemo(() => parseStatusLine(statusLine), [statusLine]);
  const [chosen, setChosen] = useState<Chosen>({});
  const [open, setOpen] = useState(false);
  const [query, setQuery] = useState('');
  const anchor = useRef<View>(null);
  const close = useCallback(() => setOpen(false), []);
  // Solta o modelo otimista quando a statusline confirma: troca feita no terminal volta a aparecer.
  useEffect(() => {
    setChosen((cur) => {
      const next = reconcileChosen(statusModel, cur);
      return next.model === cur.model ? cur : next;
    });
  }, [statusModel]);

  const isCodex = provider === 'codex';
  const isClaude = !provider || provider === 'claude';
  const permission = usePermissionControl({ serverId, name, provider: isCodex ? 'codex' : isClaude ? 'claude' : null });
  const approval = useCodexApproval({ name, headless, enabled: isCodex });
  const model = useModelControl({ serverId, name, provider, chosen, onChosen: setChosen, close });
  const effort = useEffortControl({ serverId, name, provider, chosen, onChosen: setChosen, close });

  // Haiku: `/effort` não tem o que mostrar, o pedido é descartado em vez de abrir o painel à toa.
  // O Raciocínio fica fora da rolagem, então `/effort` já o encontra à vista.
  useEffect(() => {
    if (!openRequest?.n) return;
    if (openRequest.which === 'effort' && effort.hidden) return;
    setOpen(true);
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [openRequest?.n]);

  useEffect(() => {
    if (!open) return;
    void model.load();
    void effort.load();
    if (isCodex && headless) void approval.load();
    // Só ao abrir: recarregar a cada statusline nova piscaria a lista aberta.
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [open]);

  const plan = permission.current === 'plan';
  const icon = permission.current
    ? (isCodex && !plan ? (approval.current ? APPROVAL_ICON[approval.current] : undefined) : MODE_ICON[permission.current])
    : undefined;
  const tint = plan ? theme.tokens.accent.base : theme.tokens.text.secondary;
  const effortText = !plan && !effort.hidden && effort.value ? effort.value : null;
  const modeText = permission.current
    ? (isCodex ? (plan ? m.chat_mode_plan() : m.chat_mode_normal()) : permissionLabel(permission.current))
    : null;

  const long = model.items.length > LONG_LIST;
  const q = long ? query.trim().toLowerCase() : '';
  const models = q ? model.items.filter((it) => `${it.hint ?? ''} ${it.label}`.toLowerCase().includes(q)) : model.items;

  // Escolher modo fecha, como no PC; a falha deixa o painel aberto com o aviso.
  const pickMode = async (mo: string) => {
    if (mo === permission.current) { close(); return; }
    if (await permission.select(mo)) close();
  };
  const pickApproval = async (mo: string) => {
    if (mo === approval.current) { close(); return; }
    if (await approval.select(mo)) close();
  };

  return (
    <>
      {hidden ? null : (
        <Pressable
          ref={anchor}
          onPress={() => setOpen(true)}
          hitSlop={{ top: 7, bottom: 7 }}
          style={({ pressed }) => [styles.chip, {
            backgroundColor: plan ? theme.tokens.accent.dim : pressed ? theme.tokens.bg.hover : theme.tokens.fillSubtle,
          }]}
          accessibilityRole="button"
          accessibilityLabel={m.composer_session_settings()}
          accessibilityHint={m.composer_session_settings_hint()}
          accessibilityValue={{ text: [modeText, model.display, effortText].filter(Boolean).join(', ') }}
        >
          {icon ? <Icon name={icon} size={13} color={tint} /> : null}
          <Text style={styles.chipText} numberOfLines={1}>
            {plan ? <Text style={[styles.chipStrong, { color: tint }]}>{`${m.chat_mode_plan()} `}</Text> : null}
            {plan ? <Text style={{ color: tint }}>{'· '}</Text> : null}
            <Text style={[plan ? { color: tint } : styles.chipStrong, !plan && { color: model.failed ? theme.tokens.status.error : theme.tokens.text.primary }]}>
              {model.display}
            </Text>
            {effortText ? <Text style={{ color: theme.tokens.text.secondary }}>{` ${effortText}`}</Text> : null}
          </Text>
          <Icon name="ChevronDown" size={13} color={tint} />
        </Pressable>
      )}

      <AnchoredPanel
        open={open}
        anchor={hidden ? null : anchor.current}
        onClose={close}
        onDismissed={() => setQuery('')}
        label={m.composer_session_settings()}
        closeLabel={m.native_close()}
        radius={12}
        padding={4}
        maxHeightRatio={1}
      >
        <View style={styles.panel}>
          <Title text={m.native_ctl_model_title()} />
          {long ? (
            <View style={styles.search}>
              <Icon name="Search" size={14} color={theme.tokens.text.muted} />
              <TextInput
                value={query}
                onChangeText={setQuery}
                placeholder={m.native_ctl_search()}
                placeholderTextColor={theme.tokens.text.muted}
                accessibilityLabel={m.native_ctl_search()}
                autoCorrect={false}
                autoCapitalize="none"
                returnKeyType="search"
                style={styles.searchInput}
              />
            </View>
          ) : null}
          <Notice text={model.notice} />
          {model.loading ? <Skeleton rows={5} /> : model.error ? (
            <Failure text={model.error} onRetry={() => void model.load()} />
          ) : models.length === 0 ? (
            <Empty text={q ? m.native_ctl_no_results() : m.comum_nenhum_modelo()} />
          ) : (
            <ScrollView style={styles.models} keyboardShouldPersistTaps="handled" accessibilityRole="radiogroup">
              {models.map((it, idx) => (
                <Row
                  key={`${it.label}-${idx}-${it.hint ?? ''}`}
                  selected={!!it.selected}
                  label={it.label}
                  onPress={() => (it.selected ? close() : void model.select(it))}
                >
                  <Text style={styles.rowText} numberOfLines={1}>
                    <Text style={styles.modelName}>{it.label}</Text>
                    {it.hint ? <Text style={styles.detail}>{`  ${it.hint}`}</Text> : null}
                  </Text>
                </Row>
              ))}
            </ScrollView>
          )}

          {effort.hidden ? null : (
            <>
              <Separator />
              <Title text={m.native_new_chat_reasoning()} />
              {effort.loading ? <Skeleton rows={1} /> : effort.error ? (
                <Failure text={effort.error} onRetry={() => void effort.load()} />
              ) : effort.items.length === 0 ? <Empty text={m.modelo_sem_niveis()} /> : (
                // Até 5 níveis dividem a largura; mais que isso quebram linha sem cortar o rótulo.
                <View style={[styles.efforts, effort.items.length > 5 && styles.wrap]} accessibilityRole="radiogroup">
                  {effort.items.map((it) => (
                    <Pressable
                      key={it.label}
                      onPress={() => { if (!it.selected) void effort.select(it.label); }}
                      disabled={effort.applying}
                      hitSlop={{ top: 4, bottom: 4 }}
                      style={({ pressed }) => [styles.effort, effort.items.length <= 5 && styles.share,
                        { backgroundColor: it.selected ? theme.tokens.accent.dim : pressed ? theme.tokens.fillSubtle : 'transparent' },
                        effort.applying && styles.busy]}
                      accessibilityRole="radio"
                      accessibilityLabel={effortLabel(it.label)}
                      accessibilityState={{ selected: !!it.selected, disabled: effort.applying }}
                    >
                      <Text style={[styles.effortText, it.selected && styles.effortOn]} numberOfLines={1}>{effortLabel(it.label)}</Text>
                    </Pressable>
                  ))}
                </View>
              )}
              <Notice text={effort.notice} />
            </>
          )}

          {permission.enabled ? (
            <>
              <Separator />
              <Title text={m.native_ctl_mode_title()} />
              {permission.options.length ? (
                <ChoiceChips
                  busy={permission.applying}
                  items={permission.options.map((mo) => ({
                    key: mo,
                    label: isCodex ? (mo === 'plan' ? m.chat_mode_plan() : m.chat_mode_normal()) : permissionLabel(mo),
                    selected: mo === permission.current,
                  }))}
                  onPick={(mo) => void pickMode(mo)}
                />
              ) : null}
              {permission.probing ? <Skeleton rows={1} /> : permission.canProbe ? (
                <Probe hint={m.native_mode_probe_hint()} onPress={() => void permission.probe()} />
              ) : permission.noCycle ? <Empty text={m.permissao_sem_ciclo()} /> : null}
              <Notice text={permission.notice} />
            </>
          ) : null}

          {permission.enabled && isCodex ? (
            <>
              <Separator />
              <Title text={m.composer_codex_approval()} />
              {approval.loading ? <Skeleton rows={3} /> : approval.error ? (
                <Failure text={approval.error} onRetry={() => void approval.load()} />
              ) : approval.modes ? (
                approval.modes.length ? (
                  <ChoiceChips
                    busy={approval.applying}
                    items={approval.modes.map((mo) => ({ key: mo.name, label: approvalShortLabel(mo.name), aria: mo.name, selected: mo.name === approval.current }))}
                    onPick={(mo) => void pickApproval(mo)}
                  />
                ) : <Empty text={headless ? m.permissao_codex_headless_sem_lista() : m.permissao_codex_sem_lista()} />
              ) : (
                <Probe hint={m.native_codex_permission_probe_hint()} onPress={() => void approval.load()} />
              )}
              <Notice text={approval.notice} />
            </>
          ) : null}
        </View>
      </AnchoredPanel>
    </>
  );
}

// Escolha curta na horizontal, o desenho do rodapé de Raciocínio do PC: quebra linha em vez de
// crescer em lista — modo e aprovação têm rótulos curtos e a altura do menu importa mais.
function ChoiceChips({ items, onPick, busy }: {
  items: { key: string; label: string; selected: boolean; aria?: string }[];
  onPick: (key: string) => void;
  busy?: boolean;
}) {
  const { theme } = useUnistyles();
  return (
    <View style={[styles.efforts, styles.wrap]} accessibilityRole="radiogroup">
      {items.map((it) => (
        <Pressable
          key={it.key}
          onPress={() => { if (!it.selected) onPick(it.key); }}
          disabled={busy}
          hitSlop={{ top: 4, bottom: 4 }}
          style={({ pressed }) => [styles.effort,
            { backgroundColor: it.selected ? theme.tokens.accent.dim : pressed ? theme.tokens.fillSubtle : 'transparent' },
            busy && styles.busy]}
          accessibilityRole="radio"
          accessibilityLabel={it.aria ?? it.label}
          accessibilityState={{ selected: it.selected, disabled: !!busy }}
        >
          <Text style={[styles.effortText, it.selected && styles.effortOn]} numberOfLines={1}>{it.label}</Text>
        </Pressable>
      ))}
    </View>
  );
}

// `popup::title`: caixa alta miúda, apagada.
function Title({ text }: { text: string }) {
  return <Text style={styles.title} numberOfLines={1} accessibilityRole="header">{text}</Text>;
}

// `popup::separator`: filete de borda a borda, desfazendo o recuo do cartão.
function Separator() {
  return <View style={styles.separator} />;
}

// `popup::row` + `ctl_row`: a atual leva o fundo accent e o tique; o toque, o véu do texto.
function Row({ selected, label, busy, onPress, children }: {
  selected: boolean; label: string; busy?: boolean; onPress: () => void; children: ReactNode;
}) {
  const { theme } = useUnistyles();
  return (
    <Pressable
      onPress={onPress}
      disabled={busy}
      style={({ pressed }) => [styles.row,
        { backgroundColor: selected ? theme.tokens.accent.dim : pressed ? theme.tokens.fillSubtle : 'transparent' }, busy && styles.busy]}
      accessibilityRole="radio"
      accessibilityLabel={label}
      accessibilityState={{ selected, disabled: !!busy }}
    >
      {children}
      {selected ? <Icon name="Check" size={16} color={theme.tokens.accent.base} /> : null}
    </Pressable>
  );
}

// `popup::skeleton`: linhas vazias de 22 no lugar da lista enquanto ela carrega.
function Skeleton({ rows }: { rows: number }) {
  return (
    <View style={styles.skeleton} accessibilityLabel={m.native_ctl_loading()} accessibilityRole="progressbar">
      {Array.from({ length: rows }, (_, i) => <View key={i} style={styles.bone} />)}
    </View>
  );
}

function Empty({ text }: { text: string }) {
  return <Text style={styles.empty}>{text}</Text>;
}

function Notice({ text }: { text?: string | null }) {
  return text ? <Text style={[styles.note, styles.danger]} accessibilityRole="alert">{text}</Text> : null;
}

function Failure({ text, onRetry }: { text: string; onRetry: () => void }) {
  return (
    <View style={styles.failure}>
      <Text style={[styles.note, styles.danger]}>{m.native_ctl_failed({ reason: text })}</Text>
      <SmallButton label={m.native_ctl_retry()} onPress={onRetry} />
    </View>
  );
}

// Ler mexe no terminal da sessão (Shift+Tab, `/permissions`): só com o toque da pessoa.
function Probe({ hint, onPress }: { hint: string; onPress: () => void }) {
  return (
    <View style={styles.probe}>
      <Text style={[styles.note, styles.grow]}>{hint}</Text>
      <SmallButton label={m.native_ctl_probe()} onPress={onPress} />
    </View>
  );
}

function SmallButton({ label, onPress }: { label: string; onPress: () => void }) {
  const { theme } = useUnistyles();
  return (
    <Pressable onPress={onPress} hitSlop={{ top: 6, bottom: 6 }} accessibilityRole="button"
      style={({ pressed }) => [styles.small, pressed && { backgroundColor: theme.tokens.fillSubtle }]}>
      <Text style={styles.smallText}>{label}</Text>
    </Pressable>
  );
}

// Medidas do popup.rs: título 10 px, linha px 8 / py 6 / raio 7 / 13 px, recuo do cartão 4.
const styles = StyleSheet.create((theme) => ({
  // 30 pt + hitSlop 7 em cima e embaixo = 44 de toque; encolhe com reticências quando a linha aperta.
  chip: {
    flexShrink: 1,
    minWidth: 0,
    height: 30,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
    borderRadius: 15,
    paddingHorizontal: 10,
  },
  chipText: {
    flexShrink: 1,
    fontSize: theme.base.text.xs,
    color: theme.tokens.text.secondary,
  },
  chipStrong: {
    fontWeight: '600',
  },
  panel: { flexShrink: 1, gap: 2 },
  title: {
    paddingHorizontal: 8,
    paddingTop: 6,
    paddingBottom: 4,
    fontSize: 10,
    fontWeight: '500',
    letterSpacing: 0.4,
    textTransform: 'uppercase',
    color: theme.tokens.text.muted,
  },
  separator: { height: 1, marginHorizontal: -4, marginVertical: 2, backgroundColor: theme.tokens.border.default },
  search: {
    height: 32,
    marginHorizontal: 4,
    marginBottom: 4,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 6,
    paddingHorizontal: 8,
    borderRadius: 8,
    borderWidth: 1,
    borderColor: theme.tokens.border.default,
    backgroundColor: theme.tokens.fillSubtle,
  },
  searchInput: { flex: 1, padding: 0, fontSize: 13, color: theme.tokens.text.primary },
  // O teto do 260 do PC, com piso para duas linhas: falta altura (teclado aberto), é esta que cede.
  models: { flexGrow: 0, flexShrink: 1, minHeight: 72, maxHeight: 288 },
  // Mais alta que os 28 do PC: no dedo, 36 é o mínimo que não erra a linha vizinha.
  row: {
    minHeight: 36,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    paddingHorizontal: 8,
    paddingVertical: 6,
    borderRadius: 7,
  },
  rowText: { flex: 1, minWidth: 0, fontSize: 13, color: theme.tokens.text.primary },
  modelName: { fontWeight: '500' },
  name: { fontWeight: '400' },
  detail: { fontSize: 12, color: theme.tokens.text.muted },
  efforts: { flexDirection: 'row', gap: 2, paddingHorizontal: 4, paddingBottom: 2 },
  wrap: { flexWrap: 'wrap' },
  effort: { minHeight: 30, justifyContent: 'center', alignItems: 'center', paddingHorizontal: 8, borderRadius: 6 },
  share: { flex: 1, minWidth: 0, paddingHorizontal: 2 },
  effortText: { fontSize: 12, color: theme.tokens.text.secondary },
  effortOn: { color: theme.tokens.text.primary, fontWeight: '600' },
  busy: { opacity: 0.6 },
  skeleton: { gap: 6, paddingHorizontal: 8, paddingVertical: 4 },
  bone: { height: 22, borderRadius: 7, backgroundColor: theme.tokens.fillSubtle },
  empty: { paddingHorizontal: 8, paddingVertical: 16, fontSize: 12, textAlign: 'center', color: theme.tokens.text.muted },
  note: { paddingHorizontal: 8, paddingVertical: 2, fontSize: 12, color: theme.tokens.text.muted },
  danger: { color: theme.tokens.status.error },
  failure: { alignItems: 'flex-start', gap: 6, paddingVertical: 4 },
  probe: { flexDirection: 'row', alignItems: 'center', paddingVertical: 2 },
  grow: { flex: 1 },
  small: {
    minHeight: 28,
    justifyContent: 'center',
    marginHorizontal: 8,
    paddingHorizontal: 10,
    borderRadius: 6,
    borderWidth: 1,
    borderColor: theme.tokens.border.default,
  },
  smallText: { fontSize: 12, fontWeight: '500', color: theme.tokens.accent.base },
}));
