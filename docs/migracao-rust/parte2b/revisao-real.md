# Revisão da 2B e da 2C com sessões reais

04/10/2026, branch `revisao-2b-2c` (de `hangar-server-parte1` em `2d91c651`, mais `bb80cf31`).
Pedido: `pedidos/2026-10-04-revisao-2b-2c-kickoff.md`.

## Ambiente do teste real

- Backend isolado: `HOME=/tmp/hangar-rev2b2c/home`, portas 28765/28766/28768, `CP_AUTH_TOKEN`
  próprio, tmux `-L hangar-rev2b2c`, `matar_orfaos` trocado por no-op no lançador.
- Fila do teste em `$HOME/.claude/.hangar-queue`: `CP_PROJECTS_DIR` aponta para um link
  `$HOME/.claude/projects -> ~/.claude-02-200/projects`. Apontar direto para a conta não serve:
  `~/.claude-02-200/.hangar-queue` é link para a fila real do dono (o teste da 2D gravou lá
  `runtime/terminal_*.json`; avisado à coordenação).
- Sessões Claude só Haiku (`claude-haiku-4-5`, imposto por um `claude` de teste no PATH), conta
  `CLAUDE_CONFIG_DIR=~/.claude-02-200`. Os hooks reais dessa conta rodam nas sessões de teste e
  gravam estado nas pastas do dono (efeito já existente no teste da 2D; nenhum arquivo do Hangar
  foi escrito pelo backend de teste dentro da conta).
- Codex não foi testado: o HOME de teste não tem login e o pedido proíbe copiar credenciais.

## Defeitos achados e corrigidos

Cada linha tem um commit e um teste que falha sem a correção (conferido revertendo só o código).

| # | Parte | Defeito e efeito | Como o Python antigo resolvia | Correção |
|---|---|---|---|---|
| 1 | 2B | Mensagem enviada com o Claude ocupado (adiada) era **perdida** quando a fila a drenava: o ator repetia os `call_id` fixos de `prepare_prompt`, o diário devolvia o recibo antigo sem gravar, `/internal/runtime/policy` recusava com 500, e 30 s depois a raiz virava `unknown` (marcada entregue, nunca enviada). Provado: "Fila 1" perdida, 2 e 3 entregues | Preparar o texto era cálculo local; falha antes do envio fazia `set_delivered(False)` (`adapter.py` drain) | `392de186`: `prepare_prompt` fora do diário; `native_message` com fase por tentativa; falha ou prazo antes de qualquer escrita vira adiada e volta para a fila |
| 2 | 2B | No caminho Rust **ninguém confirmava** as entradas sem terminal: nenhuma linha virava `confirmed`, a poda não as alcançava e com 1000 a fila recusaria toda mensagem | `apos_entrega` agendava a confirmação depois de cada drain do adapter | `4c25fbea`: o ator confirma a cada transição para `idle` |
| 3 | 2B | Tempestade de gravação: 1 mensagem simples = **62 regravações do estado** (200–277 KB, fsync do arquivo e da pasta) + 63 da projeção. Cada `format_status`/`reload_stamp`/`patch_meta` passava por prepare/vista/begin/finish e toda mudança de estado gravava a vista inteira (104 KB de comandos) | Estado só em memória; fila em JSONL com `tmp+replace`, sem fsync | `d2b99afa`: consultas sem efeito fora do diário; vista gravada só quando a parte durável muda |
| 4 | 2B | A fila Rust regravava a projeção (2 fsync) em toda chamada, leituras incluídas | `089fec02` já tinha corrigido no Python; o Rust ficou de fora | `8638d407`: compara com o arquivo antes de gravar |
| 5 | 2B | Índice de recibos guardava o **transcript inteiro** em memória pela vida da sessão; o Rust ainda clonava o buffer e todas as ocorrências a cada leitura, uma vez por entrada pendente; a reserva Python criava índice novo e relia o arquivo inteiro a cada `idle` | `_CommittedIndex`: offset, âncora e conjunto de linhas, leitura incremental | `b1ee3889`: guarda identidade, offset e 256 bytes; lê só o acrescentado; âncora relida do arquivo; um índice por conversa na reserva |
| 6 | 2B | Falha de `format_status`/`reload_stamp` (Python lento ou reiniciando) punha o ator em erro e passava **a sessão inteira** ao Python; recusava até a adoção | Formatação local; falha só registrada | `aafe267e`: perde só aquela parte, publica o problema e solta o pedido pendente |
| 7 | 2B | Ator que terminava com erro ficava registrado: `detach` falhava para sempre (sessão presa no Rust) e o retrato inicial de eventos de **todas** as sessões respondia 503 | Não havia ator separado | `1785efba`: libera a entrada depois de conferir a trava livre; sessão sem ator fica fora do retrato |
| 8 | 2B | Uma linha malformada da CLI (ex.: `control_response` sem `request_id`) encerrava o ator | O leitor pulava a mensagem ruim e seguia | `c0a7380b`: ignora e registra uma vez por sessão, sem o conteúdo |
| 9 | 2B | Cano morto depois de o Rust soltar a sessão: `_restore` falhava fora do `try` e a sessão ficava em "recuperando", recusando tudo até reiniciar o backend | `_esquecer_cano`; o próximo envio subia de novo com `--resume` | `4b5ac872`: fica estacionada no Python com o motivo no diário |
| 10 | 2B | ~20 recusas diferentes da fila saíam como `queue_io`/`InvalidData`, sem motivo, limitadas para todas as sessões juntas; a adoção recusada também | — | `4c20a3d0`, `0ea51d0d`: a frase fixa da recusa vai ao log do Rust e ao diário; erros do sistema/serde seguem só com o tipo |
| 11 | 2B | "Mandar agora" (orientar a fila) escrevia no stdin sem turno ou com permissão/pergunta pendente; a fila sumia da tela | Recusava com "Não há turno…" / "Responda a permissão…" | `5c94747b`: mesmas recusas, entrada continua na fila |
| 12 | 2B | **Primeira mensagem de sessão nova nunca confirmava**: despachada antes de o transcript existir, cursor sem identidade de arquivo (visto no uso real: "um") | Casamento por texto, sem cursor | `02e6506f`: cursor tirado antes de o arquivo existir casa com o arquivo novo da mesma conversa |
| 13 | 2B | `/clear` e outros comandos locais ficavam entregues e nunca confirmados (não viram linha `user`; visto no uso real) | `result` com `local_command` confirmava as entradas de barra | `f5af7e00`: mesma regra no ator |
| 14 | 2B | `_policy_calls` da rota de política guardava cada chamada e resultado para sempre | — | junto do #1 (`392de186`): só a chamada em curso fica |
| 15 | 2C | Prévia do Claude **congelava após `/clear`** quando outra conexão no mesmo chat tinha fechado antes (leitor da conexão morta preso no transcript apagado; vale também sem Rust) | O leitor só alimentava o sidecar do Pi | `0ec4b6eb`: o reset reinstala o leitor da conexão viva |
| 16 | 2C | Liberação do observador descartada durante a pausa por falhas ou com as 4 vagas de E/S ocupadas: `tmux -C` anexado até 90 s | `capture-pane` não deixava cliente anexado | `8d8927b5`: liberação fura a pausa e tenta de novo uma vez |
| 17 | 2C | Nome de sessão fora de `[A-Za-z0-9._-]{1,64}` (espaço, acento): 400 a cada captura e diário a cada ≤30 s | — | `b0234061`: Python aplica a mesma regra e lê direto |
| 18 | 2C | Saída avulsa de hook atribuída ao observador (ele vira o cliente "atual" do tmux ao anexar) derrubava o parser do modo controle | — | `c6d04981`: linha sem `%` fora de bloco é ignorada |

