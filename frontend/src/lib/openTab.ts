// Abre o link numa aba nova e diz se o navegador deixou. `noopener`/`noreferrer` nas features
// fariam o `window.open` devolver `null` sempre, e não haveria como saber se a janela foi
// bloqueada. A aba nasce em branco e é ELA que navega, com `no-referrer`: o site aberto não fica
// sabendo o endereço do Hangar, como nos links `rel="noopener noreferrer"` do resto do app.
// `false` também quando a aba não deixa ser preparada: quem chama mostra o link para tocar.
export function openInNewTab(url: string): boolean {
  let href: string;
  try {
    href = new URL(url).href;
  } catch {
    return false;
  }
  const w = window.open('', '_blank');
  if (!w) return false;
  try {
    w.opener = null;
    const doc = w.document;
    const policy = doc.createElement('meta');
    policy.name = 'referrer';
    policy.content = 'no-referrer';
    const refresh = doc.createElement('meta');
    refresh.httpEquiv = 'refresh';
    refresh.content = `0;url="${href}"`;
    doc.head.append(policy, refresh);
    return true;
  } catch {
    w.close();
    return false;
  }
}
