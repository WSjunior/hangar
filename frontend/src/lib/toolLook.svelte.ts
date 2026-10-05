// Aparência das chamadas de ferramenta no chat: 'classico' (o bloco de duas linhas de sempre,
// "● Bash <arg>" / "└ Pronto (38 linhas) • clique para ver"), 'terminal' (como o terminal do
// Claude Code: "● Update(arquivo)" / "⎿ resumo" / diff numerado) ou 'chips' (a pele portada do
// beautiful-ui: uma linha por chamada, o argumento num chip, o detalhe abrindo embaixo, e a faixa
// de chips de diff no fim da rodada).
//
// É INTERRUPTOR, não migração: o default é 'classico', então quem não mexer não vê diferença
// nenhuma, e voltar é um clique. As duas peles leem exatamente os MESMOS dados derivados do
// ToolCard — nada de comportamento muda entre elas (o diff da edição, o erro em texto, o realce
// do Read e o anexo de imagem valem nas duas).
//
// Mesmo padrão do navMode/sidebarPrefs: chave no localStorage + $state, reage na hora, sem reload.
const LOOK_KEY = 'cp_tool_look';

export type ToolLook = 'classico' | 'chips' | 'terminal';

// Ausência de chave = 'classico': ninguém é migrado por acidente.
function loadLook(): ToolLook {
  try {
    const v = localStorage.getItem(LOOK_KEY);
    return v === 'chips' || v === 'terminal' ? v : 'classico';
  } catch { return 'classico'; }
}

let look = $state<ToolLook>(loadLook());

export const toolLook = {
  get look() { return look; },
  set look(v: ToolLook) {
    look = v;
    try {
      if (v === 'classico') localStorage.removeItem(LOOK_KEY);
      else localStorage.setItem(LOOK_KEY, v);
    } catch { /* modo privado: vale pela sessão */ }
  },
};
