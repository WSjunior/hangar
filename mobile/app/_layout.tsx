import 'react-native-url-polyfill/auto';
import '../src/theme/unistyles';
import { useEffect } from 'react';
import { Stack, useRouter, useSegments } from 'expo-router';
import { GestureHandlerRootView } from 'react-native-gesture-handler';
// Sem o KeyboardProvider, KeyboardStickyView/KeyboardChatScrollView lançam em runtime
import { KeyboardProvider } from 'react-native-keyboard-controller';
import { configureCore } from '../src/net/configureCore';
import { useServers } from '../src/stores/servers';
import { aplicarTemaSalvo, useAparencia } from '../src/stores/aparencia';
import { aplicarMaterial } from '../src/theme/aplicarMaterial';
import { Toaster, toast } from '../src/ui/Toast';
import * as m from '../src/paraglide/messages';

configureCore();
aplicarTemaSalvo();
aplicarMaterial(useAparencia.getState());

export default function Layout() {
  const router = useRouter();
  const segments = useSegments();
  const ready = useServers((s) => s.ready);
  const servers = useServers((s) => s.servers);
  const aviso = useServers((s) => s.aviso);
  const limparAviso = useServers((s) => s.limparAviso);

  useEffect(() => {
    void useServers.getState().load();
  }, []);

  useEffect(() => {
    if (!ready) return;
    const onLogin = segments[0] === 'login';
    if (servers.length === 0 && !onLogin) {
      router.replace('/login');
    }
  }, [ready, servers.length, segments]);

  // Aqui, e não na lista de sessões: as duas formas de perder um servidor (keystore que recusou a
  // escrita, token que o servidor rejeitou) acontecem com qualquer tela aberta, inclusive dentro
  // de um chat. O layout é o único lugar montado o tempo todo.
  useEffect(() => {
    if (!aviso) return;
    toast.erro(
      aviso.tipo === 'token'
        ? m.servidores_aviso_token({ label: aviso.label })
        : m.servidores_aviso_persistencia(),
    );
    limparAviso();
  }, [aviso, limparAviso]);

  return (
    <KeyboardProvider>
      <GestureHandlerRootView style={{ flex: 1 }}>
        <Stack screenOptions={{ headerShown: false }}>
          <Stack.Screen name="create" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="config" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/ask" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/activity" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/loop" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/pair" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/files" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/terminal" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/attachments" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/codex-limits" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
          <Stack.Screen name="s/[server]/[name]/bastao" options={{ presentation: 'formSheet', headerShown: false, sheetAllowedDetents: [0.92], sheetGrabberVisible: true }} />
        </Stack>
        <Toaster position="bottom-center" />
      </GestureHandlerRootView>
    </KeyboardProvider>
  );
}
