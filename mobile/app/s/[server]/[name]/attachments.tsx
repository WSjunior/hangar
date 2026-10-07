import { useCallback, useEffect, useState } from 'react';
import { ActivityIndicator, Pressable, ScrollView, Text, View } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { useLocalSearchParams, useRouter } from 'expo-router';
import { WebView } from 'react-native-webview';
import * as m from '../../../../src/paraglide/messages';
import { fileUrlNative, fileAuthHeader, listUploads, uploadUrlNative } from '@hangar/core';
import type { UploadFile } from '@hangar/core';
import { AttachmentCard } from '../../../../src/features/attachments/AttachmentCard';
import { Lightbox } from '../../../../src/features/attachments/Lightbox';
import { fileKind } from '@hangar/core';
import { useServers } from '../../../../src/stores/servers';
import { superficie } from '../../../../src/theme/superficie';
import { AudioChip } from '../../../../src/chat/AudioChip';
import { DocumentViewer, type DocumentItem } from '../../../../src/features/attachments/DocumentViewer';
import { DictationError, dictateUpload } from '../../../../src/features/ditado/dictationRun';
import { useDitadoEstiloStore } from '../../../../src/features/ditado/ditadoEstiloStore';
import { useSessions } from '../../../../src/stores/sessions';
import { toast } from '../../../../src/ui/Toast';

