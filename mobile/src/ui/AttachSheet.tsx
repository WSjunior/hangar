import { useRef } from 'react';
import { Image, Pressable, ScrollView, Text, View } from 'react-native';
import { StyleSheet } from 'react-native-unistyles';
import { Sheet } from './Sheet';
import { Icon, type IconName } from './Icon';
import { pickCamera, pickFile, pickImage, type PickedAttachment } from './attachmentPicker';
import { useSettingsColors } from '../features/config/colors';
import * as m from '../paraglide/messages';

type Props = {
  open: boolean;
  onClose: () => void;
  /** Mesmo destino de antes: quem chama só adota o anexo escolhido. */
  onPick: (picked: PickedAttachment) => void;
  onError: (message: string) => void;
  /** Só na conversa com anexos já enviados; ausente, o item não aparece. */
  onSessionAttachments?: () => void;
  /**
   * Faixa de fotos recentes. Vem de quem tiver o expo-media-library no build; ausente ou vazia, a
   * faixa mostra só os cartões Câmera e Fotos.
   */
  recentPhotos?: PickedAttachment[];
  /** Abre a lista de comandos da sessão; ausente, o item não aparece. */
  onCommands?: () => void;
  /** Estilo do ditado vigente e a ação de trocá-lo; ausente, o item não aparece. */
  dictationStyle?: { label: string; onPress: () => void };
  /** Mandar também pro par/grupo; ausente (sessão sem par), o item não aparece. */
  sendToGroup?: { label: string; on: boolean; onPress: () => void };
};

const TILE = 112;

export function AttachSheet({ open, onClose, onPick, onError, onSessionAttachments, recentPhotos, onCommands, dictationStyle, sendToGroup }: Props) {
  const c = useSettingsColors();
  // A ação roda depois que a folha some: o iOS recusa abrir câmera/galeria por cima de uma folha
  // ainda em animação de saída.
  const afterDismiss = useRef<(() => void) | null>(null);

  const choose = (action: () => void) => {
    afterDismiss.current = action;
    onClose();
  };
  const run = (picker: () => Promise<PickedAttachment | null>) => () =>
    choose(() => {
      picker()
        .then((picked) => { if (picked) onPick(picked); })
        .catch((e: unknown) => onError(e instanceof Error && e.message ? e.message : m.board_falha_upload()));
    });

  const camera = run(pickCamera);
  const photos = run(pickImage);
  const hasRecent = !!recentPhotos?.length;

  // Comandos e estilo do ditado também abrem outra folha: só depois que esta sumir.
  const rows: { icon: IconName; label: string; value?: string; onPress: () => void }[] = [
    { icon: 'File', label: m.composer_adicionar_arquivos(), onPress: run(pickFile) },
    ...(onSessionAttachments
      ? [{ icon: 'Paperclip' as IconName, label: m.ctx_anexos_da_sessao(), onPress: () => choose(onSessionAttachments) }]
      : []),
    ...(onCommands ? [{ icon: 'SquareSlash' as IconName, label: m.comandos_titulo(), onPress: () => choose(onCommands) }] : []),
    ...(dictationStyle
      ? [{ icon: 'AudioLines' as IconName, label: m.ditado_estilo_titulo(), value: dictationStyle.label, onPress: () => choose(dictationStyle.onPress) }]
      : []),
    ...(sendToGroup
      ? [{ icon: 'ArrowLeftRight' as IconName, label: sendToGroup.label, value: sendToGroup.on ? '✓' : undefined, onPress: () => choose(sendToGroup.onPress) }]
      : []),
  ];

  const tile = (icon: IconName, label: string, onPress: () => void, grow: boolean) => (
    <Pressable
      key={label}
      onPress={onPress}
      accessibilityRole="button"
      accessibilityLabel={label}
      style={({ pressed }) => [styles.tile, grow ? styles.tileGrow : styles.tileFixed, { backgroundColor: c.inset, borderColor: c.border }, pressed && styles.pressed]}
    >
      <Icon name={icon} size={26} color={c.text} />
      <Text style={[styles.tileLabel, { color: c.text }]}>{label}</Text>
    </Pressable>
  );

  return (
    <Sheet
      open={open}
      sizes={['auto']}
      onDismiss={() => {
        const action = afterDismiss.current;
        afterDismiss.current = null;
        onClose();
        action?.();
      }}
    >
      <View style={styles.body}>
        <View style={styles.header}>
          <Pressable
            onPress={() => choose(() => {})}
            hitSlop={8}
            accessibilityRole="button"
            accessibilityLabel={m.native_close()}
            style={({ pressed }) => [styles.close, { backgroundColor: c.inset, borderColor: c.border }, pressed && styles.pressed]}
          >
            <Icon name="X" size={18} color={c.text} />
          </Pressable>
          <Text style={[styles.title, { color: c.text }]} numberOfLines={1} accessibilityRole="header">
            {m.composer_adicionar_ao_chat()}
          </Text>
          {/* Sem fotos recentes a galeria já é o cartão da faixa: o link repetiria a mesma ação. */}
          {hasRecent ? (
            <Pressable onPress={photos} hitSlop={8} accessibilityRole="button" style={styles.link}>
              <Text style={[styles.linkText, { color: c.accent }]}>{m.composer_fotos()}</Text>
            </Pressable>
          ) : <View style={styles.link} />}
        </View>

        {hasRecent ? (
          <ScrollView horizontal showsHorizontalScrollIndicator={false} contentContainerStyle={styles.strip}>
            {tile('Camera', m.composer_camera(), camera, false)}
            {recentPhotos!.map((p) => (
              <Pressable
                key={p.uri}
                onPress={() => choose(() => onPick(p))}
                accessibilityRole="imagebutton"
                accessibilityLabel={p.name}
                style={({ pressed }) => [styles.tileFixed, styles.photo, pressed && styles.pressed]}
              >
                <Image source={{ uri: p.uri }} style={styles.photoImg} />
              </Pressable>
            ))}
          </ScrollView>
        ) : (
          <View style={styles.strip}>
            {tile('Camera', m.composer_camera(), camera, true)}
            {tile('Images', m.composer_fotos(), photos, true)}
          </View>
        )}

        <View style={[styles.group, { backgroundColor: c.inset, borderColor: c.border }]}>
          {rows.map((row, i) => (
            <Pressable
              key={row.label}
              onPress={row.onPress}
              accessibilityRole="button"
              accessibilityLabel={row.label}
              accessibilityValue={row.value ? { text: row.value } : undefined}
              style={({ pressed }) => [styles.row, i > 0 && { borderTopWidth: StyleSheet.hairlineWidth, borderTopColor: c.borderStrong }, pressed && { backgroundColor: c.hover }]}
            >
              <Icon name={row.icon} size={20} color={c.text} />
              <Text style={[styles.rowLabel, styles.rowGrow, { color: c.text }]}>{row.label}</Text>
              {row.value ? <Text style={[styles.rowValue, { color: c.muted }]} numberOfLines={1}>{row.value}</Text> : null}
            </Pressable>
          ))}
        </View>
      </View>
    </Sheet>
  );
}

