import * as m from '../../paraglide/messages';

// Nomes dos modos como o app de PC escreve. Modo que o app não conhece (CLI novo) sai como o id cru:
// dado, não interface.
const LABEL: Record<string, () => string> = {
  plan: m.native_mode_plan,
  auto: m.native_mode_auto,
  manual: m.native_mode_manual,
  acceptEdits: m.native_mode_acceptEdits,
  bypassPermissions: m.native_mode_bypassPermissions,
  dontAsk: m.native_mode_dontAsk,
};

export const permissionLabel = (mode: string) => LABEL[mode]?.() ?? mode;
