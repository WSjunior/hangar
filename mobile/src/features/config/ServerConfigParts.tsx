import { useState, type ReactNode } from 'react';
import { Pressable, Switch, Text, TextInput, View } from 'react-native';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { Icon, type IconName } from '../../ui/Icon';
import { superficie } from '../../theme/superficie';
import { Pagina } from './Pagina';
import { PageHeader, Pill } from './PageHeader';
import { InfoNotice } from './InfoNotice';
import { Segmented } from './Segmented';
import { useSettingsColors } from './colors';
import { FIELDS, VERDICTS, textOf, type ServerConfig } from './serverConfig';
import * as m from '../../paraglide/messages';

/**
 * Casca das páginas de configuração do servidor (Voz, Jev, Notificações, Anexos, Avançado): os quatro
 * estados da leitura e o rodapé do Salvar fora da rolagem, como o `server_config_footer` do Rust.
 * `extra` aparece mesmo sem a leitura do `/api/config` (as horas silenciosas têm leitura própria).
 */
export function ServerConfigPage({ title, cfg, children, extra }: { title: string; cfg: ServerConfig; children: ReactNode; extra?: ReactNode }) {
  const { theme } = useUnistyles();
  const c = useSettingsColors();
  const insets = useSafeAreaInsets();
  const ready = cfg.load.status === 'ready';
  const footer = cfg.server && ready && (cfg.dirty || cfg.saving || cfg.saved);
  let body: ReactNode;
  if (!cfg.server) body = <InfoNotice text={m.native_settings_offline()} />;
  else if (cfg.load.status === 'loading') body = <Text style={[styles.muted, { color: c.muted }]}>{m.native_server_loading()}</Text>;
  else if (cfg.load.status === 'error') body = (
    <View style={styles.errorBox}>
      <Text accessibilityRole="alert" style={[styles.muted, { color: theme.tokens.status.error }]}>{cfg.load.error}</Text>
      <View style={styles.row}><Pill icon="RefreshCw" label={m.native_server_retry()} onPress={cfg.reload} /></View>
    </View>
  );
  else body = children;
  return (
    <View style={styles.fill}>
      <Pagina>
        <PageHeader title={title} />
        {body}
        {cfg.server ? extra : null}
        {footer ? <View style={{ height: 64 + insets.bottom }} /> : null}
      </Pagina>
      {footer ? (
        <View style={[styles.footer, { paddingBottom: 10 + insets.bottom, borderTopColor: c.border, backgroundColor: superficie(theme, 0.95) }]}>
          {/* O motivo do Salvar desligado vem antes de um erro antigo: é o que trava o botão agora. */}
          {cfg.saveBlocked ? (
            <Text accessibilityLiveRegion="polite" numberOfLines={3} style={[styles.footerText, { color: theme.tokens.status.warning }]}>
              {m.native_server_save_blocked_provider_key()}
            </Text>
          ) : cfg.saveError ? (
            <Text accessibilityRole="alert" numberOfLines={3} style={[styles.footerText, { color: theme.tokens.status.error }]}>{cfg.saveError}</Text>
          ) : cfg.saved ? (
            <Text accessibilityLiveRegion="polite" style={[styles.footerText, { color: theme.tokens.status.success }]}>{m.native_server_saved()}</Text>
          ) : <View style={styles.footerText} />}
          {/* Falhou: o rascunho fica para tentar de novo, e Desfazer devolve os valores do servidor. */}
          {cfg.saveError && !cfg.saving ? <Pill label={m.native_server_undo()} onPress={cfg.discard} /> : null}
          {cfg.dirty || cfg.saving ? (
            <Pressable
              onPress={cfg.save}
              disabled={cfg.saving || cfg.saveBlocked}
              accessibilityRole="button"
              accessibilityState={{ disabled: cfg.saving || cfg.saveBlocked, busy: cfg.saving }}
              style={({ pressed }) => [styles.save, { backgroundColor: c.accent, opacity: cfg.saving || cfg.saveBlocked ? 0.6 : pressed ? 0.8 : 1 }]}
            >
              <Text style={styles.saveText}>{cfg.saving ? m.native_server_saving() : m.native_server_save()}</Text>
            </Pressable>
          ) : null}
        </View>
      ) : null}
    </View>
  );
}

/** Caixa de linhas (settings_box do Rust). */
export function Box({ children }: { children: ReactNode }) {
  const { theme } = useUnistyles();
  const c = useSettingsColors();
  return <View style={[styles.box, { borderColor: c.border, backgroundColor: superficie(theme, 0.6) }]}><View style={styles.bare}>{children}</View></View>;
}

export function Chip({ text, tone = 'muted' }: { text: string; tone?: 'muted' | 'accent' | 'success' }) {
  const { theme } = useUnistyles();
  const c = useSettingsColors();
  const [fg, bg] = tone === 'accent' ? [c.accentText, c.accentDim]
    : tone === 'success' ? [theme.tokens.status.success, 'rgba(52,199,89,0.14)'] : [c.muted, c.inset];
  return <View style={[styles.chip, { backgroundColor: bg }]}><Text style={[styles.chipText, { color: fg }]}>{text}</Text></View>;
}