const styles = StyleSheet.create((theme) => ({
  body: { paddingHorizontal: theme.base.space[4], paddingBottom: theme.base.space[6], gap: theme.base.space[4] },
  header: { flexDirection: 'row', alignItems: 'center', justifyContent: 'space-between', minHeight: 40 },
  close: {
    width: 36,
    height: 36,
    borderRadius: 18,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: 'center',
    justifyContent: 'center',
  },
  // Absoluto para ficar no centro da folha, não no espaço que sobra entre o X e o link.
  title: {
    position: 'absolute',
    left: 56,
    right: 56,
    textAlign: 'center',
    fontSize: theme.base.text.base,
    fontWeight: '600',
  },
  link: { minHeight: 36, justifyContent: 'center' },
  linkText: { fontSize: theme.base.text.base, fontWeight: '500' },
  strip: { flexDirection: 'row', gap: theme.base.space[2] },
  tile: {
    height: TILE,
    borderRadius: 22,
    borderWidth: StyleSheet.hairlineWidth,
    alignItems: 'center',
    justifyContent: 'center',
    gap: theme.base.space[2],
  },
  tileGrow: { flex: 1 },
  tileFixed: { width: TILE },
  tileLabel: { fontSize: theme.base.text.sm, fontWeight: '600' },
  photo: { height: TILE, borderRadius: 22, overflow: 'hidden' },
  photoImg: { width: '100%', height: '100%' },
  group: { borderRadius: 22, borderWidth: StyleSheet.hairlineWidth, overflow: 'hidden' },
  row: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[3],
    minHeight: 52,
    paddingHorizontal: theme.base.space[4],
  },
  rowLabel: { fontSize: theme.base.text.base },
  rowGrow: { flex: 1 },
  rowValue: { flexShrink: 1, maxWidth: '45%', fontSize: theme.base.text.sm },
  pressed: { opacity: 0.6 },
}));
