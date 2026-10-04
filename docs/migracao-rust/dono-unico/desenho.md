# Desenho: o Rust como único dono do que já migrou

Pedido: `../pedidos/2026-10-04-dono-unico-kickoff.md`. Inventário com arquivo:linha:
`inventario.md`. Plano: `plano.md`. Nada disto foi implementado.

## A regra, em uma frase

Quem atende uma sessão ou um pedido é decidido por **processo, plataforma, provedor ou tipo de
pedido** — nunca por uma falha. Com o Rust de pé, o que migrou é dele do nascimento ao fim;
falha vira erro com código e motivo, e a sessão continua no Rust.

Consequências diretas:

- Não existe mais `PreparingRust`, `quiesce`, `carry`, adoção de cano já conectado pelo Python,
  contador 3 + 1, `rust_refused`, `Fallback` das rotas públicas nem o cabeçalho
  `x-hangar-workspace-fallback`.
- O Python continua **lançando o processo** do cano (argv, env, conta, motor, escopo systemd,
  `setsid`) e gravando o sidecar, mas nunca conecta um cliente nele com o Rust de pé. Lançar o
  processo não é atender a sessão: o cano é dono da CLI, e o Rust é o único cliente do cano.
- Repetição automática da mesma operação contra o Rust (as 3 tentativas com pausa) sai junto:
  o dono pediu erro visível, e a repetição era metade do mecanismo de passagem. Quem reenvia é
  o usuário ou a fila (que já tem orçamento próprio).

## O que fica, e por quê

| O quê | Onde | Por quê |
|---|---|---|
| Reserva do processo inteiro: Supervisor, conferência do protocolo, 3 quedas em 60 s, `_take_over` com `lifespan="off"` | `rust_server.py:301-506`, `main.py:238-256` | decisão do dono; é o único caminho em que o Python atende o que migrou |
| Retomada das sessões quando o Python assume a porta: `deactivate_runtime` → `recover` → `_restore` → `LegacyBridge.reconnect`, com prova de morte e contenção | `rust_server.py:422-451`, `runtime_coordinator.py:918-993`, `runtime_process.py` | sem ela as sessões vivas ficam sem dono quando o Rust desiste |
| Cliente legado inteiro (`LegacyIO`, `LegacyBridge.op/confirm/reconnect`, `reserve_call`, `reserve_op` do terminal, `QueueStore` Python) | `runtime_adapter.py`, `runtime_terminal.py:398-731`, `runtime_queue.py` | é o que atende quando o Python é dono da porta; sai na parte 7 |
| Trava de arquivo por chave (`WriterLease` / `try_lock`) | `runtime_queue.py`, `runtime/queue.rs:217-225` | impede dois donos do arquivo da fila na troca de processo |
| Windows: observação do terminal pelo Python | `terminal_observer.py:118`, `terminal_control.rs:173` | divisão fixa por plataforma, não passagem |
| Pi, Kimi, omp, orq, Codex com e sem terminal | adapters Python | não migraram (Codex sem terminal: decisão 2) |
| Nome de sessão fora de `[A-Za-z0-9._-]{1,64}` observado pelo Python | `terminal_observer.py:117-119` | dono fixo pela sessão, decidido antes de qualquer pedido |
| Citações com linhas em memória (Codex transferido) no Python | `api.py:8436-8452` | o Rust não tem essa capacidade; dono fixo pelo tipo de pedido |
| Corpo inválido de Git/arquivos validado pelo Python (4xx) | `workspace_routes.rs:385-392` | o Python só recusa; nada roda nele |
| Serviços que o Rust pede ao Python (política, fatos do terminal, publicação do plugin, contexto de Git) | `runtime_policy.py`, `internal_api.py` | é o Rust chamando o Python, não passagem de posse |
| Espera de nascimento do terminal (`_await_birth`, 15 s) | `runtime_coordinator.py:254-271` | prova de identidade do pane antes de qualquer efeito |
| `assert_writer`: Python nunca escreve num pane do Rust | `runtime_terminal.py:220-253` | é a trava que garante um escritor |