/** Abre e fecha um trecho, como o `<details>` do web. */
export function Disclosure({ open, label, onChange, small }: { open: boolean; label: string; onChange: (open: boolean) => void; small?: boolean }) {
  const c = useSettingsColors();
  return (
    <Pressable
      onPress={() => onChange(!open)}
      accessibilityRole="button"
      accessibilityLabel={label}
      accessibilityState={{ expanded: open }}
      hitSlop={8}
      style={styles.disclosure}
    >
      <Icon name={open ? 'ChevronDown' : 'ChevronRight'} size={small ? 13 : 15} color={c.accentText} />
      <Text style={[small ? styles.disclosureSmall : styles.disclosureText, { color: c.accentText }]}>{label}</Text>
    </Pressable>
  );
}

/** Título de bloco com a ajuda embaixo e, à direita, o que vier (estado da Voz). */
export function BlockHead({ title, help, right }: { title: string; help?: string; right?: ReactNode }) {
  const c = useSettingsColors();
  return (
    <View style={styles.blockHead}>
      <View style={styles.flex}>
        <Text accessibilityRole="header" style={[styles.blockTitle, { color: c.text }]}>{title}</Text>
        {help ? <Text style={[styles.help, { color: c.muted }]}>{help}</Text> : null}
      </View>
      {right}
    </View>
  );
}

export function Muted({ children }: { children: ReactNode }) {
  const c = useSettingsColors();
  return <Text style={[styles.muted, { color: c.muted }]}>{children}</Text>;
}

/** Linha no desenho do SettingsRow, com etiquetas ao lado do título e o controle embaixo. */
export function RowShell({ icon, title, chips, help, right, children }: {
  icon: IconName; title: string; chips?: ReactNode; help?: string; right?: ReactNode; children?: ReactNode;
}) {
  const c = useSettingsColors();
  return (
    <View style={[styles.rowShell, { borderTopColor: c.border }]}>
      <View style={styles.headLine}>
        <View style={[styles.iconBox, { borderColor: c.border, backgroundColor: c.inset }]}><Icon name={icon} size={16} color={c.muted} /></View>
        <View style={styles.flex}>
          <View style={styles.titleLine}>
            <Text style={[styles.title, { color: c.text }]}>{title}</Text>
            {chips}
          </View>
          {help ? <Text style={[styles.help, { color: c.muted }]}>{help}</Text> : null}
        </View>
        {right}
      </View>
      {children ? <View style={styles.below}>{children}</View> : null}
    </View>
  );
}

export function ScopeChips({ cfg, k }: { cfg: ServerConfig; k: string }) {
  return (
    <>
      <Chip text={m.native_server_scope()} />
      {cfg.editedInApp(k) && !cfg.removing(k) ? <Chip text={m.native_server_edited()} tone="accent" /> : null}
    </>
  );
}

export function useInputStyle() {
  const c = useSettingsColors();
  return { style: [styles.input, { borderColor: c.borderStrong, backgroundColor: c.inset, color: c.text }], placeholderTextColor: c.faint };
}

