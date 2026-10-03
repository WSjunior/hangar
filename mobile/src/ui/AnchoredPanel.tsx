import { useEffect, useRef, useState, type ReactNode } from 'react';
import { AccessibilityInfo, Modal, Platform, Pressable, View, useWindowDimensions } from 'react-native';
import Animated, { Easing, useAnimatedStyle, useReducedMotion, useSharedValue, withSpring, withTiming } from 'react-native-reanimated';
import { scheduleOnRN } from 'react-native-worklets';
import { useReanimatedKeyboardAnimation } from 'react-native-keyboard-controller';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { Glass } from './Glass';

type Rect = { x: number; y: number; width: number; height: number };

type Props = {
  open: boolean;
  // Gatilho: o painel abre acima dele, alinhado à borda direita.
  anchor: View | null;
  onClose: () => void;
  // Fim da saída animada: devolver o foco ao gatilho.
  onDismissed?: () => void;
  label: string;
  closeLabel: string;
  // Cartão do popover do PC (raio 12, recuo 4) em vez do menu nativo.
  radius?: number;
  padding?: number;
  // Fração da tela que o painel pode ocupar; o espaço acima do gatilho continua sendo o teto.
  maxHeightRatio?: number;
  children: ReactNode;
};

// Offset e margem do Positioner do popover do PC (popup.rs `layer`).
const GAP = 8;
const MAX_WIDTH = 420;
// Raio do menu nativo que as outras pílulas abrem (UIMenu do iOS 26); no Android, o do menu Material.
const MENU_RADIUS = Platform.OS === 'ios' ? 24 : 16;

// Painel flutuante preso a um gatilho, como o popover do app de PC: brota dele sem escurecer o
// fundo, fecha no toque fora, no voltar do Android e sobe acima do teclado.
export function AnchoredPanel({
  open, anchor, onClose, onDismissed, label, closeLabel, radius = MENU_RADIUS, padding = 12, maxHeightRatio = 0.45, children,
}: Props) {
  const { rt, theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const reduced = useReducedMotion();
  const { height: keyboard } = useReanimatedKeyboardAnimation();
  const { height: windowHeight } = useWindowDimensions();
  const progress = useSharedValue(0);
  const [rect, setRect] = useState<Rect | null>(null);
  // Tamanho da camada do Modal: no Android edge-to-edge a janela do Dimensions não bate com ela.
  const [frame, setFrame] = useState<{ width: number; height: number } | null>(null);
  const dismissed = useRef(onDismissed);
  dismissed.current = onDismissed;

  useEffect(() => {
    if (!open) return;
    if (!anchor) { setRect({ x: 0, y: 0, width: 0, height: 0 }); return; }
    anchor.measureInWindow((x, y, width, height) => setRect({ x, y, width, height }));
  }, [open, anchor]);

  const ready = !!rect && !!frame;
  useEffect(() => {
    if (open && ready) {
      progress.value = reduced ? 1 : withSpring(1, { duration: 260, dampingRatio: 0.78 });
      return;
    }
    if (open || !rect) return;
    const done = () => { setRect(null); setFrame(null); dismissed.current?.(); };
    if (reduced) { progress.value = 0; done(); return; }
    progress.value = withTiming(0, { duration: 120, easing: Easing.in(Easing.cubic) }, (finished) => {
      if (finished) scheduleOnRN(done);
    });
  }, [open, ready, rect, reduced, progress]);

  const W = frame?.width ?? 0;
  // A camada do Modal às vezes mede antes de ocupar a tela: o teto saía baixo e a lista sumia.
  const H = Math.max(frame?.height ?? 0, windowHeight);
  const width = Math.max(0, Math.min(W - 2 * GAP, MAX_WIDTH));
  const anchored = rect && rect.width ? rect : { x: W - GAP, y: H * 0.7, width: 0, height: 0 };
  const right = Math.max(GAP, Math.min(W - anchored.x - anchored.width, W - width - GAP));
  const baseBottom = H - anchored.y + GAP;
  // A escala nasce do centro da pílula, no pé do painel.
  const originX = Math.max(0, Math.min(width, anchored.x + anchored.width / 2 - (W - right - width)));
  const top = insets.top + GAP;
  const dark = rt.themeName === 'dark';

  const motion = useAnimatedStyle(() => {
    // Teclado aberto empurra o pé do painel para cima dele; o teto cede para a busca não sumir.
    // Teto em vez de altura fixa: o painel encolhe ao conteúdo, como o menu nativo.
    const bottom = Math.max(baseBottom, -keyboard.value + GAP);
    const p = progress.value;
    return {
      bottom,
      maxHeight: Math.max(160, Math.min(H * maxHeightRatio, H - bottom - top)),
      opacity: Math.min(1, p),
      transform: [{ scale: 0.96 + 0.04 * p }],
    };
  });

  return (
    <Modal
      visible={!!rect}
      transparent
      animationType="none"
      statusBarTranslucent
      navigationBarTranslucent
      backdropColor="transparent"
      onRequestClose={onClose}
      // O leitor de tela já leva o foco à janela nova do Modal; o nome do painel vai falado.
      onShow={() => AccessibilityInfo.announceForAccessibility(label)}
    >
      <View
        style={styles.layer}
        pointerEvents={open ? 'auto' : 'none'}
        onLayout={(e) => setFrame({ width: e.nativeEvent.layout.width, height: e.nativeEvent.layout.height })}
      >
        {/* Fundo sem escurecer, como o menu nativo: só engole o toque fora e fecha. */}
        <Pressable style={StyleSheet.absoluteFill} onPress={onClose} accessibilityRole="button" accessibilityLabel={closeLabel} />
        {ready ? (
          <Animated.View
            style={[
              styles.shadow,
              { right, width, borderRadius: radius, transformOrigin: [originX, '100%', 0] },
              { boxShadow: `0 8px 28px rgba(0,0,0,${dark ? 0.4 : 0.16})` },
              motion,
            ]}
          >
            {/* O vidro dentro do Modal não tem o que desfocar e saía transparente: o que estava atrás
                (cartão de uso) atravessava a lista. A tinta é quase sólida, como o popup_fill do PC:
                menu flutuante sobre papel de parede vivo precisa ser legível. */}
            <View
              pointerEvents="none"
              style={[StyleSheet.absoluteFill, { borderRadius: radius, backgroundColor: `rgba(${theme.tokens.glass.panelRgb.join(',')},${dark ? 0.97 : 0.98})` }]}
            />
            <Glass
              variant="modal"
              accessibilityViewIsModal
              onAccessibilityEscape={onClose}
              style={[styles.panel, { borderRadius: radius, padding }]}
            >
              {children}
            </Glass>
          </Animated.View>
        ) : null}
      </View>
    </Modal>
  );
}

const styles = StyleSheet.create({
  layer: { flex: 1 },
  shadow: { position: 'absolute' },
  panel: { flexShrink: 1 },
});
