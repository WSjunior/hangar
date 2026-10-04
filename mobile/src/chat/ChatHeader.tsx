import { Pressable, Text, View } from 'react-native';
import { Pressable as EdgePressable } from 'react-native-gesture-handler';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { useLocalSearchParams } from 'expo-router';
import { cwdParts, rotuloEstado, type State, type ThemeTokens } from '@hangar/core';
import * as m from '../paraglide/messages';
import { Icon } from '../ui/Icon';
import { HangarMark } from '../ui/HangarMark';
import { WorkingMark } from '../ui/WorkingMark';
import { superficie } from '../theme/superficie';
import { useSessions } from '../stores/sessions';
import { useServers } from '../stores/servers';

const PILL: Record<State, keyof ThemeTokens['pill']> = {
  working: 'working',
  idle: 'idle',
  awaiting_input: 'input',
  dead: 'dead',
};

// Cabeçalho do chat no layout do nativo: ‹ · marca na cor do estado · nome e "pasta @ servidor" ·
// pílula de estado · ⋯. A cor fica só na marca e no ponto da pílula, que é onde ela diz algo; o resto
// é texto. O anel de contexto mora no composer e o terminal no ⋯. O loop desce para uma linha
// própria: na mesma linha espremia o nome da sessão até as reticências.
export function ChatHeader({
  name,
  state,
  onBack,
  onMore,
  onTitlePress,
  chipLoop,
}: {
  name: string;
  state: State | null;
  onBack: () => void;
  onMore: () => void;
  onTitlePress: () => void;
  chipLoop?: React.ReactNode;
}) {
  const { theme } = useUnistyles();
  // Destino lido da rota e da lista: quem monta o cabeçalho não precisa repassar máquina e pasta.
  const params = useLocalSearchParams<{ server?: string | string[] }>();
  const serverId = Array.isArray(params.server) ? params.server[0] : params.server;
  const row = useSessions((s) => s.rows.find((r) => r.serverId === serverId && r.name === name) ?? null);
  const serverLabel = useServers((s) => s.servers.find((x) => x.id === serverId)?.label) ?? row?.serverLabel ?? '';
  const pasta = row?.cwd ? cwdParts(row.cwd).base : '';
  const destino = pasta && serverLabel ? `${pasta} @ ${serverLabel}` : pasta || serverLabel;
  const corEstado = (state && theme.tokens.pill?.[PILL[state]]?.fg) || theme.tokens.text.muted;
  return (
    <View style={styles.wrap}>
      <View style={styles.bar}>
        {/* Mora na faixa de arrasto da gaveta: no Android o gesto nativo dela engolia o toque de um
            Pressable comum. O do gesture-handler entra na mesma disputa e o toque parado vence. */}
        <EdgePressable
          onPress={onBack}
          hitSlop={8}
          style={styles.back}
          accessibilityRole="button"
          accessibilityLabel={m.chat_voltar_sessoes()}
        >
          <Icon name="ChevronLeft" size={22} color={theme.tokens.text.primary} />
        </EdgePressable>
        <Pressable
          onPress={onTitlePress}
          style={({ pressed }) => [styles.titulo, pressed && styles.tocado]}
          accessibilityRole="button"
          // Nome, estado e destino completos para o leitor de tela; a ação vai na dica.
          accessibilityLabel={[name, state ? rotuloEstado(state) : '', serverLabel, row?.cwd].filter(Boolean).join(', ')}
          accessibilityHint={m.sessao_trocar_de()}
        >
          {state === 'working' ? <WorkingMark size={18} color={corEstado} /> : <HangarMark size={18} color={corEstado} />}
          <View style={styles.textos}>
            <Text style={[styles.name, { color: theme.tokens.text.primary }]} numberOfLines={1}>
              {name}
            </Text>
            {destino ? (
              <Text style={[styles.destino, { color: theme.tokens.text.muted }]} numberOfLines={1}>
                {destino}
              </Text>
            ) : null}
          </View>
        </Pressable>
        {/* "Pronta" é o estado normal e não diz nada: a marca já mostra. Trabalhando, aguardando ou
            encerrada ganham a pílula. */}
        {state && state !== 'idle' ? (
          <View style={[styles.pill, { backgroundColor: superficie(theme, 0.8) }]}>
            <View style={[styles.ponto, { backgroundColor: corEstado }]} />
            <Text style={[styles.pillTxt, { color: theme.tokens.text.primary }]} numberOfLines={1}>
              {rotuloEstado(state)}
            </Text>
          </View>
        ) : null}
        <Pressable
          onPress={onMore}
          hitSlop={8}
          style={styles.more}
          accessibilityRole="button"
          accessibilityLabel={m.navbar_mais_acoes()}
        >
          <Icon name="Ellipsis" size={18} color={theme.tokens.text.secondary} />
        </Pressable>
      </View>
      {chipLoop ? <View style={styles.chips}>{chipLoop}</View> : null}
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  wrap: {
    borderBottomWidth: 1,
    borderBottomColor: theme.tokens.border.subtle,
  },
  bar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[1],
    paddingHorizontal: theme.base.space[1],
    paddingVertical: theme.base.space[1],
    minHeight: 48,
  },
  // Linha discreta do loop, alinhada ao nome (depois do ‹ e da marca).
  chips: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[2],
    paddingLeft: 78, // coluna do nome: recuo + ‹ + marca + vãos
    paddingRight: theme.base.space[2],
    paddingBottom: theme.base.space[1],
  },
  titulo: {
    flex: 1,
    flexShrink: 1,
    minWidth: 0,
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[2],
    minHeight: 44,
    paddingHorizontal: theme.base.space[1],
    borderRadius: theme.base.radius.md,
  },
  textos: {
    flex: 1,
    minWidth: 0,
    justifyContent: 'center',
  },
  destino: {
    fontSize: theme.base.text.xxs,
  },
  tocado: {
    backgroundColor: theme.tokens.bg.hover,
  },
  back: {
    width: 40,
    height: 44,
    alignItems: 'center',
    justifyContent: 'center',
  },
  name: {
    fontSize: theme.base.text.base,
    fontWeight: '600',
  },
  pill: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: 5,
    paddingHorizontal: 9,
    paddingVertical: 3,
    borderRadius: theme.base.radius.full,
    flexShrink: 0,
  },
  ponto: {
    width: 6,
    height: 6,
    borderRadius: 3,
  },
  pillTxt: {
    fontSize: theme.base.text.xxs,
  },
  more: {
    width: 44,
    height: 44,
    alignItems: 'center',
    justifyContent: 'center',
  },
}));