## Estado do processo: um só, não um por sessão

O coordenador ganha um modo do processo, que substitui a fase por sessão como critério de dono:

| Modo | Quando | Dono das sessões migradas |
|---|---|---|
| `pending` | boot com binário esperado, até o Supervisor decidir; entre uma queda do Rust (1ª ou 2ª) e a subida do Rust novo | ninguém: operações internas esperam o desfecho por até `PENDING_WAIT_S = 30` s e depois seguem ou falham com `runtime_starting` |
| `rust` | Supervisor confirmou partida (protocolo e saúde) | Rust |
| `python` | sem binário, `CP_RUST_SERVER=0`, `--reload`, ou Supervisor desistiu (não subiu, protocolo, 3 quedas, endereço privado inválido) | Python, até reiniciar o backend |

Em `pending` e `rust`, abrir cliente Python num cano, abrir `QueueStore` com trava no Python
ou rodar o drain legado é **erro de programação** (exceção na hora, com teste que prova que
nenhum caminho chega lá). A porta pública é do Rust: entre a queda e a volta dele ninguém de
fora alcança o Python, então `pending` só afeta tarefas internas (loop, temporizadores).

**Tetos.** Uma partida do Rust leva no máximo 20 s: até 10 s pela linha `runtime_ready`
(`rust_server.py:314`) e até 10 s pela saúde (`:318`). `PENDING_WAIT_S = 30` cobre uma partida
com folga. Uma sequência de quedas seguidas chega a ~60 s; quem esperar além dos 30 s recebe
`runtime_starting` e repete, e o envio segue a regra da queda abaixo. As esperas internas mais
longas que já existem (`run_sync` 185 s, `queue_rpc` 35 s) passam a esperar o modo antes de
contar o próprio prazo.

Hoje, a cada queda 1 ou 2, o Python religa todas as sessões e o Rust novo as adota de volta.
No desenho novo não: as sessões ficam sem dono por segundos e o Rust novo as abre direto. O
`recover` em todas só roda quando o modo vira `python`.

## Nascimento direto no Rust

**Claude sem terminal, modo `rust`:**

1. `POST /api/sessions` → `registry._create_headless` grava o sidecar (igual a hoje).
2. `ensure_open(name)` (substitui `wake`/`acordar` no modo `rust`):
   1. o Python sobe o **processo** do cano (o mesmo `subir_cano_processo`, escopo systemd,
      `setsid`) e grava `cano{pid,escuta,token,ts,versao,config_marca}` no sidecar, sem
      conectar. A `versao` gravada é a do lançador (hoje 2 nos dois); a real é conferida pelo
      Rust ao conectar. O que `_subir_cano` faz hoje continua com o Python: argv/env com
      `engine_models` da troca de conta (`adapter.py:1034-1040`), teto de subidas com espera
      crescente (`:1013-1020`), e **nunca** relançar com o `pid` do sidecar vivo (dois Claude no
      mesmo `.jsonl`);
   2. registra o slot já no Rust (sem `QueueStore` Python, sem trava Python) e manda
      `open {descriptor}` ao Rust;
   3. o Rust pega a trava, abre a fila, roda `Recover` (entrada em despacho vira incerta),
      conecta no cano (tentando até 10 s enquanto o cano começa a escutar), sobe o ator e
      **responde já** — sem esperar o `initialize`. O ator faz o `initialize` e drena a fila
      quando fica entregável (`actor.rs:318-323`, `:515-535`). Trava ainda presa (no Windows o
      `LockFileEx` de um Rust que acabou de cair pode demorar a soltar) → o `open` espera até
      3 s, como o `close` já espera (`gateway.rs:37-44`), antes de responder `runtime_lease`;
   4. o `open` falhou ao conectar (`cano_connect`/`cano_auth`/`cano_timeout`): o Python mata o
      grupo do processo que acabou de lançar e limpa `cano` do sidecar (o que `_subir_cano` faz
      hoje quando o cano não escuta, `adapter.py:1093-1098`) e devolve o erro com o código.
      Quem pede a espera do `initialize` (troca de conta com `require_initialize`,
      `api.py:2956-2958`) espera o `initialized` da vista do Rust com teto e recebe
      `headless_nao_subiu` com a frase quando o CLI recusa.
