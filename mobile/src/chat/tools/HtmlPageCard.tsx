import { memo, useCallback, useEffect, useMemo, useRef, useState } from 'react';
import { Linking, Pressable, Text, View, useWindowDimensions, type LayoutChangeEvent } from 'react-native';
import { StyleSheet, useUnistyles } from 'react-native-unistyles';
import { WebView, type WebViewMessageEvent } from 'react-native-webview';
import {
  baseOf, fileAuthHeader, frameHeight, pageFetchState, pageUrls, reservedHeight, themeVariables,
  type HtmlPageRef, type PageFetchState, type ThemeTokens,
} from '@hangar/core';
import { useServers } from '../../stores/servers';
import { toast } from '../../ui/Toast';
import * as m from '../../paraglide/messages';

type ThemeParams = { theme: 'dark' | 'light'; styles: { variables: Record<string, string> } };

// Nome da variável da página → token do app (o mesmo mapa da PWA). Fonte fica com a padrão da
// página: as do app não são nomes de CSS.
const tokenVars = (t: ThemeTokens): Record<string, string> => ({
  '--foreground': t.text.primary, '--muted-foreground': t.text.muted, '--surface': t.bg.surface,
  '--border': t.border.default, '--accent': t.accent.base, '--accent-foreground': t.text.inverse,
  '--danger': t.status.error, '--warning': t.status.warning, '--success': t.status.success,
  '--code-background': t.bg.base, '--chart-1': t.chart[0], '--chart-2': t.chart[1], '--chart-3': t.chart[2],
  '--chart-4': t.chart[3],
});

// Troca só o conteúdo do bloco de tema que o servidor injeta; o data-base fica para as trocas seguintes.
function themed(html: string, p: ThemeParams): string {
  const css = `:root{color-scheme:${p.theme};${Object.entries(p.styles.variables).map(([k, v]) => `${k}:${v}`).join(';')}}`;
  return html.replace(/(<style id="hangar-theme" data-base="([^"]*)">)[\s\S]*?(<\/style>)/,
    (_all, open: string, base: string, close: string) => `${open}${css}${base.replaceAll('&quot;', '"')}${close}`);
}

const applyScript = (p: ThemeParams) => `window.__hangarApply&&window.__hangarApply(${JSON.stringify(p)});true`;

// Só o documento inicial carrega; qualquer navegação da página (link, location, window.open) para.
// ponytail: no Android a lib libera a navegação se o JS não responder em 250 ms (JS travado deixa
// passar); fechar isso de vez exige filtro nativo.
const onlyInitial = (r: { url: string }) => r.url === 'about:blank' || r.url.startsWith('about:srcdoc');

