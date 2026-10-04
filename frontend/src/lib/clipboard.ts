// Copia texto pro clipboard, com fallback execCommand pra LAN via HTTP puro (onde a Clipboard API
// nao existe fora de contexto seguro). Um lugar so -> bubbles e sidebar nao duplicam o fallback.
// Devolve se copiou: fora de um gesto da pessoa (depois de um `await`) os dois caminhos recusam.
export async function copyText(s: string): Promise<boolean> {
  try {
    await navigator.clipboard.writeText(s);
    return true;
  } catch {
    const ta = document.createElement('textarea');
    ta.value = s;
    ta.style.position = 'fixed';
    ta.style.opacity = '0';
    document.body.appendChild(ta);
    ta.select();
    const ok = document.execCommand('copy');
    ta.remove();
    return ok;
  }
}
