import { isEmptyBand, type PluginNode } from '@hangar/core';
import * as m from '../paraglide/messages';

// Interface dos mods no CELULAR: oculta por padrão, porque alguns mods ficam grandes demais na tela pequena.
// Preferência do APARELHO (localStorage, prefixo cp_), não do servidor, e vale para todas as sessões. Mesmo padrão
// do navMode: $state + persistência, e a escolha reage na hora, sem reload.
const KEY = 'cp_mods_celular';

// '1' liga; ausente ou '0' desliga. Storage bloqueado (modo privado) vale como desligado.
function ler(): boolean {
  try { return localStorage.getItem(KEY) === '1'; } catch { return false; }
}

let ligado = $state(ler());

export const modsCelular = {
  get ligado() { return ligado; },
  set ligado(v: boolean) {
    ligado = v;
    try { localStorage.setItem(KEY, v ? '1' : '0'); } catch { /* modo privado: vale pela sessão */ }
  },
};

/** Faixa e painéis dos mods aparecem? No desktop sempre; no celular, só com a preferência ligada. */
export function modsNaTela(desktop: boolean, ligadoNoCelular: boolean): boolean {
  return desktop || ligadoNoCelular;
}

/** O item do menu "⋯": só no celular e só quando a sessão tem algo de mod (faixa ou painel); `null` é sem item. */
export function itemModsCelular(desktop: boolean, band: PluginNode, paineis: number): { paineis: number } | null {
  return !desktop && (!isEmptyBand(band) || paineis > 0) ? { paineis } : null;
}

/** Rótulo do item: o destino da troca, com a contagem de painéis quando a interface está oculta. */
export function rotuloModsCelular(ligadoNoCelular: boolean, paineis: number): string {
  if (ligadoNoCelular) return m.mods_celular_ocultar();
  if (paineis === 1) return m.mods_celular_mostrar_um();
  return paineis > 1 ? m.mods_celular_mostrar_paineis({ n: String(paineis) }) : m.mods_celular_mostrar();
}