Contrato interno: `f2357e0c` sobe `RUST_SERVER_PROTOCOL`/`INTERNAL_PROTOCOL` para **11**
(combinado com `migracao-rust-2`; 9 e 10 são da parte 3 e da 2D).

## Casos reais

Sessões Haiku no backend isolado, com os binários desta branch.

| Caso | Antes das correções | Depois |
|---|---|---|
| Mensagem simples | 1 entrega; 62 regravações do estado e 63 da projeção | 1 entrega; 10 regravações do estado e 3 da projeção; estado 128 KB (antes 277 KB) |
| 3 mensagens com o Claude ocupado | "Fila 1" perdida (`unknown`, nunca enviada), ator em erro (`policy_refused` 500); nada confirmado | Cada uma entregue 1 vez e confirmada; log do Rust sem erro |
| Pergunta (AskUserQuestion) | — | Resposta pelo `/answer` chegou ("Verde") |
| Permissão (Write em modo manual) | — | `/select 1` aprovou; arquivo criado; sessão voltou a `idle` |
| `/clear` | — | Sidecar e lista no transcript novo; mensagem seguinte entregue 1 vez e confirmada; `/clear` confirmado (após #13) |
| Reinício só do backend com mensagem adiada na fila | — | Entregue 1 vez depois de voltar, confirmada |
| `kill -9` no `hangar-server` logo após um envio | — | Supervisor subiu outro em ~4 s; mensagem do meio entregue 1 vez; a seguinte também |
| Falha forçada antes de efeito (campo que o Rust recusa no estado da `rv-a`) | — | 4 recusas de adoção contadas no diário, depois `runtime.parte_para_python` só da `rv-a` (`adocao_recusada`); as 3 mensagens entregues 1 vez cada e confirmadas pela reserva Python; `rv-b` seguiu no Rust |
| 200 mensagens seguidas | — | POST: mediana 9 ms, p95 48 ms (as 200 em 2,5 s). Entrega: 223 s para as 200 (~1,1 s cada, o turno do Haiku); **cada uma no transcript exatamente 1 vez, todas confirmadas, nenhuma desistida**. Estado da fila: 274 → 367 KB no pico (200 linhas pendentes) → 294 KB no fim; memória do `hangar-server` 25 → 62 MB no meio, 52 MB no fim |

Sessões com terminal (2C), sessão Haiku `rv-t` no tmux isolado, com o chat aberto por SSE:

| Caso | Resultado |
|---|---|
| Observador anexado | Um cliente de controle `control-mode,ignore-size,no-output`, sem `read-only` |
| Digitação pelo Python com o observador ligado | Mensagem entregue 1 vez; estado `idle → working → idle`; 3 prévias durante a resposta |
| `/clear` com uma segunda conexão que fechou antes | A conexão viva recebeu `reset` e depois 4 prévias (até 387 caracteres) com `working → idle`: a prévia não congelou (#15) |

## Encontrado e não corrigido

Pendente de decisão/correção: mapas do ator que só crescem (`roots`, `attempts` com o quadro inteiro); entrada rejeitada/cancelada antes da escrita fica marcada entregue; parada do ator cortando um drain entre reivindicar e preparar; janela de 256 recibos faz cada gravação custar ~250 KB; observador vira o cliente "atual" do tmux e manda foco ao Claude ao anexar (inerente ao `tmux -C`); Codex e Windows não testados.
