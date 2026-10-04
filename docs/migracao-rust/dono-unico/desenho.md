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
| `pending` | boot com binário esperado, até o Supervisor decidir; entre uma queda do Rust (1ª ou 2ª) e a subida do Rust novo | ninguém: operações internas esperam o desfecho (até o teto de partida) e depois seguem ou falham com `runtime_starting` |
| `rust` | Supervisor confirmou partida (protocolo e saúde) | Rust |
| `python` | sem binário, `CP_RUST_SERVER=0`, `--reload`, ou Supervisor desistiu (não subiu, protocolo, 3 quedas, endereço privado inválido) | Python, até reiniciar o backend |

Em `pending` e `rust`, abrir cliente Python num cano, abrir `QueueStore` com trava no Python
ou rodar o drain legado é **erro de programação** (exceção na hora, com teste que prova que
nenhum caminho chega lá). A porta pública é do Rust: entre a queda e a volta dele ninguém de
fora alcança o Python, então `pending` só afeta tarefas internas (loop, temporizadores).

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
      Rust ao conectar;
   2. registra o slot já no Rust (sem `QueueStore` Python, sem trava Python) e manda
      `open {descriptor}` ao Rust;
   3. o Rust pega a trava, abre a fila, roda `Recover` (entrada em despacho vira incerta),
      conecta no cano (tentando até 10 s enquanto o cano começa a escutar), sobe o ator e
      **responde já** — sem esperar o `initialize`. O ator faz o `initialize` e drena a fila
      quando fica entregável (`actor.rs:318-323`, `:515-535`).
3. Mensagem logo após criar: `_send_managed` → `op(submit)` no Rust → `deferred` (ator ainda
   não entregável, `actor.rs:619`) → o ator entrega depois do `initialize`. Não há drain Python,
   reivindicação a devolver nem passagem: a corrida de `../parte2d/corrida-nascimento.md` deixa
   de existir por construção.
4. `ensure_open` serializa por nome (`registration_locks`); quem chega durante a abertura espera
   a resposta do `open` (segundos, não o `initialize`).

**Claude com terminal, modo `rust`:** o Python cria o pane; a primeira chamada ao driver
→ `prepare_session` → `_await_birth` (fica) → importação única da fila antiga, se houver (trava
tomada e solta antes) → slot no Rust → `open {descriptor terminal}`. Sem fase Python.

**Codex sem terminal:** fica no Python, como provedor não migrado (decisão 2).

**Sessão do Rust em erro** (cano caiu, E/S da fila, pânico do ator): o Rust publica o problema;
a próxima operação que precisa de sessão viva faz **uma** reabertura no Rust (`close` + `ensure_open`,
que relança o processo do cano se ele morreu). Deu certo: o problema some. Falhou: a operação
volta com o erro e o problema fica na tela. Nunca vai ao Python.

## Administração da sessão (renomear, matar, `/clear`, trocar modo, parar, recarregar)

`change()` passa a ser: barreira → `close` no Rust (solta a trava, o cano segue vivo) → ação do
Python **sem cliente** (arquivo, sidecar, sinal no processo, pane) → se a sessão continua,
`ensure_open` no Rust. Durante a barreira, operações esperam (como hoje). A ação que hoje
precisa conversar com a CLI (ex.: `set_permission_mode_sem_terminal`, se usar o cliente) vira
um `control` do Rust; a Task 5 classifica cada uma das dez antes de mexer.

`shutdown()` não faz nada por sessão: o Rust morre (stdin fechado ou SIGTERM), o sistema solta
as travas de arquivo, os canos seguem vivos no escopo deles.

**Administração do terminal** (`/model`, `/effort`, motor, `/btw`, modo, resposta por chat —
o Python digita no pane): teclado emprestado pelo Rust (decisão 1); `run_admin` deixa de fazer
`detach` → Python → `adopt`.

## Restart do backend com sessões sem terminal vivas

1. Parada: o Rust morre; o Supervisor vê a morte já em parada e **não** roda `recover`; o
   lifespan não faz `quiesce` nem `detach`. Canos seguem vivos. Nenhum "religou"/"desligou".
2. Subida: o lifespan registra só os metadados (modo `pending`); `reconectar_todas`,
   `apos_entrega`, aquecimento do Codex e vigia de ociosas **não rodam** em `pending`/`rust`.
3. Supervisor confirma o Rust → modo `rust` → `open` de cada sidecar com cano vivo. O Rust roda
   `Recover` da fila e hidrata pelo snapshot do cano (`actor.rs:286-318`).
4. Supervisor desiste → modo `python` → o que o lifespan faz hoje (`register` + `reconectar_todas`
   + `apos_entrega`) roda nesse momento, uma vez, dentro do `_take_over`.

Entrada sem confirmação no despacho quando o Rust morreu fica **incerta** (`Recover`), nunca é
reenviada às cegas — igual à regra atual da fila.

## O que o usuário vê em cada falha

| Falha | Na tela | Registro |
|---|---|---|
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

## Contrato interno 14

Sobe `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos na Task 1:

- `/runtime/op`: `adopt` → `open` (só `descriptor`; sem `carry`; responde depois de trava,
  fila, `Recover` e conexão, antes do `initialize`); `detach` → `close`. `cano::peek` sai.
- `POST /internal/diag` (Rust → Python, segredo interno): `{evento, sessao, codigo, motivo}`;
  o Python grava no diário. Limitado por `warn_limit` do lado Rust.
- Snapshot: `problema = "runtime_falhou"`, `problema_detalhe = "<código>: <frase>"`.

Mudança de contrato em Task posterior (ex.: o teclado emprestado da decisão 1) usa o próximo
número livre naquele momento, nunca reaproveita o 14 — o Python do checkout e o binário da
release podem vir de pushes diferentes, e só o número pega isso.

## Riscos

- **Restart muda de ordem.** Hoje o Python religa os canos no lifespan; no desenho novo ninguém
  religa até o Supervisor decidir (até ~10 s). Uma pergunta de permissão pendente aparece na
  lista só depois do `open`. Coberto pelo caso "restart com fila" da prova real.
- **Sessão do Rust não estaciona** (65 min ociosa): já é assim hoje para sessões adotadas; o
  desenho não piora nem resolve. Fica anotado.
- **Cano v1 vivo** (de antes da 2B): o Rust recusa (`cano_version`). Com o Rust de pé ele passa
  a dar erro visível, e a reabertura relança o processo como v2 (`--resume` da mesma conversa).
  Turno em andamento num cano v1 é perdido nessa troca; o `cano.py` atual já é v2.
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