3. Mensagem logo após criar: `_send_managed` → `op(submit)` no Rust → `deferred` (ator ainda
   não entregável, `actor.rs:619`) → o ator entrega depois do `initialize`. Não há drain Python,
   reivindicação a devolver nem passagem: a corrida de `../parte2d/corrida-nascimento.md` deixa
   de existir por construção.
4. `ensure_open` serializa por nome (`registration_locks`); quem chega durante a abertura espera
   a resposta do `open` (segundos, não o `initialize`).

**Claude com terminal, modo `rust`:** o Python cria o pane; a primeira chamada ao driver
→ `prepare_session` → `_await_birth` (fica) → importação única da fila antiga, se houver (trava
tomada e solta antes) → slot no Rust → `open {descriptor terminal}`. Sem fase Python.

**Codex sem terminal:** fica no Python, como provedor não migrado (decisão 2). O aquecimento do
Codex (`watch_sessions`, `api.py:457`) continua rodando em qualquer modo.

**O que o `carry` levava e de onde vem sem ele.** O `quiesce` mandava `initialized`, `model`,
`effort`, `permission_mode`, `previous_non_plan`, `commands`, `usage`, `context_window` e `cost`
(`runtime_adapter.py:286-289`), aplicados por cima de tudo (`gateway.rs:90`). Sem ele:
`model`, `effort` e `permission_mode` vêm do sidecar, que é o `meta` do descritor; `commands` e
`initialized` vêm da resposta do `initialize`; `usage` vem da política `last_usage`
(`claude.rs:289`); depois da primeira vida no Rust, tudo vem também da vista salva na fila, que
guarda esses campos (só os de `VOLATILE_VIEW`, `actor.rs:988`, não são salvos). O risco que sobra
é o **segundo `initialize`**: o snapshot do cano não marca `initialized` (só reaplica o
`system/init`, `claude.rs:272`), então o Rust reenvia o `initialize` a um CLI que já o recebeu
(`actor.rs:319-323`). Isso nunca foi medido no Claude (só no Codex: `-32600 "Already
initialized"`, `docs/decisoes/harnesses.md:2126-2127`); se o Claude recusar, `claude.rs:510-513`
grava `headless_nao_subiu` e a sessão nunca fica entregável. A Task 1 mede e trata antes de
qualquer outra mudança.

**Sessão do Rust em erro** (cano caiu, E/S da fila, pânico do ator, entrega incerta do
terminal): o Rust publica o problema; a próxima operação que precisa da sessão — inclusive a
leitura `ensure_projection` que o `/info` e o histórico fazem (`internal_api.py:160-167`) — faz
**uma** reabertura no Rust: `close` + `ensure_open` (relança o processo do cano só se o `pid`
morreu) para sem terminal, `close` + `open` do terminal (vínculo resolvido de novo, ator novo,
erro zerado) para com terminal. Deu certo: o problema some. Falhou: a operação volta com o erro e
o problema fica na tela. Nunca vai ao Python. Sem isso a sessão com terminal em
`terminal_delivery_unknown` ficaria travada até reiniciar o backend: o `problem` invalida o
cache (`runtime_adapter.py:655-663`), `_op_once` recusa (`runtime_coordinator.py:812-815`) e o
Rust só limpa o erro depois de uma entrega que deu certo (`terminal.rs:185`, `:286`).

**Estado do runtime momentaneamente indisponível** (o canal de eventos oscilou e invalidou o
cache de todas as sessões, `runtime_coordinator.py:460-463`): a operação pede o snapshot e
espera a reposição por até 5 s antes de responder o erro. Esperar a vista voltar não é passagem
de dono; é o que a pausa de `_RETRY_PAUSE_S` cobria por acaso.

