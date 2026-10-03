import { useCallback, useEffect, useRef, useState, type ReactNode } from 'react';
import { Alert, Platform, Pressable, Switch, Text, TextInput, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import * as DocumentPicker from 'expo-document-picker';
import { Directory, File } from 'expo-file-system';
import {
  defaultShortcuts, exportShortcuts, getConfigForServer, importShortcuts, patchConfigForServer, resolveShortcuts,
  serializeShortcuts, type Shortcut, type ShortcutImportResult, type ShortcutInternalAction,
} from '@hangar/core';
import { Pagina } from '../../src/features/config/Pagina';
import { PageHeader, Pill } from '../../src/features/config/PageHeader';
import { SectionCard } from '../../src/features/config/SectionCard';
import { Segmented } from '../../src/features/config/Segmented';
import { InfoNotice } from '../../src/features/config/InfoNotice';
import { useSettingsColors } from '../../src/features/config/colors';
import { Icon, type IconName } from '../../src/ui/Icon';
import { useServers } from '../../src/stores/servers';
import * as m from '../../src/paraglide/messages';

const NATIVES: ShortcutInternalAction[] = ['terminal', 'modo', 'navegador', 'anexos', 'rodar', 'externo'];
const NATIVE_LABEL: Record<ShortcutInternalAction, () => string> = {
  terminal: m.native_shortcuts_native_terminal,
  modo: m.native_shortcuts_native_modo,
  navegador: m.native_shortcuts_native_navegador,
  anexos: m.native_shortcuts_native_anexos,
  rodar: m.native_shortcuts_native_rodar,
  externo: m.native_shortcuts_native_externo,
};
const NATIVE_ICON: Record<ShortcutInternalAction, string> = {
  terminal: 'glifo:terminal', modo: 'glifo:git', navegador: 'glifo:globe', anexos: 'glifo:folder', rodar: 'glifo:play',
  externo: 'glifo:terminal',
};
const GLYPHS: [string, IconName][] = [
  ['bolt', 'Zap'], ['play', 'Play'], ['rocket', 'Rocket'], ['gear', 'Settings'], ['git', 'GitBranch'],
  ['chat', 'MessageCircle'], ['star', 'Star'], ['folder', 'Folder'], ['key', 'Key'], ['terminal', 'SquareTerminal'],
  ['globe', 'Globe'], ['robot', 'Bot'],
];
const LABEL_MAX = 24;
const EMOJI_MAX = 4;
const IMPORT_MAX_BYTES = 4 * 1024 * 1024;

type Item = Shortcut & Record<string, unknown>;
type Form = {
  editing: string | null;
  // Gerado uma vez: reenviar depois de uma falha de rede não duplica o atalho novo.
  id: string;
  original: Record<string, unknown> | null;
  shell: boolean;
  label: string;
  emoji: string;
  content: string;
  glyph: string;
  direct: boolean;
  confirm: boolean;
  hangar: boolean;
  home: boolean;
  ask: boolean;
};
type ExportDraft = { loading: boolean; saving: boolean; items: Shortcut[]; selected: Set<string> };
type SecretField = { id: string; label: string; name: string; value: string };
type ImportDraft = { data: unknown; preview: ShortcutImportResult; fields: SecretField[]; expanded: Set<string>; applying: boolean };

const message = (e: unknown) => (e instanceof Error ? e.message : String(e));
const newId = () => `a-${Date.now().toString(36)}-${Math.floor(Math.random() * 36 ** 4).toString(36)}`;
const isScript = (id: string) => id.startsWith('script:');

function glyphIcon(name: string): IconName {
  return GLYPHS.find(([g]) => g === name)?.[1] ?? 'Zap';
}

function ShortcutIcon({ icon, size, color }: { icon?: string; size: number; color: string }) {
  const emoji = icon?.startsWith('emoji:') ? icon.slice(6).trim() : '';
  if (emoji) return <Text style={{ fontSize: size, lineHeight: size + 4 }}>{emoji}</Text>;
  return <Icon name={glyphIcon(icon?.startsWith('glifo:') ? icon.slice(6) : 'bolt')} size={size} color={color} />;
}

function formFor(item: Item | null): Form {
  const icon = typeof item?.icon === 'string' ? item.icon : undefined;
  const emoji = icon?.startsWith('emoji:') ? icon.slice(6) : '';
  // Glifo que esta versão não conhece fica com o nome e volta igual ao gravar.
  const glyph = emoji ? 'bolt' : icon?.startsWith('glifo:') ? icon.slice(6) : 'bolt';
  const shell = item?.type === 'shell';
  return {
    editing: item ? item.id : null,
    id: item ? item.id : newId(),
    original: item ? { ...item } : null,
    shell,
    label: item && item.type !== 'internal' ? item.label : '',
    emoji,
    content: item?.type === 'shell' ? item.command : item?.type === 'send_text' ? item.text : '',
    glyph,
    direct: item?.send_direct !== false,
    confirm: item?.confirm === true,
    hangar: shell && item?.runs_in === 'hangar',
    home: item?.hangar_home !== false,
    ask: item?.answer_in_app !== false,
  };
}

/** Draft::into_item do Rust: campo desconhecido do objeto editado fica; só o que difere do padrão é gravado. */
function formToItem(f: Form): Item {
  const o: Record<string, unknown> = { ...(f.original ?? {}) };
  for (const key of ['send_direct', 'confirm', 'pasta', 'runs_in', 'hangar_home', 'answer_in_app']) delete o[key];
  if (f.shell && f.hangar) o.runs_in = 'hangar';
  if (f.shell && f.hangar && !f.home) o.hangar_home = false;
  if (f.shell && !f.ask) o.answer_in_app = false;
  o.id = f.id;
  o.type = f.shell ? 'shell' : 'send_text';
  o[f.shell ? 'command' : 'text'] = f.content.trim();
  if (!f.shell && !f.direct) o.send_direct = false;
  o.label = f.label.trim();
  const emoji = f.emoji.trim();
  o.icon = emoji ? `emoji:${emoji}` : `glifo:${f.glyph}`;
  if (f.confirm) o.confirm = true;
  return o as Item;
}

export default function ShortcutsScreen() {
  const server = useServers((s) => s.active());
  const c = useSettingsColors();
  const [items, setItems] = useState<Item[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [loadError, setLoadError] = useState('');
  const [saving, setSaving] = useState(false);
  const [saveError, setSaveError] = useState('');
  const [saved, setSaved] = useState(false);
  const [form, setForm] = useState<Form | null>(null);
  const [exportDraft, setExportDraft] = useState<ExportDraft | null>(null);
  const [importDraft, setImportDraft] = useState<ImportDraft | null>(null);
  const [importLoading, setImportLoading] = useState(false);
  const [note, setNote] = useState<{ text: string; error: boolean } | null>(null);
  const [warnings, setWarnings] = useState<string[]>([]);
  const loadGen = useRef(0);
  const saveSeq = useRef(0);
  const transferSeq = useRef(0);
  const savedTimer = useRef<ReturnType<typeof setTimeout>>(undefined);

  const busy = saving || importLoading || !!importDraft?.applying || !!exportDraft?.loading || !!exportDraft?.saving;

  const load = useCallback((target: typeof server) => {
    const gen = ++loadGen.current;
    setLoadError('');
    if (!target) return;
    setLoading(true);
    getConfigForServer(target)
      .then((config) => {
        if (gen !== loadGen.current) return;
        if (!config || typeof config.campos !== 'object' || config.campos === null) throw new Error(m.native_invalid_response());
        const raw = config.campos.shortcuts?.valor;
        setItems(resolveShortcuts(typeof raw === 'string' ? raw : '') as Item[]);
        setLoaded(true);
      })
      .catch((e: unknown) => { if (gen === loadGen.current) setLoadError(message(e)); })
      .finally(() => { if (gen === loadGen.current) setLoading(false); });
  }, []);

  useEffect(() => {
    setItems([]);
    setLoaded(false);
    setForm(null);
    setExportDraft(null);
    setImportDraft(null);
    setImportLoading(false);
    setNote(null);
    setWarnings([]);
    setSaveError('');
    saveSeq.current++;
    transferSeq.current++;
    setSaving(false);
    load(server);
    return () => { loadGen.current++; };
  }, [load, server?.id, server?.baseUrl, server?.token]);

  useEffect(() => () => clearTimeout(savedTimer.current), []);

  // O atalho aberto no formulário saiu da lista (removido, padrão restaurado, lista relida): não há o que gravar.
  useEffect(() => {
    if (form?.editing && !items.some((i) => i.id === form.editing)) setForm(null);
  }, [items, form?.editing]);

  /** Toda mudança grava a lista inteira na hora; `null` restaura o padrão. */
  const save = (list: Item[] | null, fromForm = false) => {
    if (!server || busy) return;
    const seq = ++saveSeq.current;
    setSaving(true);
    setSaveError('');
    setSaved(false);
    patchConfigForServer(server, { shortcuts: list ? serializeShortcuts(list) : null })
      .then(() => {
        if (seq !== saveSeq.current) return;
        setItems(list ?? (defaultShortcuts() as Item[]));
        if (fromForm) setForm(null);
        setSaved(true);
        clearTimeout(savedTimer.current);
        savedTimer.current = setTimeout(() => setSaved(false), 2500);
      })
      .catch((e: unknown) => {
        if (seq !== saveSeq.current) return;
        // Uma falha de rede pode ter gravado mesmo assim: relê para a tela mostrar o que ficou.
        setSaveError(message(e));
        load(server);
      })
      .finally(() => { if (seq === saveSeq.current) setSaving(false); });
  };

  const edit = (change: (list: Item[]) => Item[]) => save(change([...items]));

  const move = (index: number, delta: number) => {
    const j = index + delta;
    if (j < 0 || j >= items.length) return;
    edit((list) => { [list[index], list[j]] = [list[j], list[index]]; return list; });
  };

  const labelOf = (item: Item) => (item.type === 'internal' ? NATIVE_LABEL[item.action]() : item.label);

  const remove = (item: Item) => {
    Alert.alert(m.native_shortcuts_remove(), labelOf(item), [
      { text: m.comum_cancelar(), style: 'cancel' },
      { text: m.native_shortcuts_remove(), style: 'destructive', onPress: () => edit((list) => list.filter((i) => i.id !== item.id)) },
    ]);
  };

  const restoreDefault = () => {
    Alert.alert(m.native_shortcuts_restore(), m.native_shortcuts_restore_help(), [
      { text: m.comum_cancelar(), style: 'cancel' },
      { text: m.native_shortcuts_restore(), style: 'destructive', onPress: () => save(null) },
    ]);
  };

  const submit = () => {
    if (!form || busy || !form.label.trim() || !form.content.trim()) return;
    if (form.editing && !items.some((i) => i.id === form.editing)) {
      setSaveError(m.native_shortcuts_gone());
      return;
    }
    const item = formToItem(form);
    const list = [...items];
    const at = list.findIndex((i) => i.id === (form.editing ?? item.id));
    if (at >= 0) list[at] = item; else list.push(item);
    save(list, true);
  };

  // --- Exportar ---
  const startExport = () => {
    if (!server || busy || importDraft || exportDraft) return;
    const seq = ++transferSeq.current;
    setNote(null);
    setWarnings([]);
    setExportDraft({ loading: true, saving: false, items: [], selected: new Set() });
    exportShortcuts(server, { includeScripts: false })
      .then((value) => {
        if (seq !== transferSeq.current) return;
        if (value?.version !== 2) {
          setExportDraft(null);
          setNote({ text: m.shortcut_transfer_update_required(), error: true });
          return;
        }
        setExportDraft({ loading: false, saving: false, items: value.shortcuts ?? [], selected: new Set() });
        setWarnings(value.warnings ?? []);
      })
      .catch((e: unknown) => {
        if (seq !== transferSeq.current) return;
        setExportDraft(null);
        setNote({ text: message(e), error: true });
      });
  };

  const saveExport = async () => {
    if (!server || !exportDraft || exportDraft.selected.size === 0 || saving || importLoading) return;
    const seq = transferSeq.current;
    const ids = exportDraft.items.map((i) => i.id).filter((id) => exportDraft.selected.has(id));
    setExportDraft({ ...exportDraft, saving: true });
    // Escolhe a pasta antes de pedir o pacote, como no desktop: o arquivo fica onde a pessoa enxerga.
    let folder: Directory;
    try {
      folder = await Directory.pickDirectoryAsync();
    } catch (e) {
      if (seq !== transferSeq.current) return;
      setExportDraft((d) => (d ? { ...d, saving: false } : d));
      if (!/CANCEL/i.test(String((e as { code?: string }).code ?? ''))) setNote({ text: m.native_save_dialog_failed(), error: true });
      return;
    }
    try {
      const value = await exportShortcuts(server, { ids, includeScripts: true });
      if (seq !== transferSeq.current) return;
      if (value?.version !== 2 || !Array.isArray(value.scripts)) throw new Error(m.shortcut_transfer_update_required());
      const { removed, ...body } = value;
      const json = `${JSON.stringify(body, null, 2)}\n`;
      const name = 'hangar-atalhos.json';
      try {
        folder.createFile(name, 'application/json').write(json);
      } catch (e) {
        throw new Error(`${m.native_save_failed()}: ${message(e)}`);
      }
      const path = `${folder.name}/${name}`;
      setExportDraft(null);
      setWarnings(value.warnings ?? []);
      setNote({
        text: removed > 0
          ? m.native_shortcuts_exported_secrets({ path, n: removed })
          : m.native_shortcuts_exported({ path }),
        error: false,
      });
    } catch (e) {
      if (seq !== transferSeq.current) return;
      setExportDraft((d) => (d ? { ...d, saving: false } : d));
      setNote({ text: message(e), error: true });
    }
  };

  // --- Importar ---
  const startImport = async () => {
    if (!server || busy || importDraft || exportDraft) return;
    const seq = ++transferSeq.current;
    setNote(null);
    setWarnings([]);
    let uri: string;
    try {
      const r = await DocumentPicker.getDocumentAsync({ type: '*/*', copyToCacheDirectory: true });
      if (r.canceled || !r.assets[0]) return;
      uri = r.assets[0].uri;
    } catch {
      setNote({ text: m.native_picker_failed(), error: true });
      return;
    }
    if (seq !== transferSeq.current) return;
    setImportLoading(true);
    try {
      const file = new File(uri);
      if ((file.size ?? 0) > IMPORT_MAX_BYTES) throw new Error(m.shortcut_import_too_large());
      const data: unknown = JSON.parse(await file.text());
      const preview = await importShortcuts({ data }, server);
      if (seq !== transferSeq.current) return;
      const version = (data as { version?: unknown } | null)?.version;
      if (version === 2 && !Array.isArray(preview.files)) {
        setNote({ text: m.shortcut_transfer_update_required(), error: true });
        return;
      }
      const fields = (preview.placeholders ?? []).flatMap((p) =>
        p.names.map((name) => ({ id: p.id, label: p.label, name, value: '' })));
      setWarnings(preview.warnings ?? []);
      setImportDraft({ data, preview, fields, expanded: new Set(), applying: false });
    } catch (e) {
      if (seq === transferSeq.current) setNote({ text: m.native_shortcuts_import_invalid({ error: message(e) }), error: true });
    } finally {
      if (seq === transferSeq.current) setImportLoading(false);
    }
  };

  const scriptSecretMissing = !!importDraft?.fields.some((f) => isScript(f.id) && !f.value);

  const applyImport = () => {
    if (!server || !importDraft || saving || importDraft.applying || scriptSecretMissing) return;
    const seq = transferSeq.current;
    const secrets: Record<string, Record<string, string>> = {};
    for (const f of importDraft.fields) (secrets[f.id] ??= {})[f.name] = f.value;
    setImportDraft({ ...importDraft, applying: true });
    importShortcuts({ data: importDraft.data, apply: true, secrets }, server)
      .then(() => {
        if (seq !== transferSeq.current) return;
        setImportDraft(null);
        setNote({ text: m.native_shortcuts_imported(), error: false });
        load(server);
      })
      .catch((e: unknown) => {
        if (seq !== transferSeq.current) return;
        setImportDraft((d) => (d ? { ...d, applying: false } : d));
        setNote({ text: m.native_shortcuts_import_invalid({ error: message(e) }), error: true });
      });
  };

  const cancelTransfer = () => {
    transferSeq.current++;
    setExportDraft(null);
    setImportDraft(null);
  };

  // --- Render ---
  const header = <PageHeader title={m.native_settings_page_shortcuts()} subtitle={m.native_shortcuts_lead()} />;
  if (!server) return <Pagina>{header}<InfoNotice text={m.native_settings_offline()} /></Pagina>;
  if (!loaded) {
    return (
      <Pagina>
        {header}
        <SectionCard>
          <View style={styles.pad}>
            {loadError ? (
              <>
                <Text accessibilityRole="alert" style={styles.error}>{`${m.native_shortcuts_load_failed()} ${loadError}`}</Text>
                <View style={styles.rowWrap}>
                  <Pill icon="RefreshCw" label={loading ? m.native_shortcuts_loading() : m.native_shortcuts_retry()} onPress={() => { if (!loading) load(server); }} />
                </View>
              </>
            ) : (
              <Text style={[styles.note, { color: c.muted }]}>{m.native_shortcuts_loading()}</Text>
            )}
          </View>
        </SectionCard>
      </Pagina>
    );
  }

  const missing = NATIVES.filter((a) => !items.some((i) => i.type === 'internal' && i.action === a));

  const row = (item: Item, n: number) => {
    const native = item.type === 'internal';
    const icon = native ? NATIVE_ICON[item.action] : item.icon;
    const mark = item.type === 'send_text' ? m.atalhos_marca_sessao_texto()
      : item.type === 'shell' ? (item.runs_in === 'hangar' ? m.atalhos_marca_hangar() : m.atalhos_marca_sessao_comando()) : null;
    const editing = form?.editing === item.id;
    const hangar = item.type === 'shell' && item.runs_in === 'hangar';
    return (
      <View key={item.id} style={[styles.row, { borderTopColor: c.border }, editing && { backgroundColor: c.accentDim }]}>
        <View style={styles.rowHead}>
          <View style={[styles.iconBox, { borderColor: c.border, backgroundColor: c.inset }]}>
            <ShortcutIcon icon={icon} size={17} color={c.muted} />
          </View>
          <View style={styles.texts}>
            <View style={styles.titleLine}>
              <Text style={[styles.rowTitle, { color: c.text }]} numberOfLines={1}>{labelOf(item)}</Text>
              {editing ? (
                <Text style={[styles.small, { color: c.faint }]}>{m.atalhos_editando()}</Text>
              ) : mark ? (
                <View style={[styles.mark, hangar
                  ? { backgroundColor: c.accentDim, borderColor: c.accent, }
                  : { borderColor: c.border }]}>
                  <Text style={[styles.small, { color: hangar ? c.accentText : c.muted }]}>{mark}</Text>
                </View>
              ) : null}
            </View>
            {!native ? (
              <Text style={[styles.mono, { color: c.faint }]} numberOfLines={1}>
                {item.type === 'shell' ? item.command : item.type === 'send_text' ? item.text : ''}
              </Text>
            ) : null}
          </View>
        </View>
        <View style={styles.actions}>
          {!native ? <IconButton icon="Pencil" label={m.native_shortcuts_edit()} disabled={busy} onPress={() => setForm(formFor(item))} /> : null}
          <IconButton icon="ArrowUp" label={m.native_shortcuts_up()} disabled={busy || n === 0} onPress={() => move(n, -1)} />
          <IconButton icon="ArrowDown" label={m.native_shortcuts_down()} disabled={busy || n + 1 === items.length} onPress={() => move(n, 1)} />
          <IconButton icon="X" label={m.native_shortcuts_remove()} disabled={busy} onPress={() => remove(item)} />
        </View>
      </View>
    );
  };

  return (
    <Pagina>
      {header}

      <SectionCard>
        {items.length === 0 ? (
          <View style={[styles.row, { borderTopColor: c.border }]}>
            <Text style={[styles.note, { color: c.muted }]}>{m.native_shortcuts_empty()}</Text>
          </View>
        ) : items.map(row)}
        {!form ? (
          <View style={[styles.footer, { borderTopColor: c.border }]}>
            <Pill icon="Plus" label={m.native_shortcuts_add()} onPress={() => { if (!busy) setForm(formFor(null)); }} />
            {missing.length ? (
              <View style={styles.restore}>
                <Text style={[styles.small, { color: c.muted }]}>{m.native_shortcuts_restore_native()}</Text>
                <View style={styles.rowWrap}>
                  {missing.map((action) => (
                    <Pill key={action} icon="Plus" label={NATIVE_LABEL[action]()}
                      onPress={() => edit((list) => [...list, { id: action, type: 'internal', action } as Item])} />
                  ))}
                </View>
              </View>
            ) : null}
          </View>
        ) : null}
      </SectionCard>

      {saveError && !form ? <Text accessibilityRole="alert" style={styles.error}>{saveError}</Text> : null}
      {loading ? <Text style={[styles.note, { color: c.muted }]}>{m.native_shortcuts_loading()}</Text> : null}

      {form ? (
        <ShortcutForm
          form={form}
          setForm={setForm}
          busy={busy}
          saving={saving}
          error={saveError}
          onCancel={() => { setForm(null); setSaveError(''); }}
          onSubmit={submit}
        />
      ) : null}

      <SectionCard icon="ArrowLeftRight" title={m.native_shortcuts_transfer()}>
        <View style={[styles.pad, styles.gap]}>
          <View style={styles.rowWrap}>
            <Pill icon="Upload" label={importLoading ? m.comum_carregando() : m.native_shortcuts_import()} onPress={() => { void startImport(); }} />
            <Pill icon="Download" label={m.native_shortcuts_export()} onPress={startExport} />
          </View>
          {note ? (
            <Text accessibilityRole={note.error ? 'alert' : undefined} style={note.error ? styles.error : [styles.note, { color: c.muted }]}>{note.text}</Text>
          ) : null}
          {warnings.length ? (
            <View style={styles.gapSmall}>
              <Text style={[styles.strong, { color: c.text }]}>{m.shortcut_bundle_warnings()}</Text>
              {warnings.map((w, i) => <Text key={i} style={styles.warning}>{w}</Text>)}
            </View>
          ) : null}
        </View>
      </SectionCard>

      {exportDraft ? (
        <SectionCard icon="Download" title={m.shortcut_export_title()} subtitle={m.shortcut_export_help()}>
          <View style={[styles.pad, styles.gap]}>
            <Text style={[styles.small, { color: c.muted }]}>{m.shortcut_source_server({ nome: server.label })}</Text>
            <Text style={[styles.small, { color: c.muted }]}>{m.shortcut_bundle_help()}</Text>
            {exportDraft.loading ? (
              <Text style={[styles.note, { color: c.muted }]}>{m.comum_carregando()}</Text>
            ) : exportDraft.items.length === 0 ? (
              <Text style={[styles.note, { color: c.text }]}>{m.shortcut_export_empty()}</Text>
            ) : (
              <>
                <View style={styles.rowWrap}>
                  <Pill label={m.shortcut_select_all()} onPress={() => setExportDraft((d) => d && !d.saving ? { ...d, selected: new Set(d.items.map((i) => i.id)) } : d)} />
                  <Pill label={m.shortcut_select_none()} onPress={() => setExportDraft((d) => d && !d.saving ? { ...d, selected: new Set() } : d)} />
                </View>
                {exportDraft.items.map((item) => (
                  <Toggle
                    key={item.id}
                    label={item.type === 'internal' ? (NATIVE_LABEL[item.action]?.() ?? item.id) : item.label || item.id}
                    value={exportDraft.selected.has(item.id)}
                    disabled={exportDraft.saving}
                    onChange={(on) => setExportDraft((d) => {
                      if (!d) return d;
                      const selected = new Set(d.selected);
                      if (on) selected.add(item.id); else selected.delete(item.id);
                      return { ...d, selected };
                    })}
                  />
                ))}
              </>
            )}
            <View style={styles.buttons}>
              <Pill label={m.native_cancel()} onPress={() => { if (!exportDraft.saving) cancelTransfer(); }} />
              <PrimaryButton
                label={m.shortcut_export_selected({ n: exportDraft.selected.size })}
                loading={exportDraft.saving}
                disabled={saving || exportDraft.loading || exportDraft.selected.size === 0}
                onPress={() => { void saveExport(); }}
              />
            </View>
          </View>
        </SectionCard>
      ) : null}

      {importDraft ? (
        <SectionCard icon="Upload" title={m.native_shortcuts_import_title()}
          subtitle={m.native_shortcuts_import_summary({ added: importDraft.preview.added, replaced: importDraft.preview.replaced })}>
          <View style={[styles.pad, styles.gap]}>
            <Text style={[styles.small, { color: c.muted }]}>{m.shortcut_source_server({ nome: server.label })}</Text>
            {importDraft.preview.files?.length ? (
              <View style={styles.gapSmall}>
                <Text style={[styles.strong, { color: c.text }]}>{m.shortcut_import_files()}</Text>
                {importDraft.preview.files.map((file) => {
                  const open = importDraft.expanded.has(file.path);
                  const status = file.status === 'create' ? m.shortcut_file_create()
                    : file.status === 'replace' ? m.shortcut_file_replace()
                    : file.status === 'same' ? m.shortcut_file_same() : m.native_invalid_response();
                  return (
                    <View key={file.path} style={styles.gapSmall}>
                      <Pressable
                        accessibilityRole="button"
                        accessibilityState={{ expanded: open }}
                        accessibilityLabel={`${status} · ${file.path}`}
                        onPress={() => setImportDraft((d) => {
                          if (!d) return d;
                          const expanded = new Set(d.expanded);
                          if (!expanded.delete(file.path)) expanded.add(file.path);
                          return { ...d, expanded };
                        })}
                        style={styles.fileLine}
                      >
                        <Icon name={open ? 'ChevronDown' : 'ChevronRight'} size={16} color={c.muted} />
                        <Text style={[styles.note, { color: c.text, flex: 1 }]}>{`${status} · ${file.path}`}</Text>
                      </Pressable>
                      {open ? <Text style={[styles.mono, styles.fileBody, { color: c.text, backgroundColor: c.inset }]}>{file.content}</Text> : null}
                    </View>
                  );
                })}
              </View>
            ) : null}
            {importDraft.fields.some((f) => !isScript(f.id)) ? (
              <Text style={[styles.small, { color: c.muted }]}>{m.native_shortcuts_import_secrets()}</Text>
            ) : null}
            {importDraft.fields.some((f) => isScript(f.id)) ? (
              <Text style={[styles.small, { color: c.muted }]}>{m.shortcut_script_secrets_required()}</Text>
            ) : null}
            {importDraft.fields.map((f, i) => (
              <Field key={`${f.id}:${f.name}`} label={`${f.label}: ${f.name}`}>
                <TextInput
                  value={f.value}
                  secureTextEntry
                  autoCapitalize="none"
                  autoCorrect={false}
                  editable={!importDraft.applying}
                  accessibilityLabel={`${f.label}: ${f.name}`}
                  onChangeText={(value) => setImportDraft((d) => d && { ...d, fields: d.fields.map((x, k) => (k === i ? { ...x, value } : x)) })}
                  style={[styles.input, { borderColor: c.borderStrong, backgroundColor: c.inset, color: c.text }]}
                />
              </Field>
            ))}
            <View style={styles.buttons}>
              <Pill label={m.native_cancel()} onPress={() => { if (!importDraft.applying) cancelTransfer(); }} />
              <PrimaryButton
                label={m.native_shortcuts_import()}
                loading={importDraft.applying}
                disabled={saving || scriptSecretMissing}
                onPress={applyImport}
              />
            </View>
          </View>
        </SectionCard>
      ) : null}

      <View style={[styles.pageFooter, { borderTopColor: c.border }]}>
        <Pill label={m.native_shortcuts_restore()} onPress={() => { if (!busy) restoreDefault(); }} />
        {saved && !saveError ? <Text style={styles.saved}>{m.native_shortcuts_saved()}</Text> : null}
      </View>
    </Pagina>
  );
}

function ShortcutForm({ form, setForm, busy, saving, error, onCancel, onSubmit }: {
  form: Form;
  setForm: (f: (prev: Form | null) => Form | null) => void;
  busy: boolean;
  saving: boolean;
  error: string;
  onCancel: () => void;
  onSubmit: () => void;
}) {
  const c = useSettingsColors();
  const set = (patch: Partial<Form>) => setForm((f) => (f ? { ...f, ...patch } : f));
  const editing = form.editing !== null;
  const valid = !!form.label.trim() && !!form.content.trim();
  const input = [styles.input, { borderColor: c.borderStrong, backgroundColor: c.inset, color: c.text }];
  const homeText = m.atalhos_onde_home({ home: '~' });
  return (
    <SectionCard icon={editing ? 'Pencil' : 'Plus'} title={editing ? m.native_shortcuts_edit() : m.native_shortcuts_add()}>
      <View style={[styles.pad, styles.formGap]}>
        <Field label={m.native_shortcuts_type()}>
          {/* Editar não troca o tipo: os campos de um e de outro não se convertem. */}
          <View pointerEvents={editing ? 'none' : 'auto'} accessibilityState={{ disabled: editing }} style={editing ? styles.dim : undefined}>
            <Segmented
              label={m.native_shortcuts_type()}
              options={[{ v: 'send', label: m.native_shortcuts_type_send() }, { v: 'shell', label: m.native_shortcuts_type_shell() }]}
              value={form.shell ? 'shell' : 'send'}
              onChange={(v) => set({ shell: v === 'shell' })}
            />
          </View>
        </Field>
        <Field label={m.native_shortcuts_label()}>
          <TextInput value={form.label} maxLength={LABEL_MAX} onChangeText={(label) => set({ label })}
            accessibilityLabel={m.native_shortcuts_label()} style={input} />
        </Field>
        <Field label={form.shell ? m.native_shortcuts_command() : m.native_shortcuts_text()}>
          <TextInput
            value={form.content}
            onChangeText={(content) => set({ content })}
            placeholder={form.shell ? m.native_shortcuts_command_hint() : m.native_shortcuts_text_hint()}
            placeholderTextColor={c.faint}
            multiline
            autoCapitalize="none"
            autoCorrect={!form.shell}
            accessibilityLabel={form.shell ? m.native_shortcuts_command() : m.native_shortcuts_text()}
            style={[...input, styles.multiline, form.shell && styles.monoInput]}
          />
        </Field>
        <Field label={m.native_shortcuts_icon()}>
          <View style={styles.glyphs}>
            {GLYPHS.map(([g, icon]) => {
              const on = !form.emoji.trim() && form.glyph === g;
              return (
                <Pressable key={g} onPress={() => set({ glyph: g, emoji: '' })} accessibilityRole="radio"
                  accessibilityLabel={g} accessibilityState={{ selected: on }}
                  style={[styles.glyph, { backgroundColor: on ? c.accentDim : 'transparent' }]}>
                  <Icon name={icon} size={18} color={on ? c.accentText : c.muted} />
                </Pressable>
              );
            })}
          </View>
          <TextInput value={form.emoji} maxLength={EMOJI_MAX} onChangeText={(emoji) => set({ emoji })}
            placeholder={m.native_shortcuts_emoji_hint()} placeholderTextColor={c.faint}
            accessibilityLabel={m.native_shortcuts_emoji_hint()} style={[...input, styles.emoji]} />
        </Field>
        {form.shell ? (
          <Field label={m.atalhos_onde()}>
            <View style={styles.gap} accessibilityRole="radiogroup">
              <WhereCard on={!form.hangar} title={m.atalhos_onde_sessao()} help={m.atalhos_onde_sessao_ajuda()}
                usage={m.atalhos_onde_sessao_uso()} onPress={() => set({ hangar: false })} />
              <WhereCard on={form.hangar} title={m.atalhos_onde_hangar()} help={m.atalhos_onde_hangar_ajuda()}
                usage={m.atalhos_onde_hangar_uso()} onPress={() => set({ hangar: true })} />
              {form.hangar ? (
                <View style={[styles.hangarBox, { backgroundColor: c.inset, borderColor: c.border }]}>
                  <Text style={[styles.strong, { color: c.text }]}>{m.atalhos_onde_clique_titulo()}</Text>
                  <View style={styles.fileLine}>
                    <Icon name="ArrowRight" size={16} color={c.accent} />
                    <Text style={[styles.note, { color: c.muted, flex: 1 }]}>{m.atalhos_onde_clique()}</Text>
                  </View>
                  <Toggle label={homeText} value={form.home} onChange={(home) => set({ home })} />
                </View>
              ) : null}
            </View>
          </Field>
        ) : null}
        {form.shell ? <Toggle label={m.atalhos_perguntas()} help={m.atalhos_perguntas_ajuda()} value={form.ask} onChange={(ask) => set({ ask })} /> : null}
        {!form.shell ? (
          <Toggle label={m.native_shortcuts_send_direct()} help={m.native_shortcuts_send_direct_help()} value={form.direct} onChange={(direct) => set({ direct })} />
        ) : null}
        <Toggle label={m.native_shortcuts_confirm()} value={form.confirm} onChange={(confirm) => set({ confirm })} />
        {error ? <Text accessibilityRole="alert" style={styles.error}>{error}</Text> : null}
        <View style={styles.buttons}>
          <Pill label={m.native_cancel()} onPress={() => { if (!busy) onCancel(); }} />
          <PrimaryButton label={m.native_shortcuts_form_ok()} loading={saving} disabled={!valid || busy} onPress={onSubmit} />
        </View>
      </View>
    </SectionCard>
  );
}

function Field({ label, children }: { label: string; children: ReactNode }) {
  const c = useSettingsColors();
  return (
    <View style={styles.gapSmall}>
      <Text style={[styles.fieldLabel, { color: c.muted }]}>{label}</Text>
      {children}
    </View>
  );
}

function Toggle({ label, help, value, onChange, disabled }: {
  label: string; help?: string; value: boolean; onChange: (v: boolean) => void; disabled?: boolean;
}) {
  const c = useSettingsColors();
  return (
    <View style={styles.toggle}>
      <View style={styles.texts}>
        <Text style={[styles.note, { color: c.text }]}>{label}</Text>
        {help ? <Text style={[styles.small, { color: c.muted }]}>{help}</Text> : null}
      </View>
      <Switch value={value} onValueChange={onChange} disabled={disabled} accessibilityLabel={label}
        trackColor={{ true: c.accent }} />
    </View>
  );
}

function WhereCard({ on, title, help, usage, onPress }: { on: boolean; title: string; help: string; usage: string; onPress: () => void }) {
  const c = useSettingsColors();
  return (
    <Pressable onPress={onPress} accessibilityRole="radio" accessibilityLabel={title} accessibilityState={{ selected: on }}
      style={[styles.where, { borderColor: on ? c.accent : c.borderStrong, backgroundColor: on ? c.accentDim : 'transparent' }]}>
      <View style={styles.fileLine}>
        <View style={[styles.radio, { borderColor: on ? c.accent : c.faint, borderWidth: on ? 4 : 1 }]} />
        <Text style={[styles.whereTitle, { color: c.text }]}>{title}</Text>
      </View>
      <Text style={[styles.note, { color: c.muted }]}>{help}</Text>
      <Text style={[styles.small, { color: c.faint }]}>{usage}</Text>
    </Pressable>
  );
}

function IconButton({ icon, label, disabled, onPress }: { icon: IconName; label: string; disabled?: boolean; onPress: () => void }) {
  const c = useSettingsColors();
  return (
    <Pressable onPress={onPress} disabled={disabled} accessibilityRole="button" accessibilityLabel={label}
      accessibilityState={{ disabled: !!disabled }} hitSlop={4}
      style={({ pressed }) => [styles.iconButton, pressed && { backgroundColor: c.hover }, disabled && styles.dim]}>
      <Icon name={icon} size={16} color={c.muted} />
    </Pressable>
  );
}

function PrimaryButton({ label, loading, disabled, onPress }: { label: string; loading?: boolean; disabled?: boolean; onPress: () => void }) {
  const c = useSettingsColors();
  const off = disabled || loading;
  return (
    <Pressable onPress={onPress} disabled={off} accessibilityRole="button" accessibilityLabel={label}
      accessibilityState={{ disabled: !!off, busy: !!loading }}
      style={({ pressed }) => [styles.primary, { backgroundColor: c.accent }, (off || pressed) && styles.dim]}>
      <Text style={styles.primaryText}>{loading ? m.comum_carregando() : label}</Text>
    </Pressable>
  );
}

const styles = StyleSheet.create((theme) => ({
  pad: { paddingHorizontal: 16, paddingVertical: 14 },
  gap: { gap: 12 },
  gapSmall: { gap: 6 },
  formGap: { gap: 18 },
  rowWrap: { flexDirection: 'row', flexWrap: 'wrap', gap: 8 },
  note: { fontSize: 14, lineHeight: 18 },
  small: { fontSize: 13 },
  strong: { fontSize: 14, fontWeight: '600' },
  error: { fontSize: 14, color: theme.tokens.status.error, paddingHorizontal: 4 },
  warning: { fontSize: 13.5, color: theme.tokens.status.warning },
  saved: { fontSize: 13.5, color: theme.tokens.status.success },
  row: { borderTopWidth: 1, paddingVertical: 12, paddingHorizontal: 16, gap: 8 },
  rowHead: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  iconBox: { width: 34, height: 34, borderRadius: 9, borderWidth: 1, alignItems: 'center', justifyContent: 'center' },
  texts: { flex: 1, minWidth: 0, gap: 2 },
  titleLine: { flexDirection: 'row', alignItems: 'center', gap: 8, flexWrap: 'wrap' },
  rowTitle: { fontSize: 15, fontWeight: '500', flexShrink: 1 },
  mark: { paddingHorizontal: 8, paddingVertical: 2, borderRadius: 9999, borderWidth: 1 },
  mono: { fontSize: 13, fontFamily: Platform.select({ ios: 'Menlo', default: 'monospace' }) },
  actions: { flexDirection: 'row', justifyContent: 'flex-end', gap: 4, paddingLeft: 46 },
  iconButton: { width: 36, height: 36, borderRadius: 8, alignItems: 'center', justifyContent: 'center' },
  footer: { borderTopWidth: 1, paddingHorizontal: 16, paddingVertical: 12, gap: 12 },
  restore: { gap: 8 },
  pageFooter: { borderTopWidth: 1, paddingTop: 16, flexDirection: 'row', alignItems: 'center', gap: 12, flexWrap: 'wrap' },
  input: { minHeight: 44, borderWidth: 1, borderRadius: 8, paddingHorizontal: 12, paddingVertical: 10, fontSize: 15 },
  multiline: { minHeight: 72, textAlignVertical: 'top' },
  monoInput: { fontFamily: Platform.select({ ios: 'Menlo', default: 'monospace' }), fontSize: 14 },
  emoji: { width: 130 },
  glyphs: { flexDirection: 'row', flexWrap: 'wrap', gap: 4 },
  glyph: { width: 44, height: 44, borderRadius: 8, alignItems: 'center', justifyContent: 'center' },
  fieldLabel: { fontSize: 14 },
  toggle: { flexDirection: 'row', alignItems: 'center', gap: 12 },
  where: { borderWidth: 1, borderRadius: 10, padding: 16, gap: 8 },
  whereTitle: { fontSize: 15, fontWeight: '600' },
  radio: { width: 14, height: 14, borderRadius: 7 },
  hangarBox: { borderWidth: 1, borderRadius: 10, padding: 16, gap: 12 },
  fileLine: { flexDirection: 'row', alignItems: 'center', gap: 8 },
  fileBody: { padding: 8, borderRadius: 6 },
  buttons: { flexDirection: 'row', justifyContent: 'flex-end', alignItems: 'center', gap: 10 },
  primary: { height: 38, paddingHorizontal: 18, borderRadius: 8, alignItems: 'center', justifyContent: 'center' },
  primaryText: { color: '#fff', fontSize: 15, fontWeight: '600' },
  dim: { opacity: 0.5 },
}));
