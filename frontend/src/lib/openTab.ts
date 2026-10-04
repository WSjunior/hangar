// Abre o link numa aba nova e diz se o navegador deixou. `noopener` nas features faria o
// `window.open` devolver `null` sempre, e aí não haveria como saber se a janela foi bloqueada:
// o `opener` é cortado depois, com a referência na mão.
export function openInNewTab(url: string): boolean {
  const w = window.open(url, '_blank');
  if (!w) return false;
  w.opener = null;
  return true;
}