**Envio quando o Rust cai no meio** (queda 1 ou 2): o Rust pode ter gravado a entrada na fila
antes de morrer (`actor.rs:588-592`). Erro de transporte (conexão caída, prazo, resposta
inválida — não um 503 com código) num envio não é "falhou": o Python espera o modo sair de
`pending` e repete o **mesmo** `operation_id` uma vez no Rust novo, que não duplica (a entrada é
procurada pelo `entry_id` antes do `Append`, `actor.rs:588-590`, e o `operation_id` repetido é
tratado em `actor.rs:557-568`). Se o modo virar `python` ou a repetição também falhar no
transporte, a resposta é "conservada, confirmação pendente" (`ok: true, delivered: false,
uncertain: true`), nunca `erro_envio_falhou`: a fila durável decide, e a bolha mostra o estado
real dela.

## Administração da sessão (renomear, matar, `/clear`, trocar modo, parar, recarregar)

`change()` passa a ser: barreira → `close` no Rust (solta a trava, o cano segue vivo) → ação do
Python **sem cliente** (arquivo, sidecar, sinal no processo, pane) → se a sessão continua,
`ensure_open` no Rust. Durante a barreira, operações esperam (como hoje). A ação que hoje
precisa conversar com a CLI (ex.: `set_permission_mode_sem_terminal`, se usar o cliente) vira
um `control` do Rust; a Task 4 classifica cada uma antes de mexer.

Além das dez ações do `change`, há caminhos que hoje chamam o cliente Python direto e entram na
mesma Task:

| Caminho | Hoje | Novo |
|---|---|---|
| Transferência Claude → Codex (`registry.py:2611-2700`, `conversation_transfer.py:766`) | `hl.parar`, `_sessions.pop/get`, `ensure_running(transfer_id=…)`, `ensure_running(so_reconectar=True)` | `close` no Rust → ação sem cliente; a verificação de ociosidade lê a vista do Rust |
| `_check_source_idle` (`conversation_transfer.py:766-767`) | **já quebrado** com o Rust dono: a fachada devolve `RuntimeView` (`runtime_adapter.py:593-601`), que não tem `.vivo` | lê `alive`/`in_progress`/`pending`/`question` da vista |
| Recuperação de transferência na subida (`api.py:449-456`) | roda no lifespan, antes do Rust | espera o modo sair de `pending` |
| Troca de conta/motor (`api.py:2936-2964`) | `parar` → `reset_start_attempts` → `ensure_running(require_initialize=True, engine_models=…)` | `close` → `parar` sem cliente → `ensure_open` com `engine_models` e espera do `initialize` |
| Troca para sem terminal (`api.py:2770`) | `ensure_running(esperar_pronta=False)` | `ensure_open` sem esperar o `initialize` |
| Envio pelo caminho antigo (`api.py:4381-4409`) | `prepare_session` devolve `False` → socket nativo direto, `PromptQueue` e `acordar` do Python | para sessão migrada no modo `rust`, `prepare_session` nunca devolve `False`: registra ou levanta o erro com código; o caminho antigo fica para o Codex sem terminal e o modo `python` |

A fachada `RuntimeAdapter.ensure_running` descarta os argumentos (`runtime_adapter.py:730-734`,
`:807-808`); ela deixa de ser chamada por esses caminhos, que passam a `ensure_open`.

`shutdown()` não faz nada por sessão: o Rust morre (stdin fechado ou SIGTERM), o sistema solta
as travas de arquivo, os canos seguem vivos no escopo deles.

**Administração do terminal** (`/model`, `/effort`, motor, `/btw`, modo, resposta por chat —
o Python digita no pane): teclado emprestado pelo Rust (decisão 1); `run_admin` deixa de fazer
`detach` → Python → `adopt`.

## Restart do backend com sessões sem terminal vivas