/** A linha de um campo de FIELDS (`config_row` do Rust). `removable`: Remover/Desfazer, só na Voz. */
export function ConfigRow({ cfg, k, removable }: { cfg: ServerConfig; k: string; removable?: boolean }) {
  const { theme } = useUnistyles();
  const c = useSettingsColors();
  const input = useInputStyle();
  const [why, setWhy] = useState(false);
  const field = FIELDS[k];
  const error = cfg.fields[k]?.erro || '';
  // Campo que a leitura não trouxe (servidor mais antigo) fica desligado: o Salvar não o levaria.
  const off = !(k in cfg.fields) || !!error;
  const label = field.label();
  const removing = cfg.removing(k);
  const kind = field.kind;

  let right: ReactNode = null;
  let control: ReactNode = null;
  if (kind.t === 'toggle') {
    right = <Switch accessibilityLabel={label} value={cfg.current(k) === true} disabled={off}
      onValueChange={(on) => { cfg.stage(k, on); }} trackColor={{ true: c.accent }} />;
  } else if (kind.t === 'number') {
    control = (
      <View style={styles.numberLine}>
        <TextInput {...input} style={[input.style, styles.number]} accessibilityLabel={label} editable={!off} keyboardType="number-pad"
          value={textOf(cfg.current(k))} onChangeText={(t) => cfg.stage(k, t.replace(/\D/g, ''))} />
        <Text style={[styles.help, { color: c.muted }]}>{kind.suffix()}</Text>
      </View>
    );
  } else if (kind.t === 'choice') {
    const options = kind.values.map((v) => ({ v, label: v || kind.empty() }));
    control = <Segmented options={options} value={textOf(cfg.current(k))} label={label} onChange={(v) => { if (!off) cfg.stage(k, v); }} />;
  } else if (kind.t === 'secret') {
    const mask = removing ? null : cfg.secretMask(k);
    control = (
      <View style={styles.secret}>
        {/* Mostra só o que foi digitado aqui; vazio de novo sai do rascunho, para não apagar a chave guardada sem querer. */}
        <TextInput {...input} accessibilityLabel={label} editable={!off} secureTextEntry autoCapitalize="none" autoCorrect={false}
          textContentType="password" autoComplete="off"
          placeholder={mask ? m.native_server_secret_paste_new() : m.native_server_secret_paste()}
          value={removing ? '' : textOf(cfg.draft[k])}
          onChangeText={(t) => (t ? cfg.stage(k, t) : cfg.unstage(k))} />
        {mask ? (
          <Text style={[styles.help, { color: c.muted }]} accessibilityHint={m.native_server_secret_no_return()}>
            <Text style={{ fontFamily: theme.base.fontMono }}>{mask}</Text> · {m.native_server_secret_set()}
          </Text>
        ) : null}
      </View>
    );
  } else {
    control = <TextInput {...input} accessibilityLabel={label} editable={!off} autoCapitalize="none" autoCorrect={false}
      value={textOf(cfg.current(k))} onChangeText={(t) => cfg.stage(k, t)} />;
  }

  const verdict = VERDICTS[k];
  return (
    <RowShell icon={field.icon} title={label} chips={<ScopeChips cfg={cfg} k={k} />} help={field.help()} right={right}>
      {verdict || control || removable || error ? (
        <>
          {verdict ? (
            <View style={styles.verdict}>
              <View style={styles.titleLine}>
                <Text style={[styles.help, { color: c.muted }]}>{verdict[0]()}</Text>
                <Disclosure small open={why} label={m.native_accounts_engine_why()} onChange={setWhy} />
              </View>
              {why ? <Text style={[styles.help, { color: c.muted }]}>{verdict[1]()}</Text> : null}
            </View>
          ) : null}
          {control}
          {removable && removing ? (
            <View style={styles.titleLine}>
              <Text accessibilityLiveRegion="polite" style={[styles.help, { color: theme.tokens.status.warning }]}>{m.native_server_remove_on_save()}</Text>
              <Pill label={m.native_server_undo()} onPress={() => cfg.unstage(k)} />
            </View>
          ) : removable && cfg.editedInApp(k) ? (
            <View style={styles.row}>
              <Pill icon="Trash2" label={m.native_server_remove()} onPress={() => cfg.stage(k, null)} />
            </View>
          ) : null}
          {error ? <Text accessibilityRole="alert" style={[styles.help, { color: theme.tokens.status.error }]}>{error}</Text> : null}
        </>
      ) : null}
    </RowShell>
  );
}

const styles = StyleSheet.create({
  fill: { flex: 1 },
  flex: { flex: 1, minWidth: 0, gap: 2 },
  row: { flexDirection: 'row' },
  muted: { fontSize: 15, lineHeight: 20, paddingHorizontal: 4 },
  errorBox: { gap: 10 },
  footer: {
    position: 'absolute', left: 0, right: 0, bottom: 0, borderTopWidth: 1,
    paddingTop: 10, paddingHorizontal: 16, flexDirection: 'row', alignItems: 'center', gap: 12,
  },
  footerText: { flex: 1, fontSize: 14 },
  save: { height: 40, paddingHorizontal: 18, borderRadius: 9999, alignItems: 'center', justifyContent: 'center' },
  saveText: { color: '#fff', fontSize: 15, fontWeight: '600' },
  box: { borderRadius: 14, borderWidth: 1, overflow: 'hidden' },
  bare: { marginTop: -1 },
  chip: { paddingHorizontal: 6, paddingVertical: 1, borderRadius: 9999 },
  chipText: { fontSize: 11.5, fontWeight: '700' },
  disclosure: { flexDirection: 'row', alignItems: 'center', gap: 4, alignSelf: 'flex-start', minHeight: 32 },
  disclosureText: { fontSize: 15, fontWeight: '500' },
  disclosureSmall: { fontSize: 14 },
  blockHead: { flexDirection: 'row', alignItems: 'flex-start', gap: 12, paddingHorizontal: 4, marginTop: 8 },
  blockTitle: { fontSize: 15, fontWeight: '600' },
  rowShell: { borderTopWidth: 1, paddingVertical: 14, paddingHorizontal: 16, gap: 10 },
  headLine: { flexDirection: 'row', alignItems: 'flex-start', gap: 14 },
  iconBox: { width: 36, height: 36, borderRadius: 10, borderWidth: 1, alignItems: 'center', justifyContent: 'center' },
  titleLine: { flexDirection: 'row', flexWrap: 'wrap', alignItems: 'center', gap: 8 },
  title: { fontSize: 15, fontWeight: '500' },
  help: { fontSize: 14, lineHeight: 18 },
  below: { paddingLeft: 50, gap: 8 },
  input: { minHeight: 44, borderWidth: 1, borderRadius: 8, paddingHorizontal: 12, fontSize: 15 },
  numberLine: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  number: { width: 96 },
  secret: { gap: 6 },
  verdict: { gap: 6 },
});
