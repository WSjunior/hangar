type ViewerEntry = { id: string; close: () => void; onPop: () => void };

let active: ViewerEntry | null = null;
let nextId = 0;
let pendingClose: Promise<void> | null = null;

export function openViewerHistory(close: () => void): void {
  // Trocar a mídia do mesmo visor não acrescenta outra parada para Voltar.
  if (active) { active.close = close; return; }
  const id = globalThis.crypto?.randomUUID?.() ?? `${Date.now()}-${++nextId}`;
  const entry: ViewerEntry = { id, close, onPop: () => {} };
  history.pushState({ ...history.state, cpViewer: entry.id }, '', location.href);
  entry.onPop = () => {
    if (active !== entry || history.state?.cpViewer === entry.id) return;
    active = null;
    window.removeEventListener('popstate', entry.onPop);
    entry.close();
  };
  active = entry;
  window.addEventListener('popstate', entry.onPop);
}

export function closeViewerHistory(): void {
  const entry = active;
  if (!entry) return;
  active = null;
  window.removeEventListener('popstate', entry.onPop);
  // Voltar já consumiu a entrada. Fechar pela interface só retira a entrada que ainda é nossa.
  if (history.state?.cpViewer !== entry.id) return;
  pendingClose = new Promise<void>((resolve) => {
    const finish = () => {
      window.removeEventListener('popstate', finish);
      pendingClose = null;
      resolve();
    };
    window.addEventListener('popstate', finish);
    history.back();
  });
}

export function waitForViewerHistory(): Promise<void> {
  // Abrir de novo ou executar uma ação espera o Voltar pendente, para ele não fechar a nova tela.
  return pendingClose ?? Promise.resolve();
}
