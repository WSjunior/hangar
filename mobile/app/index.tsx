import { useRef, useState } from 'react';
import { Keyboard, Platform, View } from 'react-native';
import { Pressable } from 'react-native-gesture-handler';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { useRouter } from 'expo-router';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import type { DrawerLayoutMethods } from 'react-native-gesture-handler/ReanimatedDrawerLayout';
import { Screen } from '../src/ui/Screen';
import { Icon } from '../src/ui/Icon';
import { SessionsDrawer } from '../src/features/sessions/SessionsDrawer';
import { ServerSheet } from '../src/features/sessions/ServerSheet';
import { CreateSessionSheet } from '../src/features/create/CreateSessionSheet';
import * as m from '../src/paraglide/messages';

// Como no app de PC: a tela inicial é a Nova conversa e as sessões moram numa gaveta lateral.
export default function Index() {
  const router = useRouter();
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const drawer = useRef<DrawerLayoutMethods>(null);
  const [serversOpen, setServersOpen] = useState(false);
  const closeDrawer = () => drawer.current?.closeDrawer();
  // O keyboardDismissMode da gaveta só age no arrasto; pelo botão o campo seguiria focado atrás dela.
  const openDrawer = () => {
    Keyboard.dismiss();
    drawer.current?.openDrawer();
  };

  // Pressable do gesture-handler: o da barra mora na faixa de arrasto da gaveta, e no Android o
  // gesto nativo dela engolia o toque do Pressable comum (só passava o toque fora da faixa).
  const topButton = (icon: 'PanelLeft' | 'Server' | 'Settings', label: string, onPress: () => void) => (
    <Pressable onPress={onPress} style={styles.icon} accessibilityRole="button" accessibilityLabel={label} hitSlop={4}>
      <Icon name={icon} size={20} color={theme.tokens.text.secondary} />
    </Pressable>
  );

  return (
    <Screen edges={[]}>
      <SessionsDrawer ref={drawer} onClose={closeDrawer} onOpenServers={() => setServersOpen(true)}>
        <View style={styles.bar}>
          {topButton('PanelLeft', m.native_sessions(), openDrawer)}
          <View style={styles.spacer} />
          {topButton('Server', m.maquinas_este_aparelho(), () => setServersOpen(true))}
          {topButton('Settings', m.config_modal_titulo(), () => router.push('/config' as never))}
        </View>
        {/* No iOS o recuo automático mede a caixa na janela só no layout, e aqui essa medida falha
            calada: o teclado tapava a caixa. A distância até o topo é conhecida: margem + barra. */}
        <CreateSessionSheet keyboardOffset={Platform.OS === 'ios' ? insets.top + BAR : undefined} />
      </SessionsDrawer>
      <ServerSheet open={serversOpen} onFechar={() => setServersOpen(false)} />
    </Screen>
  );
}

const BAR = 44;

const styles = StyleSheet.create((theme) => ({
  bar: { flexDirection: 'row', alignItems: 'center', paddingHorizontal: theme.base.space[1] },
  spacer: { flex: 1 },
  icon: { width: BAR, height: BAR, alignItems: 'center', justifyContent: 'center' },
}));