1. Parada: o Rust morre; o Supervisor decide **antes de qualquer ação** se é parada (espera o
   respiro e confere `stopping()`) e, sendo, não roda `deactivate_runtime` nem `recover` — hoje
   `deactivate_runtime` roda antes dessa conferência (`rust_server.py:372-377`). O lifespan não
   faz `quiesce` nem `detach`. Canos seguem vivos. Nenhum "religou"/"desligou".
2. Subida: o lifespan registra só os metadados (modo `pending`); `reconectar_todas`,
   `apos_entrega`, a recuperação de transferência e a vigia de ociosas do Claude sem terminal
   **não rodam** em `pending`/`rust`. O aquecimento do Codex roda (decisão 2).
3. Supervisor confirma o Rust → modo `rust` → `open` de cada sidecar com cano vivo. O Rust roda
   `Recover` da fila e hidrata pelo snapshot do cano (`actor.rs:286-318`). Sidecar com cano
   morto (reboot da máquina) e entrada não entregue na projeção da fila → `ensure_open`, que
   relança o processo: é o que o `apos_entrega` de cada sidecar fazia no lifespan
   (`api.py:331-332`). Depois a recuperação de transferência pendente roda.
4. Supervisor desiste → modo `python` → o que o lifespan faz hoje (`register` + `reconectar_todas`
   + `apos_entrega`) roda nesse momento, uma vez, dentro do `_take_over`.

Entrada sem confirmação no despacho quando o Rust morreu fica **incerta** (`Recover`), nunca é
reenviada às cegas — igual à regra atual da fila.

## O que o usuário vê em cada falha

| Falha | Na tela | Registro |
|---|---|---|
| Envio sem resposta porque o Rust caiu no meio | a bolha fica "aguardando confirmação"; nenhum aviso de falha | `runtime.send_uncertain {codigo}` |
| Envio recusado pelo Rust | aviso no chat por 8 s com a frase e o código (`erro_envio_falhou`, já existe em `api.py:4435-4439`); a bolha fica pendente/incerta conforme a fila | `runtime.rust_op_failed {codigo, detalhe, kind}` no diário; `runtime recusou operação` no `hangar-server.log` |
| Controle (modelo, esforço, permissão, interromper, resposta) recusado | o mesmo aviso no chat | idem |
| Sessão do Rust em erro | faixa acima do compositor e selo no cartão: "O runtime da sessão falhou — `<código>`: `<frase>`" (código novo `runtime_falhou` em `problema`, com a frase em `problema_detalhe`; a faixa e o selo já existem na web, no app e no nativo) | `runtime.reopened` / `runtime.reopen_failed` |
| Histórico não carrega | o erro de carregamento que o chat já mostra (`chat_erro_carregar_historico`, `Chat.svelte:1898`), agora com a frase do 503 | `POST /internal/diag` do Rust → diário `rust.history_failed` |
| SSE do chat não abre | o app reconecta como em qualquer queda; persistindo, o histórico mostra o erro acima | `rust.events_failed` |
| Git/arquivos ocupado | aviso no painel "Git ocupado, tente em instantes" (503 `workspace_busy` com `Retry-After`) | `rust.workspace_busy` |
| Git/arquivos sem contexto ou indisponível | aviso no painel com o motivo (503 `workspace_context`/`workspace_unavailable`) | `rust.workspace_failed` |
| Observação do terminal falha (decisão 3) | faixa "Observação do terminal indisponível — `<código>`"; o cartão fica no último estado | `terminal_observer.erro` |
| Rust cai (1ª ou 2ª queda) | o app perde a conexão por segundos e reconecta | `hangar_server.partida` |
| Rust desiste | nada muda na tela; tudo segue pelo Python | `hangar_server.reserva` (já existe) |

Textos novos entram por `m.<chave>()` em `messages/pt.json` e `messages/en.json` no mesmo
commit, e nos dois lugares que traduzem o código do problema (web `frontend/src/lib/problema.ts`,
app `mobile/src/chat/SessionProblem.tsx`); o nativo já mostra a frase de `problema_detalhe`
(`desktop-native/src/app.rs:5442`).

