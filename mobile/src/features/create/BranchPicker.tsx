import { useEffect, useState } from 'react';
import { Pressable, ScrollView, Text, TextInput, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { getFolderBranchesForServer, type FolderBranches, type Server, type WorktreeChoice } from '@hangar/core';
import { superficie } from '../../theme/superficie';
import * as m from '../../paraglide/messages';

type Mode = 'current' | { existing: string } | 'new';
type Props = { server: Server; cwd: string; sessionName: string; value: WorktreeChoice | null;
               onChange: (v: WorktreeChoice | null) => void };

// O painel desmonta ao fechar: a escolha volta de `value` para não zerar a cada abertura.
const modeOf = (v: WorktreeChoice | null): Mode => (!v ? 'current' : v.new_branch ? 'new' : { existing: v.branch });

export function BranchPicker({ server, cwd, sessionName, value, onChange }: Props) {
  const [info, setInfo] = useState<FolderBranches | null>(null);
  const [mode, setMode] = useState<Mode>(() => modeOf(value));
  const [name, setName] = useState(() => (value?.new_branch && value.branch !== sessionName ? value.branch : ''));
  const [base, setBase] = useState(value?.base ?? '');

  useEffect(() => {
    let vivo = true;
    setInfo(null);
    getFolderBranchesForServer(server, cwd)
      .then((r) => { if (vivo) { setInfo(r); setBase((b) => b || (r.current ?? '')); } })
      .catch(() => { if (vivo) setInfo(null); });   // pasta sem git: o seletor some
    return () => { vivo = false; };
  }, [server, cwd]);

  useEffect(() => {
    if (mode === 'new') onChange({ branch: (name || sessionName).trim(), new_branch: true, base: base || null });
    else if (typeof mode === 'object') onChange({ branch: mode.existing });
    else onChange(null);
  }, [mode, name, base, sessionName, onChange]);

  if (!info) return null;
  const others = [...info.branches, ...info.remotes].filter((b) => b !== info.current);
  const row = (key: string, label: string, on: boolean, onPress: () => void) => (
    <Pressable key={key} onPress={onPress} accessibilityRole="button" accessibilityLabel={label}
      accessibilityState={{ selected: on }} style={[styles.row, on && styles.on]}>
      <Text style={styles.rowTxt} numberOfLines={1}>{label}</Text>
    </Pressable>
  );
  return (
    <ScrollView contentContainerStyle={styles.box} keyboardShouldPersistTaps="handled">
      {row('current', `${m.native_create_checkout_current()}${info.current ? ` · ${info.current}` : ''}`, mode === 'current', () => setMode('current'))}
      {others.map((b) => row(`existing:${b}`, `${b} · ${m.native_create_checkout_worktree()}`,
        typeof mode === 'object' && mode.existing === b, () => setMode({ existing: b })))}
      {row('new', m.worktree_nova_branch({ base: base || info.current || '' }), mode === 'new', () => setMode('new'))}
      {mode === 'new' ? (
        <View style={styles.box}>
          <TextInput accessibilityLabel={m.worktree_nome_branch()} placeholder={sessionName} value={name}
            onChangeText={setName} autoCapitalize="none" autoCorrect={false} style={styles.input}
            placeholderTextColor="#8d8489" />
          <Text style={styles.label}>{m.worktree_base()}</Text>
          {[info.current, ...others].filter((b): b is string => !!b).map((b) => row(`base:${b}`, b, base === b, () => setBase(b)))}
        </View>
      ) : null}
      {mode !== 'current' ? <Text style={styles.help}>{m.worktree_modo()} — {m.worktree_modo_ajuda()}</Text> : null}
    </ScrollView>
  );
}

const styles = StyleSheet.create((theme) => ({
  box: { gap: theme.base.space[2] },
  row: { minHeight: 44, justifyContent: 'center', paddingHorizontal: theme.base.space[3], borderRadius: theme.base.radius.md },
  on: { backgroundColor: theme.tokens.accent.dim },
  rowTxt: { fontSize: theme.base.text.sm, color: theme.tokens.text.primary },
  input: {
    minHeight: 44, paddingHorizontal: theme.base.space[3], borderRadius: theme.base.radius.md, fontSize: 16,
    color: theme.tokens.text.primary, backgroundColor: superficie(theme), borderWidth: 1, borderColor: theme.tokens.border.default,
  },
  label: { color: theme.tokens.text.secondary, fontSize: theme.base.text.sm },
  help: { color: theme.tokens.text.secondary, fontSize: theme.base.text.sm },
}));
