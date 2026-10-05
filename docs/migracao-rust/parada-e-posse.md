# Parada no SIGTERM e chat na troca de dono

Pedido: `pedidos/2026-10-04-parada-e-posse-kickoff.md`. Branch `fix/shutdown-and-ownership`, de
`origin/hangar-server-parte1` (`b14507f5`, com `bb80cf31`), juntada depois com a parte 1 em `20af41c8`.

## Defeito 1: o backend não terminava no SIGTERM

**Causa.** O uvicorn, ao parar, espera todo pedido aberto terminar antes de rodar o lifespan, e sem
`timeout_graceful_shutdown` essa espera não tem prazo. A espera longa do plugin
(`POST /api/plugin/pull`, `ESPERA_S = 25.0`, e o `/ask` com o mesmo teto) não sabia da parada: o
backend só saía quando ela vencia. O teto de parada do systemd de usuário nesta máquina é 10 s
(`TimeoutStopUSec=10s`), então quase toda parada com uma sessão com terminal viva terminava em
SIGKILL. O journal real de 04/10 07:18:33 mostra exatamente isso: `Shutting down` →
`Waiting for background tasks to complete` → SIGKILL 10 s depois. Os SSE não seguravam nada: o
`sse-starlette` fecha sozinho.

**Evidência no backend isolado.** Lançador de teste com remendo só no `uvicorn.Server` para
listar, durante a espera, as tarefas presas (`asyncio.format_call_graph`). Em todas as rodadas sem
conserto sobrou uma tarefa, `POST /api/plugin/pull` parada em `plugin_bridge.pull`, com
`conexões=0 tarefas=1`. Ela saía quando o prazo de 25 s dela vencia: 9,4 s numa rodada, SIGKILL
aos 10 s nas outras.

**Conserto.**
- `rust_server.Server` (subclasse do `uvicorn.Server`, usada no Python atrás do Rust, na reserva do
  `main.py` e na tomada da porta): ao receber o sinal, chama `plugin_bridge.stop_waits()`, que só
  agenda no loop (o sinal pode chegar com a trava `_lock` tomada). As esperas do `/pull` e do
  `/ask` respondem vazio, como na janela que fecha; pedido novo já na parada responde vazio na hora.
  O plugin trata o vazio como de costume e volta a chamar o backend novo. O sinal marca a parada na
  hora: `_entregar` devolve falso a partir daí (o chamador usa o pane), e texto ou resposta que
  chegou à fila junto com o aviso de parada ainda é entregue.
- `timeout_graceful_shutdown=5` no `kw` do uvicorn: outro pedido longo que ainda apareça é
  cancelado aos 5 s, e o lifespan (fila, diário, posse do Rust, canos) sempre roda. Um envio
  cancelado assim pode já ter sido entregue pela thread dele; a fila durável com confirmação é quem
  evita a duplicata.

## Defeito 2: o SSE da sessão sem terminal fechava com "sessão em transferência"

**Causa.** Sessão sem terminal nova (ou recarregada): a primeira mensagem sobe o cano pelo Python, e
logo em seguida o Rust adota a sessão (`adopt`: fase `PreparingRust`, `quiesce` do cliente Python,
que loga `desligou … (cano segue vivo)`). Nessa janela três leituras recusavam com
`RuntimeError("sessão em transferência…")`:
1. o monitor de estado (`state_monitor`) era escolhido uma vez, na abertura do chat. O do Python, com
   a sessão desligada, chama `problema_de`, embrulhado por `native_slot`, que recusa fora das fases
   estáveis. O erro sobe pelo `Difusor` e o pump fecha o SSE. Era exatamente a sequência do notebook:
   `desligou name=… (cano segue vivo)` e, no mesmo segundo, `sse: fechou … erro no pump`. Além disso,
   mesmo sem erro, o monitor nunca trocava de fonte depois que o dono mudava;
2. a releitura da fila no chat (`PromptQueue.follow` → `load` → `queue_gate`);
3. a lista de sessões (`registry.list_with_state` → `snapshot`), que respondia 500 para todos.

**Conserto.**
- `owner_state_stream` (`runtime_adapter.py`): o monitor da sessão sem terminal segue a posse. Na
  passagem ele espera, conferindo a cada 0,25 s, e com o novo dono reabre a fonte certa (Rust ou
  Python). Erro de uma fonte cuja posse acabou de sair, ou `TransferInProgress` de uma passagem curta
  que a consulta não viu, não sobe; outro erro com o mesmo dono continua subindo. Uma tarefa só itera
  a fonte e só avança quando o chat pede o próximo, como a iteração direta. Sessão com terminal
  (Claude ou Codex) segue chamando o monitor direto: o do Codex é sensível a qualquer passo a mais
  entre notificações (juntava `working` e `idle`, 6 testes do `test_codex_adapter.py` pegaram).
- `queue_gate` recusa com `TransferInProgress` (subclasse de `RuntimeError`, mesma frase). Na
  passagem, `route_queue` deixa só o `load` ler a projeção em disco (`_queue_dir()/<nome>.jsonl`,
  mantida pelos stores Python e Rust). Escrita continua recusada.
