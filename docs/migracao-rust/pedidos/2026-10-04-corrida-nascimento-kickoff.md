# Mensagem enviada a uma sessão que ainda está nascendo é recusada (regressão da 2D)

## O defeito (uso real do dono, notebook, 04/10 ~11:10)

O dono criou a sessão `Projetos` pela "Nova conversa" (Claude com terminal, motor) e mandou a
primeira mensagem enquanto ela ainda nascia. Resposta: "vínculo gerenciado indisponível; escrita
suspensa". Segundos depois funcionou. Antes da 2D, a mensagem esperava na fila e saía quando a
sessão ficava pronta.

Evidência do notebook (backend.log): pane criado 11:10:41; 11:10:42 `RESOLVE name=Projetos … prev=-`,
transcript ainda inexistente, e o SSE fechou com "sessão em transferência; aguarde a posse ser
confirmada"; estado `terminal_a025…` criado 11:10:42 e uma entrega confirmada logo depois. O diário
estava cheio (teto de 4 MB) e não registrou o POST recusado.

## Causa provável (conferir no código, não confiar neste resumo)

`LegacyBridge.binding` (`backend/app/runtime_adapter.py` ~223-230): sem sidecar headless com `key`,
provider claude vai para `runtime_terminal.resolve_binding`, que devolve `None` enquanto os fatos do
terminal (pane, agente, prova da sessão) ainda não existem. Aí
`RuntimeCoordinator.prepare_session` (`backend/app/runtime_coordinator.py` ~190-197) levanta
"vínculo gerenciado indisponível; escrita suspensa" porque `outside_scope` não confirma que a
sessão está fora do escopo. Veio em `bead8a45` (2D). Confira também a mesma janela para sessão SEM
terminal recém-criada (sidecar ainda sendo gravado) e o "sessão em transferência" do SSE.

## O que fazer

1. Reproduza com teste que falha sem o conserto: sessão com terminal recém-criada (pane existe,
   agente/transcript ainda não) recebe envio → hoje recusa.
2. Conserte pela causa, seguindo a regra da migração e o comportamento do Python antigo: sessão que
   está nascendo não é falha nem suspensão. A mensagem espera o vínculo (ou entra na fila e sai
   quando a sessão fica pronta), uma vez só, sem duplicar. Suspensão de escrita fica só para o caso
   em que o vínculo existia e se perdeu (o motivo pelo qual a 2D a criou: leia a spec/verificação da
   2D em `docs/migracao-rust/parte2d/` antes de mudar). Sessão que de fato não existe responde como
   antes ("sessão não encontrada"), nunca "escrita suspensa".
3. Testes reais, só Haiku (`--model claude-haiku-4-5`), conta `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`,
   backend de teste isolado desta branch: `HOME` temporário, portas livres diferentes de
   8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L <nome>` próprio (a branch já tem `bb80cf31`).
   Casos: criar sessão com terminal e mandar mensagem imediatamente (repita 5 vezes, e com o
   `hangar-server` de teste no ar); idem sessão sem terminal; mensagem para nome que não existe.
   Cada mensagem entregue uma vez, confirmada no transcript. Anote em
   `docs/migracao-rust/parte2d/corrida-nascimento.md`. No fim, pare tudo e apague o `HOME`.
4. `cargo test --locked --workspace` se mexer no Rust; pytest dos arquivos do runtime e do terminal
   (falhas conhecidas desta máquina: `omp` não instalado; 3 de `test_update_channel` que dependem da
   ordem). Revisão independente por subagente antes do push.

## Entrega

Branch `fix/terminal-birth-race` (este cwd, de `origin/hangar-server-parte1` `ac5c975d`). Commits em
inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só desta branch e CI do
`server.yml` verde nos três sistemas. Mande para `migracao-rust-2`: causa, conserto, commits, testes
com números, casos reais, link do CI. Não junte na `hangar-server-parte1`.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux
padrão ou em sessão real; rodar instaladores; mexer na `main` ou na `hangar-server-parte1`; modelo
diferente de Haiku; log com texto de conversa.
