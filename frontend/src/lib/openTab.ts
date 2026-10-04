// Abre o link numa aba nova e diz se o navegador deixou. `noopener`/`noreferrer` nas features
// fariam o `window.open` devolver `null` sempre, e não haveria como saber se a janela foi
// bloqueada. A aba nasce em branco e é ELA que navega, com `no-referrer`: o site aberto não fica
// sabendo o endereço do Hangar, como nos links `rel="noopener noreferrer"` do resto do app.
export function openInNewTab(url: string): boolean {
  const w = window.open('', '_blank');
  if (!w) return false;
  w.opener = null;
  const doc = w.document;
  const policy = doc.createElement('meta');
  policy.name = 'referrer';
  policy.content = 'no-referrer';
  const refresh = doc.createElement('meta');
  refresh.httpEquiv = 'refresh';
  refresh.content = `0;url="${new URL(url).href}"`;
  doc.head.append(policy, refresh);
  return true;
}
