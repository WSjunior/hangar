import { useEffect, useRef, useState } from 'react';
import { Linking, Pressable, Text, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { kindOf, isPermission, questionParts } from '@hangar/core';
import * as m from '../paraglide/messages';
import { superficie } from '../theme/superficie';
import { Icon } from '../ui/Icon';
import { semEmoji } from '../ui/semEmoji';
import { useAparencia } from '../stores/aparencia';

interface Props {
  question: string;
  options: string[];
  onSelect: (n: number) => void | Promise<void>;
  onCancel: () => void | Promise<void>;
  /** Envia as opções marcadas (só múltipla escolha). Ausente = sem botão de enviar. */
  onSubmit?: () => void | Promise<void>;
}

// MÚLTIPLA ESCOLHA: o TUI desenha a caixinha antes do rótulo ("[ ] Alfa", "[✔] Alfa"), e só por ela
// dá pra saber. Marcar e enviar são ações diferentes: o toque alterna, quem envia é o botão.
const CAIXA = /^\[(.?)\]\s*/;
const marcada = (o: string) => /^\[[^\s\]]\]/.test(o);

export function OptionButtons({ question, options, onSelect, onCancel, onSubmit }: Props) {
  const { theme } = useUnistyles();
  const permission = isPermission(options);
  const kinds = options.map((o) => kindOf(o));
  const multipla = options.some((o) => CAIXA.test(o));
  const marcadas = options.filter(marcada).length;
  const locked = useRef(false);
  const mounted = useRef(true);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  // Aparência › Destaque das perguntas: a moldura do pedido que espera a pessoa.
  const destaque = useAparencia((s) => s.destaquePergunta);
  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);
  async function run(action: () => void | Promise<void>) {
    if (locked.current) return;
    locked.current = true;
    setBusy(true);
    setError('');
    try { await action(); }
    catch (e) {
      if (mounted.current) setError(e instanceof Error ? e.message : m.comum_falha_envio_opcao());
    } finally {
      locked.current = false;
      if (mounted.current) setBusy(false);
    }
  }

  return (
    <View style={[styles.wrap, { borderColor: destaque === 'amber' ? theme.tokens.status.warning : theme.tokens.accent.base }]}>
      {permission ? (
        <View style={[styles.permChip, { backgroundColor: theme.tokens.accent.dim }]}>
          <Icon name="ShieldAlert" size={13} color={theme.tokens.accent.base} />
          <Text style={[styles.permTxt, { color: theme.tokens.accent.base }]}>{semEmoji(m.permissao_pedido())}</Text>
        </View>
      ) : null}
      <Text style={[styles.question, { color: theme.tokens.text.primary }]}>
        {questionParts(question).map((part, i) =>
          part.kind === 'code' ? (
            <Text key={i} style={[styles.qCode, { backgroundColor: superficie(theme, 0.8), color: theme.tokens.text.primary }]}>
              {part.text}
            </Text>
          ) : part.kind === 'link' ? (
            <Text
              key={i}
              accessibilityRole="link"
              style={{ color: theme.tokens.accent.base, textDecorationLine: 'underline' }}
              onPress={() => void Linking.openURL(part.text).catch((e: unknown) => setError(e instanceof Error ? e.message : m.comum_falha_envio_opcao()))}
            >
              {part.text}
            </Text>
          ) : (
            part.text
          ),
        )}
      </Text>
      <View style={styles.list}>
        {options.map((opt, i) => {
          const kind = kinds[i];
          const isAllow = permission && kind === 'allow';
          const isAlways = permission && kind === 'always';
          const isDeny = permission && kind === 'deny';
          return (
            <Pressable
              key={i}
              onPress={() => void run(() => onSelect(i + 1))}
              disabled={busy}
              accessibilityState={{ disabled: busy, busy, checked: multipla ? marcada(opt) : undefined }}
              style={[
                styles.btn,
                { backgroundColor: superficie(theme, 0.8), borderColor: theme.tokens.border.default },
                isAllow && { backgroundColor: theme.tokens.accent.base, borderColor: theme.tokens.accent.base },
                isAlways && { backgroundColor: theme.tokens.accent.dim, borderColor: theme.tokens.accent.base },
                isDeny && { borderColor: theme.tokens.status.error },
              ]}
              accessibilityRole="button"
            >
              <Text
                style={[
                  styles.num,
                  { color: theme.tokens.text.secondary },
                  isAllow && { color: '#fff' },
                  isAlways && { color: theme.tokens.accent.base },
                  isDeny && { color: theme.tokens.status.error },
                ]}
              >
                {i + 1}.
              </Text>
              {multipla ? (
                <View
                  style={[styles.caixa, { borderColor: theme.tokens.border.default }, marcada(opt) && { backgroundColor: theme.tokens.accent.base, borderColor: theme.tokens.accent.base }]}
                  accessibilityElementsHidden
                  importantForAccessibility="no-hide-descendants"
                >
                  {marcada(opt) ? <Icon name="Check" size={12} color="#fff" /> : null}
                </View>
              ) : null}
              <Text
                style={[
                  styles.optTxt,
                  { color: theme.tokens.text.primary },
                  isAllow && { color: '#fff' },
                  isAlways && { color: theme.tokens.accent.base },
                  isDeny && { color: theme.tokens.status.error },
                ]}
              >
                {multipla ? opt.replace(CAIXA, '') : opt}
              </Text>
            </Pressable>
          );
        })}
        {multipla && onSubmit ? (
          <Pressable
            onPress={() => void run(onSubmit)}
            disabled={busy || marcadas === 0}
            accessibilityState={{ disabled: busy || marcadas === 0, busy }}
            style={[styles.btn, { backgroundColor: theme.tokens.accent.base, borderColor: theme.tokens.accent.base }, marcadas === 0 && styles.off]}
            accessibilityRole="button"
          >
            <Icon name="Send" size={14} color="#fff" />
            <Text style={[styles.optTxt, { color: '#fff' }]}>{m.opcoes_enviar_marcadas({ n: marcadas })}</Text>
          </Pressable>
        ) : null}
        <Pressable
          onPress={() => void run(onCancel)}
          disabled={busy}
          accessibilityState={{ disabled: busy, busy }}
          style={[styles.btn, styles.btnCancel, { borderColor: theme.tokens.status.error }]}
          accessibilityRole="button"
        >
          <Text style={[styles.num, { color: theme.tokens.status.error }]}>✕</Text>
          <Text style={[styles.optTxt, { color: theme.tokens.status.error }]}>{m.comum_cancelar()}</Text>
        </Pressable>
      </View>
      {error ? <Text accessibilityRole="alert" style={{ color: theme.tokens.status.error }}>{error}</Text> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  wrap: {
    padding: theme.base.space[3],
    gap: theme.base.space[3],
    borderWidth: 1,
    borderRadius: 14,
  },
  permChip: {
    alignSelf: 'flex-start',
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
    borderRadius: theme.base.radius.full,
    paddingHorizontal: theme.base.space[2],
    paddingVertical: 4,
  },
  permTxt: {
    fontSize: theme.base.text.xs,
    fontWeight: '600',
  },
  question: {
    fontSize: theme.base.text.base,
    fontWeight: '500',
    lineHeight: 22,
  },
  qCode: {
    fontFamily: theme.base.fontMono,
    fontSize: 14,
    paddingHorizontal: 4,
    borderRadius: 4,
  },
  list: {
    gap: theme.base.space[2],
  },
  btn: {
    minHeight: 44,
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[3],
    paddingHorizontal: theme.base.space[3],
    borderWidth: 1,
    borderRadius: theme.base.radius.lg,
  },
  btnCancel: {
    borderColor: theme.tokens.status.error,
  },
  num: {
    fontFamily: theme.base.fontMono,
    fontSize: theme.base.text.sm,
    minWidth: 20,
  },
  optTxt: {
    fontSize: theme.base.text.base,
    flex: 1,
  },
  caixa: {
    width: 18,
    height: 18,
    borderRadius: 4,
    borderWidth: 1.5,
    alignItems: 'center',
    justifyContent: 'center',
  },
  off: {
    opacity: 0.45,
  },
}));
