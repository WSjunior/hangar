import { useCallback, useEffect, useRef, useState } from 'react';
import { Pressable, Switch, Text, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { codexOpcoes, type Server } from '@hangar/core';
import * as m from '../../paraglide/messages';

export function CodexContextControl({ server, onBusy }: { server: Server | null; onBusy: (busy: boolean) => void }) {
  const [enabled, setEnabled] = useState(false);
  const [ready, setReady] = useState(false);
  const [busy, setBusy] = useState(true);
  const [error, setError] = useState('');
  const controller = useRef<AbortController | null>(null);

  const update = useCallback(async (control: AbortController, value?: boolean) => {
    setBusy(true);
    onBusy(true);
    setError('');
    try {
      const result = await codexOpcoes(server, control.signal, value);
      if (controller.current !== control || control.signal.aborted) return;
      setEnabled(result.contexto_estendido);
      setReady(true);
    } catch (e) {
      if (controller.current === control && !control.signal.aborted) {
        setError(e instanceof Error ? e.message : m.comum_falha_aplicar());
      }
    } finally {
      if (controller.current === control && !control.signal.aborted) {
        setBusy(false);
        onBusy(false);
      }
    }
  }, [server, onBusy]);

  useEffect(() => {
    const control = new AbortController();
    controller.current = control;
    setReady(false);
    void update(control);
    return () => { control.abort(); onBusy(false); };
  }, [update, onBusy]);

  return (
    <View style={styles.root}>
      <View style={styles.row}>
        <Text style={styles.label}>{m.codex_contexto_titulo()}</Text>
        <Switch accessibilityLabel={m.codex_contexto_titulo()} value={enabled} disabled={busy || !ready}
          onValueChange={(value) => { if (controller.current) void update(controller.current, value); }} />
      </View>
      <Text style={styles.hint}>{m.codex_contexto_padrao()}</Text>
      {busy ? <Text style={styles.hint}>{m.comum_carregando()}</Text> : null}
      {error ? <>
        <Text style={styles.error} accessibilityRole="alert">{error}</Text>
        <Pressable accessibilityRole="button" disabled={busy} style={styles.retry}
          onPress={() => { if (controller.current) void update(controller.current); }}>
          <Text style={styles.label}>{m.config_server_tentar_de_novo()}</Text>
        </Pressable>
      </> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  root: { gap: theme.base.space[2] },
  row: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', minHeight: 44, gap: 12 },
  label: { color: theme.tokens.text.primary, flexShrink: 1 },
  hint: { color: theme.tokens.text.secondary, fontSize: 12 },
  error: { color: theme.tokens.status.error, fontSize: 12 },
  retry: { minHeight: 44, justifyContent: 'center' },
}));
