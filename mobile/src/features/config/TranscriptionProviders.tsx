import { useCallback, useEffect, useState, type ReactNode } from 'react';
import { Pressable, Text, TextInput, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import {
  editTranscriptionProviderKey, fmtWhen, getTranscriptionProvidersStatus, moveTranscriptionProvider, parseTranscriptionProviders,
  type TranscriptionProviderConfig, type TranscriptionProviderStatus,
} from '@hangar/core';
import { BlockHead, Box, useInputStyle } from './ServerConfigParts';
import { Pill } from './PageHeader';
import { Segmented } from './Segmented';
import { useSettingsColors } from './colors';
import { PROVIDERS, type ServerConfig } from './serverConfig';
import { Icon, type IconName } from '../../ui/Icon';
import * as m from '../../paraglide/messages';

type Status = { state: 'loading' } | { state: 'error'; error: string } | { state: 'ready'; byId: Record<string, TranscriptionProviderStatus> };

/**
 * Serviços de transcrição em ordem (`render_providers` do Rust): o primeiro transcreve, e quem falha ou fica sem cota
 * passa a vez. Servidor cuja leitura não trouxe a lista não mostra o bloco.
 */
export function TranscriptionProviders({ cfg }: { cfg: ServerConfig }) {
  const { theme } = useUnistyles();
  const c = useSettingsColors();
  const input = useInputStyle();
  const [status, setStatus] = useState<Status>({ state: 'loading' });
  const server = cfg.server;
  const present = PROVIDERS in cfg.fields;

  const loadStatus = useCallback(() => {
    if (!server) return () => {};
    let live = true;
    setStatus({ state: 'loading' });
    getTranscriptionProvidersStatus(server)
      .then((r) => { if (live) setStatus({ state: 'ready', byId: Object.fromEntries(r.providers.map((p) => [p.id, p])) }); })
      .catch((e: unknown) => {
        if (live) setStatus({ state: 'error', error: e instanceof Error && e.message ? e.message : m.erro_desconhecido() });
      });
    return () => { live = false; };
  }, [server]);

  // Relê a cada leitura boa da página (abrir e Salvar): a espera muda com a lista.
  useEffect(() => (cfg.load.status === 'ready' && present ? loadStatus() : undefined),
    [cfg.load.status, cfg.fields, present, loadStatus]);

  if (!present) return null;
  const list = parseTranscriptionProviders(cfg.current(PROVIDERS));
  const stored = parseTranscriptionProviders(cfg.fields[PROVIDERS]?.valor);
  const stage = (next: TranscriptionProviderConfig[]) => cfg.stage(PROVIDERS, next);
  const edit = (i: number, item: TranscriptionProviderConfig) => stage(list.map((p, n) => (n === i ? item : p)));
  const kinds = [{ v: 'openai', label: m.native_voice_provider_kind_openai() }, { v: 'elevenlabs', label: m.native_voice_provider_kind_elevenlabs() }] as const;
  const now = Date.now() / 1000;
  const add = () => stage([...list, {
    // ponytail: relógio + sorteio; dois toques nunca colidem na prática.
    id: `p${Date.now().toString(16)}${Math.random().toString(16).slice(2, 6)}`,
    kind: 'openai', name: '', base_url: '', api_key: '', model: '',
  }]);
  const iconButton = (icon: IconName, label: string, onPress: () => void, disabled: boolean) => (
    <Pressable onPress={onPress} disabled={disabled} hitSlop={6} style={[styles.iconBtn, disabled && styles.off]}
      accessibilityRole="button" accessibilityLabel={label} accessibilityState={{ disabled }}>
      <Icon name={icon} size={16} color={c.muted} />
    </Pressable>
  );

  return (
    <>
      <BlockHead title={m.native_voice_providers()} help={m.native_voice_providers_help()}
        right={PROVIDERS in cfg.draft ? <Pill label={m.native_server_undo()} onPress={() => cfg.unstage(PROVIDERS)} /> : null} />
      <Box>
        {list.length === 0 ? (
          <View style={[styles.line, { borderTopColor: c.border }]}>
            <Text style={[styles.help, { color: c.muted }]}>{m.native_voice_providers_empty()}</Text>
          </View>
        ) : null}
        {list.map((p, i) => {
          const state = status.state === 'ready' ? status.byId[p.id] : undefined;
          const name = p.name.trim() || state?.name || (p.kind === 'elevenlabs'
            ? m.native_voice_provider_kind_elevenlabs() : m.native_voice_provider_kind_openai());
          const title = `${i + 1}. ${name}`;
          const until = state?.waiting_until && state.waiting_until > now ? state.waiting_until : null;
          const waiting = until ? m.native_voice_provider_waiting({ until: fmtWhen(until) }) : null;
          const mask = stored.find((s) => s.id === p.id)?.api_key || undefined;
          const keptMask = mask && p.api_key === mask ? mask : null;
          return (
            <View key={p.id} style={[styles.line, styles.item, { borderTopColor: c.border }]}>
              <View style={styles.head}>
                <Text numberOfLines={1} style={[styles.title, { color: c.text }]}>{title}</Text>
                {iconButton('ArrowUp', m.native_voice_provider_up({ name }), () => stage(moveTranscriptionProvider(list, i, -1)), i === 0)}
                {iconButton('ArrowDown', m.native_voice_provider_down({ name }), () => stage(moveTranscriptionProvider(list, i, 1)), i === list.length - 1)}
                <Pressable onPress={() => stage(list.filter((_, n) => n !== i))} hitSlop={6} style={styles.remove}
                  accessibilityRole="button" accessibilityLabel={m.native_voice_provider_remove({ name })}>
                  <Icon name="X" size={14} color={c.muted} />
                  <Text style={[styles.removeText, { color: c.muted }]}>{m.native_server_remove()}</Text>
                </Pressable>
              </View>
              {waiting ? (
                <Text accessibilityLiveRegion="polite" style={[styles.small, { color: theme.tokens.status.warning }]}>
                  {state?.reason ? `${waiting}: ${state.reason}` : waiting}
                </Text>
              ) : null}
              <Segmented options={kinds} value={p.kind} label={title}
                onChange={(v) => edit(i, { ...p, kind: v })} />
              {p.kind === 'openai' ? (
                <Labeled label={m.native_voice_provider_endpoint()}>
                  <TextInput {...input} accessibilityLabel={`${m.native_voice_provider_endpoint()}, ${title}`}
                    placeholder="https://api.groq.com/openai/v1" autoCapitalize="none" autoCorrect={false} keyboardType="url"
                    value={p.base_url} onChangeText={(t) => edit(i, { ...p, base_url: t })} />
                </Labeled>
              ) : null}
              <Labeled label={m.native_voice_provider_key()}>
                {/* Como o ConfigRow: mostra só o digitado; apagar volta à máscara, que mantém a chave guardada. */}
                <TextInput {...input} accessibilityLabel={`${m.native_voice_provider_key()}, ${title}`} secureTextEntry
                  autoCapitalize="none" autoCorrect={false} textContentType="password" autoComplete="off"
                  placeholder={mask ? m.native_server_secret_paste_new() : m.native_server_secret_paste()}
                  value={p.api_key === mask ? '' : p.api_key} onChangeText={(t) => edit(i, editTranscriptionProviderKey(p, t, mask))} />
              </Labeled>
              {keptMask ? (
                <Text style={[styles.small, { color: c.muted }]}>
                  <Text style={{ fontFamily: theme.base.fontMono }}>{keptMask}</Text> · {m.native_server_secret_set()}
                </Text>
              ) : null}
              {p.api_key ? null : (
                <Text accessibilityRole="alert" style={[styles.small, { color: theme.tokens.status.warning }]}>{m.native_voice_provider_missing_key()}</Text>
              )}
              <Labeled label={m.native_voice_provider_model()}>
                {/* Modelo padrão do servidor: identificador, não frase de tela. */}
                <TextInput {...input} accessibilityLabel={`${m.native_voice_provider_model()}, ${title}`}
                  placeholder={p.kind === 'elevenlabs' ? 'scribe_v2' : 'whisper-large-v3'}
                  autoCapitalize="none" autoCorrect={false} value={p.model} onChangeText={(t) => edit(i, { ...p, model: t })} />
              </Labeled>
            </View>
          );
        })}
        <View style={[styles.line, styles.addLine, { borderTopColor: c.border }]}>
          <Pill icon="Plus" label={m.native_voice_provider_add()} onPress={add} />
        </View>
      </Box>
      {status.state === 'error' ? (
        <View style={styles.errorLine}>
          <Text accessibilityRole="alert" style={[styles.help, styles.flex, { color: theme.tokens.status.error }]}>
            {m.native_voice_providers_status_failed({ error: status.error })}
          </Text>
          <Pill label={m.native_server_retry()} onPress={() => { loadStatus(); }} />
        </View>
      ) : null}
    </>
  );
}

function Labeled({ label, children }: { label: string; children: ReactNode }) {
  const c = useSettingsColors();
  return (
    <View style={styles.labeled}>
      <Text style={[styles.small, { color: c.muted }]}>{label}</Text>
      {children}
    </View>
  );
}

const styles = StyleSheet.create({
  line: { borderTopWidth: 1, paddingVertical: 14, paddingHorizontal: 16 },
  item: { gap: 10 },
  addLine: { paddingVertical: 10, flexDirection: 'row' },
  head: { flexDirection: 'row', alignItems: 'center', gap: 6 },
  title: { flex: 1, minWidth: 0, fontSize: 15, fontWeight: '500' },
  iconBtn: { width: 32, height: 32, alignItems: 'center', justifyContent: 'center' },
  off: { opacity: 0.4 },
  remove: { flexDirection: 'row', alignItems: 'center', gap: 4, height: 32, paddingHorizontal: 6 },
  removeText: { fontSize: 13.5, fontWeight: '500' },
  labeled: { gap: 4 },
  small: { fontSize: 13.5, lineHeight: 17 },
  help: { fontSize: 14, lineHeight: 18 },
  errorLine: { flexDirection: 'row', alignItems: 'center', gap: 8, paddingHorizontal: 4 },
  flex: { flex: 1 },
});