- As leituras embrulhadas (`snapshot`, `problema_de`, `comandos`…) na passagem usam a vista do
  Python (sessão sem processo nele, então "ociosa") até o novo dono confirmar. `rename` e
  `close_sync` continuam recusando. Quem lê assim hoje é só a lista (`registry.list_with_state`) e o
  monitor: nenhum chamador escreve a partir dessa leitura, e toda escrita na passagem segue recusada.
  O Rust grava a projeção da fila a cada mudança (`runtime/queue.rs`), então ela não fica atrás.

**"desligou" logo depois de "religou" é esperado.** Na subida o Python religa o cliente a cada cano
vivo (`religou`) porque é o dono até o Rust adotar; a adoção solta esse cliente (`desligou … (cano
segue vivo)`). Na parada do systemd o Rust recebe o SIGTERM junto e morre primeiro; o Python recupera
as sessões (`religou`) e o lifespan as solta de novo (`desligou`). Nos dois casos o cano segue vivo.

> Deixou de valer em parte com o dono único (`dono-unico/plano.md`, Task 4): a administração
> (renomear, parar, recarregar, trocar modo ou conta, transferir) fecha no Rust e reabre nele sem
> religar cliente Python, e o lifespan não solta mais nada na parada. Sobra o par na readoção do
> boot até a Task 5, que tira também esse.

## Testes

Falham sem o conserto (conferido tirando só `backend/app`) e passam com ele:
`test_parada_solta_as_esperas_longas_na_hora`, `test_sinal_de_parada_avisa_as_esperas`
(`test_plugin_bridge.py`), `test_state_monitor_waits_for_new_owner_instead_of_failing`,
`test_reads_during_hand_over_use_python_view` (`test_runtime_adapter.py`),
`test_queue_read_during_hand_over_uses_projection` (`test_runtime_queue.py`),
`test_texto_que_chega_junto_com_a_parada_ainda_e_entregue` (`test_plugin_bridge.py`, da revisão). O
`test_state_monitor_error_without_owner_change_still_surfaces` protege o outro lado: erro real
continua subindo.

## Casos reais

Backend desta branch isolado como unit transiente do systemd de usuário (`systemd-run --user`,
`TimeoutStopSec=10`, o mesmo teto do serviço real): `HOME` temporário, `hangar-server` e
`hangar-cano` debug desta branch na 29765, convite e Connect em 29766/29768, `CP_AUTH_TOKEN` próprio,
`tmux -L hangar-shut` por embrulho no `PATH`, `claude` embrulhado com `--model claude-haiku-4-5` e
`CLAUDE_CONFIG_DIR=~/.claude-02-200`. Chat aberto por um vigia que registra só o tipo de cada
evento SSE. Respostas todas do Haiku.

| Caso | Sem conserto | Com conserto |
|---|---|---|
| Parada com 1–4 sessões sem terminal, 1 com terminal e chats abertos | 10,05 / 9,37 / 10,18 / 10,02 / 9,46 / 10,07 s; SIGKILL em 4 de 6 | 0,18 / 0,20 / 0,16 s; nenhum SIGKILL; `shutdown` do uvicorn em ~0,12 s |
| Canos sem terminal depois da parada | vivos | vivos (4 de 4, depois 5 de 5) e readotados na subida: `religou` e depois `desligou` de cada um, mensagem entregue e respondida depois do restart |
| Chat aberto na sessão nova durante a adoção pelo Rust | `desligou name=hl3` e `sse: fechou … erro no pump: RuntimeError: sessão em transferência` 0,7 s depois de abrir | `desligou name=hl4`/`hl5`, o chat abriu uma vez e não caiu, a resposta chegou pelo mesmo SSE |
| Lista de sessões lida a cada 0,1 s durante a adoção | 500 (`native_slot` no `snapshot`) | 150 de 150 com 200 |
| Chat aberto atravessando o restart | — | reconecta uma vez (o servidor reiniciou) e não cai depois |

Depois da revisão independente (achados tratados: entrega junto com a parada, passagem curta entre
duas consultas, limpeza da fonte num segundo cancelamento) e com o código já juntado à parte 1
(contrato interno 13, sem mudança de contrato aqui), uma rodada final em ambiente novo: adoção da
sessão nova com o chat aberto sem queda, lista 150 de 150 com 200, paradas de 0,20 s e 0,18 s sem
SIGKILL, canos vivos e readotados, mensagens entregues depois do restart.
Limites conhecidos, aceitos: na passagem (segundos) a lista pode mostrar a sessão ociosa e sem
cartão de aprovação, porque lê a vista do Python; uma entrega sem confirmação (`modo` fora de
`fill`/`user`) agendada no mesmo instante em que a espera responde vazio na parada pode ficar sem
destino. Espera pelo dono acima de 10 s vai ao diário como `runtime.state_owner_stuck`.

Não foi medido: Codex sem terminal na adoção, Windows/macOS (só pelo CI), e o notebook, cujo teto de
parada não conferi.