## Contrato interno

Sobe `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos, a cada mudança, nunca reaproveitando
um número — o Python do checkout e o binário da release podem vir de pushes diferentes, e só o
número pega isso. As Tasks juntam na branch de execução em série, na ordem do plano:

| Número | Task | Mudança |
|---|---|---|
| 14 | 1 | `/runtime/op`: `adopt` → `open` (responde depois de trava, fila, `Recover` e conexão, antes do `initialize`; espera a trava até 3 s; `carry` ainda aceito, opcional) e `detach` → `close`; `cano::peek` sai; `POST /internal/diag` (Rust → Python): `{evento, sessao, codigo, motivo}`, limitado por `warn_limit` |
| 15 | 4 | `carry` sai do `open` nos dois lados |
| 16 | 6 | teclado emprestado do terminal (pedir, prazo, devolver) |
| 17 | 8 | `x-hangar-workspace-fallback` sai: um Rust antigo com um Python novo faria o Python delegar de volta ao Rust (a reserva circular do PR #30) |

`/internal/diag` fica no `router` de `/internal`, atrás do `require_internal`
(`internal_api.py:27`, `:58`): o que protege é o segredo interno, não a origem — todo pedido que
o Rust repassa chega de 127.0.0.1 e o uvicorn interno confia nos cabeçalhos dele
(`rust_server.py:88-92`).

O problema `runtime_falhou` não muda contrato: o Python o monta a partir do evento `problem`, que
já leva `error_code` e `message`.

## Riscos

- **Restart muda de ordem.** Hoje o Python religa os canos no lifespan; no desenho novo ninguém
  religa até o Supervisor decidir (até 20 s). Uma pergunta de permissão pendente aparece na
  lista só depois do `open`. Coberto pelo caso "restart com fila" da prova real.
- **Sessão do Rust não estaciona** (65 min ociosa): já é assim hoje para sessões adotadas; o
  desenho não piora nem resolve. Fica anotado.
- **Cano v1 vivo** (de antes da 2B): o Rust recusa (`cano_version`) e a sessão mostra o erro.
  A reabertura só relança como v2 (`--resume` da mesma conversa) depois de parar o v1, e só com
  ele ocioso; turno em andamento num v1 espera terminar. O `cano.py` atual já é v2.
- **Windows.** A trava `LockFileEx` de um Rust que caiu pode demorar a soltar (espera de 3 s no
  `open`), e o plano mexe em Supervisor, subprocesso e trava: cada Task que toca neles lê antes
  "Regras vigentes" de `docs/decisoes/windows.md` e confere o job Windows do CI pelo log, job por
  job. A prova real é só no Linux; a VM Windows segue pendente como na parte 1.
- **Mais tráfego Rust → Python para o diário.** Limitado a 1 por minuto por (sessão, código).
- **Codex sem terminal no Rust nunca rodou no uso real** — por isso fica no Python (decisão 2).

## Decisões do dono (04/10/2026)

As três recomendações foram aceitas (recado da `migracao-rust-2`):

1. **Administração do terminal com o Rust de pé** (`/model`, `/effort`, motor, `/btw`, troca de
   modo, resposta por chat): **teclado emprestado.** O Rust pausa as próprias escritas e empresta
   o teclado do pane ao Python por uma operação, com prazo; fila, trava e estado continuam no
   Rust. Prazo vencido devolve o teclado e a operação falha com código. Contrato: o próximo
   número livre na Task 6.
2. **Codex sem terminal fica no Python, declarado como provedor não migrado** (igual Pi), até uma
   Task própria com prova real do Codex no Rust. O dono é fixo pelo provedor, decidido antes de
   qualquer pedido.
3. **Observação do terminal quando o Rust erra: erro visível, sem captura do Python.** Enquanto
   falhar, o estado do cartão fica parado e a fila do terminal espera; o Rust tenta de novo com
   pausa até 60 s. Windows e ponte desligada continuam lendo pelo Python.
