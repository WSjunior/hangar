import { useState } from 'react';
import { Platform, Pressable, Text, TextInput, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { useRouter } from 'expo-router';
import { MenuView } from '@react-native-menu/menu';
import { serverColor } from '@hangar/core';
import { Pagina } from '../../src/features/config/Pagina';
import { SettingsMenuGroup, SettingsMenuItem } from '../../src/features/config/SettingsMenu';
import { useSettingsColors } from '../../src/features/config/colors';
import { Icon, type IconName } from '../../src/ui/Icon';
import { useServers } from '../../src/stores/servers';
import * as m from '../../src/paraglide/messages';

type Page = { route: string; icon: IconName; title: () => string; rows: Array<() => string> };

// Ordem e ícones do Rust (settings.rs), sem as páginas só de desktop. `rows` são os
// títulos de linha que a busca acha, que só abre a página.
const DEVICE: Page[] = [
  { route: '/config/geral', icon: 'Globe', title: m.native_settings_page_general,
    rows: [m.config_idioma_rotulo, m.lista_agrupar] },
  { route: '/config/aparencia', icon: 'Palette', title: m.native_settings_page_appearance,
    rows: [m.config_tema_curto, m.native_settings_accent, m.config_fundo_curto, m.config_fundo_escolher,
      m.config_fundo_transparencia, m.config_fundo_solidez, m.config_aparencia_pensamento_tools] },
  { route: '/config/usage-log', icon: 'FileText', title: m.native_settings_page_diary, rows: [] },
  { route: '/config/sobre', icon: 'Info', title: m.native_settings_page_about,
    rows: [m.atualizar_versao, m.config_sobre_servidor, m.atualizar_versao_servidor] },
];
const SERVER: Page[] = [
  { route: '/config/maquinas', icon: 'Server', title: m.native_settings_page_servers,
    rows: [m.sessao_adicionar_servidor, m.config_maquinas_testar] },
  { route: '/config/sync', icon: 'RefreshCw', title: m.native_settings_page_sync, rows: [] },
  { route: '/config/shared-config', icon: 'Layers', title: m.shared_config_title, rows: [] },
  { route: '/config/contas', icon: 'User', title: m.native_settings_page_accounts,
    rows: [m.contas_atualizar, m.contas_secao_claude, m.contas_secao_modelos, m.contas_secao_outros] },
  { route: '/config/orchestration', icon: 'Users', title: m.native_settings_page_orchestration, rows: [] },
  { route: '/config/harnesses', icon: 'Activity', title: m.native_settings_page_harnesses, rows: [] },
  { route: '/config/voice', icon: 'Mic', title: m.native_settings_page_voice, rows: [] },
  { route: '/config/jev', icon: 'Zap', title: m.jev_title, rows: [] },
  { route: '/config/notifications', icon: 'Bell', title: m.native_settings_page_notifications, rows: [] },
  { route: '/config/shortcuts', icon: 'Keyboard', title: m.native_settings_page_shortcuts, rows: [] },
  { route: '/config/attachments', icon: 'Paperclip', title: m.native_settings_page_attachments, rows: [] },
  { route: '/config/worktrees', icon: 'GitBranch', title: m.worktrees_titulo, rows: [] },
  { route: '/config/advanced', icon: 'SlidersHorizontal', title: m.native_settings_page_advanced, rows: [] },
];

/** Minúsculas e sem acento, para "aparencia" achar "Aparência". */
const fold = (s: string) => s.normalize('NFD').replace(/[̀-ͯ]/g, '').toLowerCase();

function search(query: string) {
  const q = fold(query.trim());
  const found: Array<{ key: string; route: string; label: string }> = [];
  for (const p of [...DEVICE, ...SERVER]) {
    const title = p.title();
    if (fold(title).includes(q)) found.push({ key: p.route, route: p.route, label: title });
    for (const row of p.rows) {
      const r = row();
      if (fold(r).includes(q)) found.push({ key: `${p.route}:${r}`, route: p.route, label: `${title} › ${r}` });
    }
  }
  return found;
}

export default function ConfigIndex() {
  const router = useRouter();
  const c = useSettingsColors();
  const [query, setQuery] = useState('');
  const open = (route: string) => router.push(route as never);
  const searching = query.trim() !== '';
  const found = searching ? search(query) : [];

  return (
    <Pagina>
      <View style={[styles.search, { borderColor: c.border, backgroundColor: c.inset }]}>
        <Icon name="Search" size={14} color={c.faint} />
        <TextInput
          value={query}
          onChangeText={setQuery}
          placeholder={m.native_settings_search()}
          placeholderTextColor={c.faint}
          accessibilityLabel={m.native_settings_search()}
          autoCorrect={false}
          autoCapitalize="none"
          clearButtonMode="while-editing"
          returnKeyType="search"
          style={[styles.searchInput, { color: c.text }]}
        />
      </View>

      {searching ? (
        <View style={styles.list}>
          {found.length === 0 ? (
            <Text style={[styles.none, { color: c.muted }]}>{m.native_settings_search_none()}</Text>
          ) : (
            found.map((f) => <SettingsMenuItem key={f.key} label={f.label} onPress={() => open(f.route)} />)
          )}
        </View>
      ) : (
        <View style={styles.list}>
          <SettingsMenuGroup title={m.native_settings_group_device()} />
          {DEVICE.map((p) => <SettingsMenuItem key={p.route} icon={p.icon} label={p.title()} onPress={() => open(p.route)} />)}
          <SettingsMenuGroup title={m.native_settings_group_server()} right={<ServerPicker />} />
          {SERVER.map((p) => <SettingsMenuItem key={p.route} icon={p.icon} label={p.title()} onPress={() => open(p.route)} />)}
        </View>
      )}
    </Pagina>
  );
}

/** "• nome ⌄" do grupo Servidor: troca a máquina que o grupo configura, como a lista de Servidores. */
function ServerPicker() {
  const c = useSettingsColors();
  const servers = useServers((s) => s.servers);
  const activeId = useServers((s) => s.activeId);
  const active = servers.find((s) => s.id === activeId) ?? null;
  if (!active) return null;
  const label = (
    <View style={styles.picker}>
      <View style={[styles.dot, { backgroundColor: serverColor(active.id) }]} />
      <Text style={[styles.pickerText, { color: c.text }]} numberOfLines={1}>{active.label}</Text>
      {servers.length > 1 ? <Icon name="ChevronDown" size={12} color={c.muted} /> : null}
    </View>
  );
  // Uma máquina só: nada para trocar, o nome fica sem a seta.
  if (servers.length < 2) return label;
  return (
    <MenuView
      title={m.native_settings_group_server()}
      actions={servers.map((s) => ({
        id: s.id,
        title: s.label,
        state: s.id === activeId ? ('on' as const) : ('off' as const),
        // O menu nativo do Android só aceita ícone de recurso; a bolinha de cor fica no iOS.
        ...(Platform.OS === 'ios' ? { image: 'circle.fill', imageColor: serverColor(s.id) } : {}),
      }))}
      onPressAction={({ nativeEvent }) => useServers.getState().setActive(nativeEvent.event)}
    >
      <Pressable accessibilityRole="button" accessibilityLabel={m.native_settings_switch_server()} hitSlop={8}>
        {label}
      </Pressable>
    </MenuView>
  );
}

const styles = StyleSheet.create({
  search: {
    height: 40,
    flexDirection: 'row',
    alignItems: 'center',
    gap: 8,
    paddingHorizontal: 10,
    borderWidth: 1,
    borderRadius: 6,
  },
  searchInput: { flex: 1, height: 40, fontSize: 15, padding: 0 },
  list: { gap: 2 },
  none: { paddingHorizontal: 10, paddingVertical: 8, fontSize: 14 },
  picker: { maxWidth: 180, flexDirection: 'row', alignItems: 'center', gap: 6 },
  dot: { width: 7, height: 7, borderRadius: 4 },
  pickerText: { flexShrink: 1, fontSize: 13.5 },
});
