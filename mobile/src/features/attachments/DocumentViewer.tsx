import { useState } from 'react';
import { ActivityIndicator, Modal, Platform, Pressable, Text, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { WebView } from 'react-native-webview';
import { useSafeAreaInsets } from 'react-native-safe-area-context';
import * as m from '../../paraglide/messages';
import { Icon } from '../../ui/Icon';
import { toast } from '../../ui/Toast';
import { canShareFile, shareFile } from './mediaCache';

export interface DocumentItem {
  uri: string;
  headers?: Record<string, string>;
  name: string;
  kind: 'pdf' | 'html' | 'video';
}

// Documento citado (pdf/html) em tela cheia, o par do diálogo do FileAttachment do web. Baixar e
// "nova aba" viram um botão só: o navegador do celular não leva o token, então os dois saem pela
// folha de compartilhar do sistema (Salvar em Arquivos, Abrir em…).
export function DocumentViewer({ doc, onClose }: { doc: DocumentItem | null; onClose: () => void }) {
  const { theme } = useUnistyles();
  const insets = useSafeAreaInsets();
  const [carregando, setCarregando] = useState(true);
  const [erro, setErro] = useState('');
  const [compartilhando, setCompartilhando] = useState(false);
  // Zera no render em que o documento troca: no `onShow` do Modal o WebView já podia ter disparado
  // o `onLoad`, e o "Carregando" ficava preso.
  const [shownUri, setShownUri] = useState(doc?.uri);
  if (doc?.uri !== shownUri) {
    setShownUri(doc?.uri);
    setCarregando(true);
    setErro('');
  }

  const compartilhar = async () => {
    if (!doc) return;
    setCompartilhando(true);
    try {
      await shareFile(doc.uri, doc.headers, doc.name);
    } catch (e) {
      toast.erro(e instanceof Error ? e.message : String(e));
    } finally {
      setCompartilhando(false);
    }
  };

  const semLeitor = doc?.kind === 'pdf' && Platform.OS === 'android';

  return (
    <Modal
      visible={!!doc}
      animationType="slide"
      onRequestClose={onClose}
    >
      {doc ? (
        <View style={[styles.root, { paddingTop: insets.top, paddingBottom: insets.bottom, backgroundColor: theme.tokens.bg.base }]}>
          <View style={styles.bar}>
            <Text style={[styles.name, { color: theme.tokens.text.secondary }]} numberOfLines={1}>{doc.name}</Text>
            {canShareFile ? (
              <Pressable onPress={() => void compartilhar()} disabled={compartilhando} style={styles.btn} accessibilityRole="button"
                accessibilityLabel={m.anexos_baixar({ nome: doc.name })} accessibilityState={{ busy: compartilhando }}>
                {compartilhando ? <ActivityIndicator color={theme.tokens.text.secondary} /> : <Icon name="Share" size={18} color={theme.tokens.text.secondary} />}
              </Pressable>
            ) : null}
            <Pressable onPress={onClose} style={styles.btn} accessibilityRole="button" accessibilityLabel={m.anexos_fechar_visualizacao()}>
              <Icon name="X" size={20} color={theme.tokens.text.secondary} />
            </Pressable>
          </View>
          {semLeitor ? (
            <Text style={[styles.msg, { color: theme.tokens.text.muted }]} accessibilityRole="alert">{m.arq_pdf_sem_leitor()}</Text>
          ) : (
            <>
              {carregando && !erro ? (
                <View style={styles.loading}>
                  <ActivityIndicator color={theme.tokens.text.muted} />
                  <Text style={[styles.msg, { color: theme.tokens.text.muted }]}>{m.comum_carregando()}</Text>
                </View>
              ) : null}
              {erro ? <Text style={[styles.msg, { color: theme.tokens.status.error }]} accessibilityRole="alert">{erro}</Text> : null}
              <WebView
                source={{ uri: doc.uri, headers: doc.headers }}
                style={styles.web}
                onLoadStart={() => setCarregando(true)}
                onLoad={() => setCarregando(false)}
                onError={(e) => { setCarregando(false); setErro(e.nativeEvent.description || m.arquivo_carregar_erro()); }}
                onHttpError={(e) => {
                  setCarregando(false);
                  const d = e.nativeEvent.description;
                  setErro(d ? `HTTP ${e.nativeEvent.statusCode}: ${d}` : m.arquivo_carregar_erro());
                }}
              />
            </>
          )}
        </View>
      ) : null}
    </Modal>
  );
}

const styles = StyleSheet.create((theme) => ({
  root: { flex: 1 },
  bar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[1],
    paddingLeft: theme.base.space[4],
    paddingRight: theme.base.space[2],
    borderBottomWidth: 1,
    borderBottomColor: theme.tokens.border.subtle,
  },
  name: { flex: 1, fontSize: theme.base.text.sm, fontFamily: theme.base.fontMono },
  btn: { width: 44, height: 44, alignItems: 'center', justifyContent: 'center' },
  loading: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[2], padding: theme.base.space[3] },
  msg: { fontSize: theme.base.text.sm, padding: theme.base.space[3] },
  web: { flex: 1, backgroundColor: '#fff' },
}));