export default function AttachmentsSheet() {
  const { theme } = useUnistyles();
  const router = useRouter();
  const { server, name } = useLocalSearchParams<{ server: string; name: string }>();
  const sessionName = String(name ?? '');
  const serverId = String(server ?? '');

  const [files, setFiles] = useState<UploadFile[]>([]);
  const [loading, setLoading] = useState(true);
  const [erro, setErro] = useState<string | null>(null);
  const [lightbox, setLightbox] = useState<UploadFile | null>(null);
  const [doc, setDoc] = useState<DocumentItem | null>(null);
  const [docUrl, setDocUrl] = useState<{ url: string; headers?: Record<string, string>; title: string } | null>(null);
  const [webErro, setWebErro] = useState<string | null>(null);
  const [webCarregando, setWebCarregando] = useState(true);
  const ready = useServers((s) => s.ready);
  const servidorSumiu = !ready ? false : !useServers.getState().servers.some((s) => s.id === serverId);

  const load = useCallback(async () => {
    setLoading(true);
    setErro(null);
    try {
      const r = await listUploads(sessionName);
      setFiles(r.files);
    } catch (e) {
      setErro(e instanceof Error ? e.message : String(e));
    } finally {
      setLoading(false);
    }
  }, [sessionName]);

  useEffect(() => {
    if (!ready) return;
    if (!useServers.getState().ensureActive(serverId)) return;
    void load();
  }, [load, ready, serverId]);

  useEffect(() => {
    setWebErro(null);
    setWebCarregando(true);
  }, [docUrl?.url]);

  const handlePress = useCallback(
    (f: UploadFile) => {
      const k = fileKind(f.filename);
      if (k === 'image') {
        setLightbox(f);
        return;
      }
      if (k === 'html' || k === 'pdf') {
        try {
          router.push(`/s/${serverId}/${sessionName}/files?path=${encodeURIComponent(f.filename)}` as never);
          return;
        } catch {
          const url = fileUrlNative(sessionName, f.filename);
          setDocUrl({ url, headers: fileAuthHeader(), title: f.filename });
          return;
        }
      }
      if (k === 'video') {
        setDoc({ uri: uploadUrlNative(sessionName, f.filename), headers: fileAuthHeader(), name: f.filename, kind: 'video' });
        return;
      }
      const url = uploadUrlNative(sessionName, f.filename);
      setDocUrl({ url, headers: fileAuthHeader(), title: f.filename });
    },
    [sessionName, serverId, router],
  );

  // Volta para a conversa: o Composer mostra "transcrevendo" e recebe o texto ou o motivo da falha.
  const retranscribe = useCallback((f: UploadFile) => {
    const target = useServers.getState().servers.find((s) => s.id === serverId);
    if (!target) { toast.erro(m.chat_servidor_removido()); return; }
    const sessions = useSessions.getState();
    const transcript = (sessions.byServerRecord?.[serverId]?.find((x) => x.name === sessionName)?.jsonl
      ?? sessions.rows.find((x) => x.serverId === serverId && x.name === sessionName)?.jsonl) || null;
    const style = useDitadoEstiloStore.getState();
    let run: Promise<unknown>;
    try {
      run = dictateUpload(target, serverId, sessionName, transcript, f.filename, style.pronto ? style.valor : undefined);
    } catch (e) {
      toast.erro(e instanceof Error ? e.message : m.composer_falha_transcricao());
      return;
    }
    // A falha guardada já fica no aviso do ditado da conversa; só a que não foi guardada vira toast.
    run.catch((e) => {
      if (e instanceof DictationError && (e.lost || e.storageIssue)) {
        toast.erro(e.storageIssue ? `${e.message} (${e.storageIssue})` : e.message);
      }
    });
    router.back();
  }, [serverId, sessionName, router]);

  const audios = files.filter((f) => fileKind(f.filename) === 'audio');
  const others = files.filter((f) => fileKind(f.filename) !== 'audio');

  if (docUrl) {
    const semToken = !docUrl.headers || Object.keys(docUrl.headers).length === 0;
    if (semToken) {
      return (
        <View style={[styles.container, { backgroundColor: theme.tokens.bg.base }]}>
          <View style={[styles.bar, { borderBottomColor: theme.tokens.border.subtle }]}>
            <Text style={[styles.docTitle, { color: theme.tokens.text.primary }]} numberOfLines={1}>
              {docUrl.title}
            </Text>
            <Pressable onPress={() => setDocUrl(null)} style={[styles.docBtn, { borderColor: theme.tokens.border.subtle }]} accessibilityRole="button" accessibilityLabel={m.anexos_fechar_visualizacao()}>
              <Text style={{ color: theme.tokens.text.primary }}>✕</Text>
            </Pressable>
          </View>
          <Text style={[styles.err, { color: theme.tokens.status.error }]} accessibilityRole="alert">
            {m.sessao_expirada()}
          </Text>
        </View>
      );
    }
    return (
      <View style={[styles.container, { backgroundColor: theme.tokens.bg.base }]}>
        <View style={[styles.bar, { borderBottomColor: theme.tokens.border.subtle }]}>
          <Text style={[styles.docTitle, { color: theme.tokens.text.primary }]} numberOfLines={1}>
            {docUrl.title}
          </Text>
          <Pressable onPress={() => setDocUrl(null)} style={[styles.docBtn, { borderColor: theme.tokens.border.subtle }]} accessibilityRole="button" accessibilityLabel={m.anexos_fechar_visualizacao()}>
            <Text style={{ color: theme.tokens.text.primary }}>✕</Text>
          </Pressable>
        </View>
        {webCarregando && !webErro ? (
          <View style={{ flexDirection: 'row', alignItems: 'center', gap: 8, padding: 12 }}>
            <ActivityIndicator color={theme.tokens.text.muted} />
            <Text style={[styles.muted, { color: theme.tokens.text.muted }]}>{m.comum_carregando()}</Text>
          </View>
        ) : null}
        {webErro ? (
          <Text style={[styles.err, { color: theme.tokens.status.error }]} accessibilityRole="alert">
            {webErro}
          </Text>
        ) : null}
        <WebView
          source={{ uri: docUrl.url, headers: docUrl.headers }}
          style={styles.webview}
          onLoadStart={() => {
            setWebCarregando(true);
          }}
          onLoad={() => setWebCarregando(false)}
          onError={(e) => {
            setWebCarregando(false);
            setWebErro(e.nativeEvent.description || m.arquivo_carregar_erro());
          }}
          onHttpError={(e) => {
            setWebCarregando(false);
            const descricao = e.nativeEvent.description;
            setWebErro(descricao ? `HTTP ${e.nativeEvent.statusCode}: ${descricao}` : m.arquivo_carregar_erro());
          }}
        />
      </View>
    );
  }

  if (!ready) {
    return (
      <View style={[styles.container, { backgroundColor: theme.tokens.bg.base, flex: 1, alignItems: 'center', justifyContent: 'center' }]}>
        <ActivityIndicator color={theme.tokens.text.muted} />
        <Text style={[styles.muted, { color: theme.tokens.text.muted }]}>{m.comum_carregando()}</Text>
      </View>
    );
  }

  if (servidorSumiu) {
    return (
      <View style={[styles.container, { backgroundColor: theme.tokens.bg.base, flex: 1, alignItems: 'center', justifyContent: 'center', gap: 8 }]}>
        <Text style={[styles.err, { color: theme.tokens.status.error }]} accessibilityRole="alert">
          {m.arq_sessao_encerrada()}
        </Text>
      </View>
    );
  }

  return (
    <View style={[styles.container, { backgroundColor: theme.tokens.bg.base }]}>
      <Text style={[styles.title, { color: theme.tokens.text.primary }]}>
        {m.ctx_anexos()}
        {files.length ? <Text style={[styles.count, { color: theme.tokens.text.secondary, backgroundColor: superficie(theme, 0.8) }]}> {files.length}</Text> : null}
      </Text>

      {loading ? (
        <View style={styles.center}>
          <ActivityIndicator color={theme.tokens.text.secondary} />
          <Text style={[styles.muted, { color: theme.tokens.text.muted }]}>{m.comum_carregando()}</Text>
        </View>
      ) : erro ? (
        <View style={styles.center}>
          <Text style={[styles.err, { color: theme.tokens.status.error }]}>{m.anexos_erro_listar()} {erro}</Text>
          <Pressable onPress={() => void load()} style={[styles.retryBtn, { borderColor: theme.tokens.border.subtle }]} accessibilityRole="button">
            <Text style={[styles.retryText, { color: theme.tokens.accent.base }]}>{m.lista_tentar_novamente()}</Text>
          </Pressable>
        </View>
      ) : files.length === 0 ? (
        <Text style={[styles.muted, { color: theme.tokens.text.muted }]}>{m.anexos_nenhum()}</Text>
      ) : (
        <ScrollView contentContainerStyle={styles.list}>
          {audios.map((f) => (
            <View key={f.filename} style={styles.audioRow}>
              <AudioChip uri={uploadUrlNative(sessionName, f.filename)} headers={fileAuthHeader()} name={f.filename} />
              <Pressable onPress={() => retranscribe(f)} hitSlop={8} style={[styles.retryBtn, { borderColor: theme.tokens.border.subtle }]}
                accessibilityRole="button" accessibilityLabel={`${m.composer_transcrever_de_novo()}: ${f.filename}`}>
                <Text style={[styles.retryText, { color: theme.tokens.accent.base }]}>{m.composer_transcrever_de_novo()}</Text>
              </Pressable>
            </View>
          ))}
          <View style={styles.grid}>
            {others.map((f) => (
              <AttachmentCard key={f.filename} file={f} sessionName={sessionName} onPress={() => handlePress(f)} />
            ))}
          </View>
        </ScrollView>
      )}

      <Lightbox
        visible={!!lightbox}
        uri={lightbox ? uploadUrlNative(sessionName, lightbox.filename) : ''}
        headers={fileAuthHeader()}
        filename={lightbox?.filename ?? ''}
        onClose={() => setLightbox(null)}
      />
      <DocumentViewer doc={doc} onClose={() => setDoc(null)} />
    </View>
  );
}

