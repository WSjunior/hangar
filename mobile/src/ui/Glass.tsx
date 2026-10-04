import { useEffect, useState } from 'react';
import { AccessibilityInfo, Platform, View, type ViewProps } from 'react-native';
import { BlurView } from 'expo-blur';
import { GlassView, isLiquidGlassAvailable } from 'expo-glass-effect';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { useBlurTarget } from './blurTarget';
import { solidezDe } from '../theme/superficie';

type Props = ViewProps & { variant?: 'panel' | 'modal' | 'chrome' };
export type GlassVariant = NonNullable<Props['variant']>;

// Todo painel de vidro anda com o slider de Transparência, não só o `panel`: o slider é a razão
// entre o que a pessoa pediu e o padrão de fábrica, e ela multiplica o alpha próprio de cada
// variante. Com o slider no padrão a conta devolve exatamente os valores de hoje. Piso 0.3 pelo
// mesmo motivo do piso do store: abaixo disso não dá pra ler o que está escrito por cima.
// O `chrome` (composer, caixa de Nova conversa) é a caixa sobre o fundo agora que a lista não tem
// painel: segue a Solidez, na mesma conta da superficie().
export function alphaDoVidro(
  theme: {
    panelAlpha: number;
    surfaceAlpha: number;
    tokens: { glass: { panelAlpha: number; solidAlpha: number; modalAlpha: number } };
  },
  variant: GlassVariant,
): number {
  const { panelAlpha, solidAlpha, modalAlpha } = theme.tokens.glass;
  if (variant === 'chrome') {
    return Math.max(0.3, Math.min(1, solidAlpha * solidezDe(theme.surfaceAlpha)));
  }
  const base = variant === 'modal' ? modalAlpha : panelAlpha;
  return Math.max(0.3, Math.min(1, base * (theme.panelAlpha / panelAlpha)));
}

export function useReduceTransparency() {
  const [on, setOn] = useState(false);
  useEffect(() => {
    AccessibilityInfo.isReduceTransparencyEnabled().then(setOn).catch(() => setOn(false));
    const sub = AccessibilityInfo.addEventListener('reduceTransparencyChanged', setOn);
    return () => sub.remove();
  }, []);
  return on;
}

// Um componente só decide o material: iOS 26 = vidro de verdade; iOS antigo = blur; Android = blur ou cor com alpha.
export function Glass({ variant = 'panel', style, children, ...rest }: Props) {
  const { theme, rt } = useUnistyles();
  const reduzir = useReduceTransparency();
  const alvoBlur = useBlurTarget();
  const [r, g, b] = theme.tokens.glass.panelRgb;
  const bg = `rgba(${r},${g},${b},${alphaDoVidro(theme, variant)})`;
  // O material é só a camada de fundo, irmã do children dentro da MESMA View: trocar tema ou ligar
  // "Reduzir transparência" com o composer aberto remonta o fundo, nunca o input (foco e rascunho).
  let fundo;
  if (reduzir) {
    fundo = <View style={[StyleSheet.absoluteFillObject, { backgroundColor: `rgb(${r},${g},${b})` }]} pointerEvents="none" />;
  } else if (Platform.OS === 'ios' && isLiquidGlassAvailable()) {
    // key pelo tema: o material do glass-effect não acompanha troca de tema em runtime.
    // O chrome flutua sobre a conversa: tinta fraca deixava a caixa lavada e o texto de trás vazando;
    // inteira apaga a refração e o vidro vira caixa chapada.
    const tinta = variant === 'chrome' ? `rgba(${r},${g},${b},${alphaDoVidro(theme, variant) * 0.75})` : bg;
    fundo = <GlassView key={rt.themeName} glassEffectStyle="regular" tintColor={tinta} isInteractive={variant === 'chrome'} style={StyleSheet.absoluteFillObject} pointerEvents="none" />;
  } else {
    fundo = (
      <BlurView
        intensity={40}
        blurMethod="dimezisBlurView"
        // Sem alvo (tela que não passa pela Screen) o Android cai em "none" — o rgba abaixo cobre o caso.
        blurTarget={alvoBlur ?? undefined}
        tint={rt.themeName === 'dark' ? 'dark' : 'light'}
        style={StyleSheet.absoluteFillObject}
        pointerEvents="none"
      >
        <View style={[StyleSheet.absoluteFillObject, { backgroundColor: bg }]} />
      </BlurView>
    );
  }
  return (
    <View style={[styles.box, style]} {...rest}>
      {fundo}
      {children}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  box: {
    borderRadius: theme.base.radius.lg,
    borderWidth: 1,
    borderColor: theme.tokens.glass.border,
    overflow: 'hidden',
  },
}));
