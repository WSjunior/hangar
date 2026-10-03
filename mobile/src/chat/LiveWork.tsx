import { useEffect, useRef, useState, type ReactNode } from 'react';
import { ActivityIndicator, Animated, Pressable, Text, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { formatElapsed, type AgentRun } from '@hangar/core';
import { HangarMark } from '../ui/HangarMark';
import { Icon } from '../ui/Icon';
import { superficie } from '../theme/superficie';
import * as m from '../paraglide/messages';
import { workingLabel } from './liveWork';

// Bloco "trabalhando" do fim da conversa (porte do fim do MessageList.svelte): linha de spinner com
// o verbo do terminal e os segundos, o raciocínio chegando e os subagentes que ainda rodam.

// Relógio só de quem mostra segundos: o tique re-renderiza a linha, não a lista.
function useAgora(): number {
  const [agora, setAgora] = useState(Date.now());
  useEffect(() => {
    const t = setInterval(() => setAgora(Date.now()), 1000);
    return () => clearInterval(t);
  }, []);
  return agora;
}

function Pulso({ children }: { children: ReactNode }) {
  const op = useRef(new Animated.Value(1)).current;
  useEffect(() => {
    const l = Animated.loop(Animated.sequence([
      Animated.timing(op, { toValue: 0.4, duration: 900, useNativeDriver: true }),
      Animated.timing(op, { toValue: 1, duration: 900, useNativeDriver: true }),
    ]));
    l.start();
    return () => l.stop();
  }, [op]);
  return <Animated.View style={{ opacity: op }}>{children}</Animated.View>;
}

/** "Pensando… 12s": texto e tempo do rótulo do terminal; sem tempo nele, conta do começo do turno. */
export function WorkingLine({ label, since }: { label?: string | null; since: number | null }) {
  const { theme } = useUnistyles();
  const agora = useAgora();
  const lido = workingLabel(label);
  const verbo = lido.text ?? m.native_working_line();
  const tempo = lido.elapsed ?? (since !== null ? formatElapsed(Math.max(0, agora - since) / 1000) : '');
  return (
    <View style={styles.linha} accessibilityRole="text" accessibilityLiveRegion="polite" accessibilityLabel={verbo}>
      <Pulso><HangarMark size={16} color={theme.tokens.accent.base} /></Pulso>
      <Text style={[styles.verbo, { color: theme.tokens.text.secondary }]} numberOfLines={1}>{verbo}</Text>
      {tempo ? <Text style={[styles.tempo, { color: theme.tokens.text.muted }]}>{tempo}</Text> : null}
    </View>
  );
}

/** Raciocínio chegando (Claude sem terminal). Some quando o bloco cai no transcript, então o tempo
 *  conta da montagem. Só a cauda: o texto inteiro a cada pedaço é trabalho à toa. */
export function PensamentoVivo({ texto }: { texto: string }) {
  const { theme } = useUnistyles();
  const inicio = useRef(Date.now()).current;
  const agora = useAgora();
  const cauda = texto.length > 280 ? '…' + texto.slice(-280) : texto;
  return (
    <View style={styles.pensamento} accessibilityLiveRegion="none">
      <View style={styles.linha}>
        <Pulso><Icon name="Brain" size={15} color={theme.tokens.accent.base} /></Pulso>
        <Text style={[styles.verbo, { color: theme.tokens.text.secondary }]}>{m.pensamento_vivo()}</Text>
        <Text style={[styles.tempo, { color: theme.tokens.text.muted }]}>
          {formatElapsed(Math.max(0, agora - inicio) / 1000)}
        </Text>
      </View>
      <Text style={[styles.cauda, { color: theme.tokens.text.muted }]} numberOfLines={4}>{cauda}</Text>
    </View>
  );
}

/** Subagentes ainda rodando, grudados no fim até acabarem de verdade: um Agent em background
 *  devolve resultado na hora e o cartão dele subia com a conversa, com a sessão parecendo parada. */
export function AgentesRodando({ agentes, inicio, onAbrir }: {
  agentes: AgentRun[];
  inicio: (id: string) => number | null;
  onAbrir?: () => void;
}) {
  const { theme } = useUnistyles();
  const agora = useAgora();
  return (
    // Sem região viva: o rótulo de cada agente leva o relógio, e o leitor de tela anunciaria a cada segundo.
    <View style={styles.agentes}>
      {agentes.map((a) => {
        const t0 = inicio(a.id);
        const tempo = t0 !== null ? m.tool_executando_ha({ tempo: formatElapsed(Math.max(0, agora - t0) / 1000) }) : m.estado_em_execucao();
        return (
          <Pressable
            key={a.id}
            onPress={onAbrir}
            disabled={!onAbrir}
            style={({ pressed }) => [styles.agente, { borderColor: theme.tokens.border.subtle }, pressed && { backgroundColor: theme.tokens.bg.hover }]}
            accessibilityRole={onAbrir ? 'button' : undefined}
            accessibilityLabel={[a.description, a.subagentType, tempo].filter(Boolean).join(', ')}
            accessibilityHint={onAbrir ? m.tool_abrir_agente() : undefined}
          >
            <ActivityIndicator size="small" color={theme.tokens.accent.base} />
            <View style={styles.agenteCorpo}>
              <View style={styles.agenteCab}>
                <Text style={[styles.agenteDesc, { color: theme.tokens.text.primary }]} numberOfLines={1}>
                  {a.description || m.atividade_subagente()}
                </Text>
                {a.subagentType ? (
                  <Text style={[styles.tag, { color: theme.tokens.text.secondary, backgroundColor: superficie(theme, 0.8) }]} numberOfLines={1}>
                    {a.subagentType}
                  </Text>
                ) : null}
              </View>
              <Text style={[styles.tempo, { color: theme.tokens.accent.base }]} numberOfLines={1}>{tempo}</Text>
            </View>
            {onAbrir ? <Icon name="ChevronRight" size={16} color={theme.tokens.text.muted} /> : null}
          </Pressable>
        );
      })}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  linha: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[2], minHeight: 32 },
  verbo: { flexShrink: 1, fontSize: theme.base.text.xs },
  tempo: { flexShrink: 0, fontSize: theme.base.text.xxs, fontVariant: ['tabular-nums'] },
  pensamento: { gap: 2 },
  cauda: { fontSize: theme.base.text.xs, fontStyle: 'italic', paddingLeft: 23 },
  agentes: { gap: theme.base.space[1] },
  agente: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[2],
    minHeight: 44,
    paddingHorizontal: theme.base.space[3],
    paddingVertical: theme.base.space[2],
    borderWidth: 1,
    borderRadius: 10,
  },
  agenteCorpo: { flex: 1, minWidth: 0, gap: 2 },
  agenteCab: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[1], minWidth: 0 },
  agenteDesc: { flexShrink: 1, fontSize: theme.base.text.sm },
  tag: { fontFamily: theme.base.fontMono, fontSize: 11, paddingHorizontal: 6, paddingVertical: 1, borderRadius: theme.base.radius.full, overflow: 'hidden' },
}));