// Página publicada pelo html_render no lugar da chamada, como o cartão do nativo (page_card.rs):
// sem moldura nem título, só a página sobre o papel de parede; carregando ocupa a altura reservada,
// erro e expirada viram uma linha de texto apagado. A mensagem da página é do agente, não confiável:
// só altura finita e link http(s) passam.
export const HtmlPageCard = memo(function HtmlPageCard({ page, sessionName, serverId }: {
  page: HtmlPageRef;
  sessionName: string;
  serverId?: string;
}) {
  const { theme, rt } = useUnistyles();
  // A página mora na máquina da sessão, que nem sempre é a ativa.
  const server = useServers((s) => (serverId ? s.servers.find((x) => x.id === serverId) ?? null : s.active()));
  const { width: screen } = useWindowDimensions();
  const [width, setWidth] = useState(0);
  const [state, setState] = useState<PageFetchState>('loading');
  const [html, setHtml] = useState('');
  const [reported, setReported] = useState<number | null>(null);
  const web = useRef<WebView>(null);

  const scheme = rt.themeName === 'light' ? 'light' : 'dark';
  const params = useMemo<ThemeParams>(() => {
    const vars = tokenVars(theme.tokens);
    return { theme: scheme, styles: { variables: themeVariables((n) => vars[n] ?? '') } };
  }, [scheme, theme.tokens]);
  const paramsRef = useRef(params);
  // Tema próprio: a página fica como foi desenhada, sem as cores do app.
  const own = page.ownTheme;

  // Tema mudou com a página aberta: ela recebe as variáveis novas sem recarregar.
  useEffect(() => {
    paramsRef.current = params;
    if (!own) web.current?.injectJavaScript(applyScript(params));
  }, [params, own]);

  const seq = useRef(0);
  const load = useCallback(async () => {
    const n = ++seq.current;
    setState('loading');
    try {
      if (!server) throw new Error('no server');
      const r = await fetch(pageUrls(baseOf(server), sessionName, page.id).raw, { headers: fileAuthHeader(server) });
      // O 404 da página vencida traz o código; sem ele (convidado, rota ausente) é erro.
      const code = r.status === 404 ? await r.json().then((b) => b?.detail?.code, () => null) : null;
      const next = pageFetchState(r.status, code);
      const text = next === 'ready' ? await r.text() : '';
      const doc = text && !own ? themed(text, paramsRef.current) : text;
      if (n !== seq.current) return;
      setHtml(doc);
      setState(next);
    } catch {
      if (n === seq.current) setState(pageFetchState('network'));
    }
  }, [server, sessionName, page.id, own]);

  useEffect(() => {
    void load();
    return () => { seq.current++; };
  }, [load]);

  const onMessage = useCallback((e: WebViewMessageEvent) => {
    let d: { method?: unknown; params?: { height?: unknown; url?: unknown } } | null;
    try { d = JSON.parse(e.nativeEvent.data); } catch { return; }
    const h = d?.params?.height;
    const url = d?.params?.url;
    if (d?.method === 'ui/notifications/size-changed' && typeof h === 'number' && Number.isFinite(h)) setReported(h);
    else if (d?.method === 'ui/open-link' && typeof url === 'string' && /^https?:\/\//i.test(url)) {
      Linking.openURL(url).catch((err: unknown) => toast.erro(err instanceof Error ? err.message : String(err)));
    }
  }, []);

  const onLayout = useCallback((e: LayoutChangeEvent) => setWidth(e.nativeEvent.layout.width), []);
  const w = width || screen - 2 * theme.base.space[4];
  const height = frameHeight(page, w, reported);
  // Altura fixa pedida pelo agente e conteúdo maior que ela: a página rola por dentro.
  const inside = page.height != null && (reported ?? reservedHeight(page, w)) > height;

  if (state === 'expired') return <Text style={styles.note} selectable>{m.page_expired()}</Text>;
  if (state === 'error') {
    return (
      <View style={styles.errorRow} accessibilityRole="alert">
        <Text style={[styles.note, styles.errorText]} selectable>{m.page_error({ title: page.title })}</Text>
        <Pressable onPress={() => void load()} style={styles.retry} accessibilityRole="button" hitSlop={8}>
          <Text style={styles.retryText}>{m.page_retry()}</Text>
        </Pressable>
      </View>
    );
  }
  return (
    <View style={[styles.page, { height }]} onLayout={onLayout} accessibilityLabel={page.title}>
      {state === 'loading' ? (
        <View style={styles.loading} accessibilityRole="progressbar">
          <Text style={styles.note}>{m.page_loading({ title: page.title })}</Text>
        </View>
      ) : (
        <WebView
          ref={web}
          source={{ html }}
          style={styles.web}
          // Com '*' toda navegação passa pelo filtro abaixo; com uma lista menor, o que ficasse de fora
          // a própria lib abriria no Linking sem gesto, em qualquer esquema.
          originWhitelist={['*']}
          onShouldStartLoadWithRequest={onlyInitial}
          // Sem janela nova: o window.open vira navegação no próprio quadro e o filtro barra.
          setSupportMultipleWindows={false}
          onMessage={onMessage}
          onLoadEnd={() => { if (!own) web.current?.injectJavaScript(applyScript(paramsRef.current)); }}
          scrollEnabled={inside}
          nestedScrollEnabled={inside}
          bounces={false}
          overScrollMode="never"
          showsVerticalScrollIndicator={false}
          showsHorizontalScrollIndicator={false}
          automaticallyAdjustContentInsets={false}
        />
      )}
    </View>
  );
});

const styles = StyleSheet.create((theme) => ({
  page: { width: '100%', borderRadius: 8, borderCurve: 'continuous', overflow: 'hidden', backgroundColor: 'transparent' },
  web: { flex: 1, backgroundColor: 'transparent' },
  loading: { flex: 1, justifyContent: 'center' },
  note: { paddingVertical: theme.base.space[3], fontSize: theme.base.text.xs, color: theme.tokens.text.muted },
  errorRow: { flexDirection: 'row', alignItems: 'center', gap: theme.base.space[2] },
  errorText: { flexShrink: 1 },
  retry: { minHeight: 44, justifyContent: 'center', paddingHorizontal: theme.base.space[1] },
  retryText: { fontSize: theme.base.text.xs, fontWeight: '600', color: theme.tokens.accent.base },
}));
