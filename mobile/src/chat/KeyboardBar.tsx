import { Keyboard, Platform, Pressable, View } from 'react-native';
import { useKeyboardState } from 'react-native-keyboard-controller';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { Icon } from '../ui/Icon';
import { superficie } from '../theme/superficie';
import * as m from '../paraglide/messages';

// A faixa do ✓ que o Safari põe acima do teclado: o campo nativo não tem, e sem ela o teclado só
// fechava arrastando a conversa. Fica por último no KeyboardAvoidingView, logo acima do teclado.
export function KeyboardBar() {
  const { theme } = useUnistyles();
  const visible = useKeyboardState((s) => s.isVisible);
  if (Platform.OS !== 'ios' || !visible) return null;
  return (
    <View style={styles.bar}>
      <Pressable
        onPress={() => Keyboard.dismiss()}
        hitSlop={8}
        style={({ pressed }) => [styles.ok, { backgroundColor: superficie(theme, 0.9) }, pressed && { opacity: 0.6 }]}
        accessibilityRole="button"
        accessibilityLabel={m.native_fechar_teclado()}
      >
        <Icon name="Check" size={20} color={theme.tokens.text.primary} />
      </Pressable>
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  bar: { flexDirection: 'row', justifyContent: 'flex-end', paddingHorizontal: theme.base.space[3], paddingVertical: 4 },
  ok: { width: 48, height: 34, borderRadius: 17, alignItems: 'center', justifyContent: 'center' },
}));