const styles = StyleSheet.create((theme) => ({
  container: {
    flex: 1,
    padding: theme.base.space[4],
    gap: theme.base.space[3],
  },
  title: {
    fontSize: theme.base.text.base,
    fontWeight: '600',
  },
  count: {
    fontSize: 12,
    fontWeight: '700',
    paddingHorizontal: 8,
    paddingVertical: 2,
    borderRadius: theme.base.radius.full,
    overflow: 'hidden',
  },
  center: {
    flex: 1,
    alignItems: 'center',
    justifyContent: 'center',
    gap: theme.base.space[2],
    paddingVertical: theme.base.space[4],
  },
  muted: {
    fontSize: theme.base.text.sm,
    textAlign: 'center',
  },
  err: {
    fontSize: theme.base.text.sm,
    textAlign: 'center',
  },
  retryBtn: {
    borderWidth: 1,
    borderRadius: theme.base.radius.full,
    paddingHorizontal: theme.base.space[3],
    paddingVertical: 6,
  },
  retryText: {
    fontSize: theme.base.text.sm,
    fontWeight: '600',
  },
  list: {
    gap: theme.base.space[3],
    paddingBottom: theme.base.space[4],
  },
  grid: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    gap: theme.base.space[3],
  },
  audioRow: {
    flexDirection: 'row',
    flexWrap: 'wrap',
    alignItems: 'center',
    gap: theme.base.space[2],
  },
  bar: {
    flexDirection: 'row',
    alignItems: 'center',
    gap: theme.base.space[2],
    paddingBottom: theme.base.space[2],
    borderBottomWidth: 1,
  },
  docTitle: {
    flex: 1,
    fontSize: theme.base.text.sm,
    fontWeight: '600',
  },
  docBtn: {
    width: 36,
    height: 36,
    borderRadius: 18,
    borderWidth: 1,
    alignItems: 'center',
    justifyContent: 'center',
  },
  webview: {
    flex: 1,
  },
}));
