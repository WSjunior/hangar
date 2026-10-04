# Inventário: toda passagem entre Python e Rust com o Rust vivo

Pedido: `../pedidos/2026-10-04-dono-unico-kickoff.md`. Linhas conferidas na árvore `665fac8e`
(branch `plan/rust-single-owner`, mesmo código de `584e65c5`). Contrato interno hoje: **13**
(`backend/app/rust_server.py:35`, `crates/hangar-server/src/lib.rs:20`).

Veredito: **some** (só existe para passar sessão ou operação entre donos com o Rust vivo),
**fica** (reserva do processo inteiro, divisão fixa por plataforma/provedor, ou necessário com
dono único), **muda** (o mecanismo continua, com outro comportamento — dito na linha).

## O que a leitura mostrou e muda o tamanho do trabalho

1. **A adoção não é só da sessão nova: é o caminho normal de toda sessão.** No boot, o lifespan
   do Python roda antes de o Rust existir (`rust_server.py:461-467`), registra cada sessão em
   `Phase.Python` (`runtime_coordinator.py:273-313`) e religa o cliente Python em todo cano vivo
   (`reconectar_todas`, `api.py:326`, log "religou"). Só depois o Supervisor sobe o Rust e
   `configure_transport` → `adopt_registered` adota cada uma (`runtime_coordinator.py:342-357`),
   desligando o cliente Python ("desligou"). A cada queda 1 ou 2 do Rust o vai-e-volta se
   repete: `deactivate_runtime` → `recover` → Python → Rust novo → `adopt`.
2. **Operações administrativas também passam a sessão ao Python e de volta.** `change()`
   (`runtime_coordinator.py:1028-1098`) faz `freeze` → `detach` do Rust → ação no Python →
   `prepare_session` → `adopt`. Entram por aí `kill`, `rename`, `para_terminal`, `para_headless`
   (`runtime_adapter.py:557-574`), `parar`, `recarregar`, `restart`, `open_terminal`,
   `open_headless`, `set_permission_mode_sem_terminal` (`runtime_adapter.py:849-850`), a troca de
   modo e o rename da API (`api.py:2701-2705`, `3124-3134`) e, no terminal, `run_admin`
   (`runtime_terminal.py:354-395`: `/model`, `/effort`, troca de motor, `/btw`, troca de modo,
   resposta por chat). `shutdown()` faz `detach` + `quiesce` de toda sessão (`:650-662`).
3. **O Rust nunca devolve sessão por conta própria; quem decide é o Python.** O Rust responde
   503 `{ok:false,error_code,message}` (`runtime/gateway.rs:232`) ou publica `problem`; o
   Python conta e passa. Exceção: as rotas HTTP públicas que o Rust atende têm um contador
   próprio que repassa ao Python (`routes.rs:50-87`).
4. **O Codex sem terminal provavelmente nunca chega ao Rust hoje.** `cano["versao"]` só é
   escrito em memória (`adapters/codex/sem_terminal.py:142`); o sidecar foi gravado antes
   (`:132`) e nenhum `update` posterior grava a versão. A condição de adoção exige
   `versao == 2` lida do sidecar (`runtime_coordinator.py:249-250`). Não medido em execução.
5. **O ator Rust sem terminal não roda o `Recover` da fila ao abrir** — só no `Stop`
   (`runtime/actor.rs:638`); o terminal roda (`runtime/terminal.rs:149`). Hoje quem faz o
   `recover` antes da adoção é o Python (`runtime_coordinator.py:546`, `:862`). Sem a passagem,
   o Rust precisa fazê-lo ao abrir.
6. **O Rust não escreve no diário.** Só no `hangar-server.log`. Todo `diag.registrar` de falha
   do Rust é o Python, ao pegar o `RustOpError`. Nas rotas que o Rust atende sozinho (histórico,
   eventos, Git/arquivos) a falha hoje só chega ao log e o cliente recebe a resposta do Python.

## 1. Parte 2B — coordenador (`backend/app/runtime_coordinator.py`)

| Linha | Mecanismo | O que faz | Veredito |
|---|---|---|---|
| 40-44 | `Phase` | `Python`, `PreparingRust`, `Rust`, `RecoveringPython` | **muda**: `PreparingRust` some; `RecoveringPython` fica só dentro da tomada da porta |
| 124-127 | `Slot.rust_refused`, `adopt_failures` | sessão marcada "fica no Python até reiniciar"; contagem de adoções falhas | **some** |
| 135-140 | `failure_reason` | `{codigo, detalhe}` para o diário | **fica**: é o código + motivo do erro visível |
| 145-155 | `_PRE_EFFECT_CODES`, `_ANSWER_CODES`, `_TERMINAL_PRE_EFFECT_ERRORS` | separa recusa repetível, resposta normal e pré-efeito terminal | **muda**: só `_ANSWER_CODES` sobra (resposta normal não é defeito do Rust) |
| 156-158 | `_RUST_TRIES=4`, `_RETRY_PAUSE_S=2.0` | o "3 + 1" | **some** |
| 161-163 | `TransferInProgress` | "posse passando; nada grava agora" | **muda**: só na janela de `RecoveringPython` da tomada da porta |
| 165-174 | `RustCacheInvalid`, `_safe_to_repeat` | cache do Rust inválido tratado como repetível | **muda**: erro visível direto |
| 193-252 | `prepare_session` | registra, espera nascimento, resolve vínculo e chama `adopt` (`:243-251`) | **muda**: registra já no Rust, sem adoção |
| 254-271 | `_await_birth` | espera até 15 s o agente do pane provar a conversa | **fica**: é prova de identidade, não passagem |
| 273-313 | `start_sessions` | registra tudo em `Python` no lifespan | **muda**: com o Rust esperado, não abre fila nem cliente Python |
| 342-357 | `configure_transport` / `adopt_registered` | readota tudo a cada Rust novo | **muda**: vira "abrir no Rust" (sem cliente Python antes) |
| 405-417 | `request_drain` / `drain` | drain via `op` | **fica** |
| 419-467 | `_events` | lê o canal privado do Rust | **fica** (sai a menção a `PreparingRust`, `:430`, `:461`) |
| 469-484 | `_rebind` | nova conversa reportada pelo Rust chama `change` | **muda** junto com `change` |
| 486-556 | `register` | slot, trava, `QueueStore`, `recover` da fila | **fica** para a reserva; com o Rust, registra sem abrir a fila no Python |
| 577-583 | `legacy_allowed` | diz se o legado pode escrever | **fica** (só testes usam) |
| 593-611 | `queue_gate` | contador `active` e recusa por `frozen`/fase | **muda**: sai o ramo de passagem |
| 613-638 | `queue_rpc` | fila no store Python ou no Rust conforme a fase | **fica** |
| 650-662 | `shutdown` | `detach` + `quiesce` de toda sessão na parada | **muda**: sem nada por sessão (o Rust solta as travas ao sair) |
| 664-693 | `_wait_active`, `_barrier`, `_rpc` | espera e serialização | **fica** |
| 695-711 | `_hand_to_python` | `detach` + `rust_refused` + diário `runtime.parte_para_python` | **some** |
| 713-729 | `_settle_rust` | Rust com `view.error` → passa ao Python | **muda**: erro visível e uma reabertura no Rust |
| 731-803 | `op` | contagem, `_settle_rust`, 3+1, reexecução no Python (`:799-803`), entrega incerta do terminal passa ao Python (`:773-780`) | **muda**: uma tentativa, erro sobe com código |
| 805-834 | `_op_once` | Rust por RPC ou `legacy.op` | **fica**: o ramo legado só com o Python dono da porta |
| 836-916 | `adopt` | `_peek`, `PreparingRust`, `quiesce`, solta a trava, RPC `adopt` com `carry`, conta falhas | **some** como passagem; a abertura no Rust fica com outro desenho |
| 918-943 | `_restore` | retoma trava, fila, `recover`, `legacy.reconnect` | **fica** (usado pela tomada da porta) |
| 945-962 | `detach` | RPC `detach` + `_restore` | **muda**: vira "fechar no Rust", sem `_restore` |
| 964-993 | `recover` | Rust morto confirmado → Python retoma a sessão | **fica**: é a reserva do processo inteiro |
| 996-1019 | `freeze`, `close_python_leases` | barreira e travas | **fica** |
| 1028-1112 | `change`, `lifecycle_call` | administração por `detach` → Python → `adopt` | **muda**: fechar no Rust → ação sem cliente → abrir no Rust |
| 1114-1151 | `_peek` | lê o cano sem tomar antes da adoção | **some** |

## 2. Parte 2B — fachada e cliente legado (`backend/app/runtime_adapter.py`)

| Linha | Mecanismo | O que faz | Veredito |
|---|---|---|---|
| 21-35 | `assert_legacy` | barra o cliente Python fora de `Python`; ramos `restoring`, `finishing`, `continuing` | **muda**: `finishing`/`continuing` (passagem) somem |
| 38-209 | `WireTicket`, `LegacyIO` | escrita registrada no cano pelo Python | **fica** para a reserva; sai o parâmetro `settling` de `finish_wire` (`:110`, `:192`, `:204`) |
| 213 | `_WRITE_WAIT_S=10` | teto da escrita em voo durante a passagem | **some** |
| 216-260 | `LegacyBridge.binding` | descritor a partir do sidecar | **fica** |
| 262-365 | `LegacyBridge.quiesce` | desliga o cliente Python, cancela o drain, devolve reivindicação, monta `carry`; diário `runtime.unclaim_skipped`, `write_uncertain`, `unclaim_failed` | **some** |
| 367-417 | `LegacyBridge.reconnect` | religa o cano ao Python e reaplica o estado | **fica** (reserva) |
| 419-554 | `LegacyBridge.op`, `confirm`, `accept_ack`, `bind_client`, `runtime_data` | caminho legado | **fica** (reserva) |
| 557-574 | `registry_method` | `kill`/`rename`/`para_terminal`/`para_headless` por `change` | **muda** com `change` |
| 604-619 | `native_slot` | `None` em `Python`; ramo `PreparingRust`; `TransferInProgress` fora de `Rust` | **muda**: sai o ramo `PreparingRust` |
| 622-697 | `apply_event` | aplica evento do Rust; `problem` vira `error` (`:682-686`) | **muda**: o erro passa a levar a frase, e chega à tela |
| 700-851 | `RuntimeAdapter` | fachada nativa | **fica**; `view(mutating=True)` já é erro visível (`:711-712`) |
| 967-1058 | `_OWNER_POLL_S`, `_OWNER_STUCK_S`, `_hands_over`, `_state_owner`, `owner_state_stream` | o monitor de estado segue a posse durante a passagem; diário `runtime.state_owner_stuck` | **muda**: fonte escolhida uma vez; reabre só se o processo inteiro trocar de dono |
| 1060-1089 | `reserve_call` | execução legada com diário | **fica** (reserva) |
| 1092-1171 | `install_adapter`, `wake` | roteia por sessão; leituras `_SYNC` caem na vista Python durante a passagem (`:1112-1119`); `wake` sobe o cano pelo Python e depois adota (`:1147-1171`) | **muda**: sai o desvio de passagem; `wake` sobe o processo sem cliente e abre no Rust |

## 3. Parte 2B — outros arquivos Python

| Arquivo:linha | Mecanismo | Veredito |
|---|---|---|
| `runtime_queue.py:668-698` | `route_queue`: `load` lê a projeção em disco durante a passagem | **muda**: sai o desvio de passagem |
| `runtime_queue.py` (resto) | `QueueStore`, diário durável, compactação | **fica** |
| `runtime_process.py` | contenção e prova de morte do Rust | **fica** |
| `runtime_policy.py` | serviços que o Rust pede ao Python | **fica** (é o Rust chamando o Python, não passagem) |
| `runtime_receipt.py` | recibos | **fica** |
| `internal_api.py:85-91` | política aceita `Rust` ou `PreparingRust` | **muda**: só `Rust` |
| `internal_api.py:158-167`, `api.py:3426-3434` | `ensure_projection` antes de `/info` e `/history`; erro vira 503 + `runtime.history_failed` | **fica**: já é o padrão desejado |
| `api.py:4413-4439` | `_send_managed`: falha vira `erro_envio_falhou` com a frase + `runtime.send_failed` | **fica**: já chega à tela (aviso no chat) |
| `rust_server.py:411-420` | `configure_runtime` dispara a readoção | **muda** |
| `rust_server.py:422-451` | `deactivate_runtime`: `recover` em toda sessão a cada queda | **muda**: só quando o Python assume a porta |
| `rust_server.py:338-341` | endereço privado inválido na saúde: liga o Rust com as pontes desligadas, Python observa e roda Git por trás | **muda**: vira falha de partida, o Python assume a porta inteira |

## 4. Lado Rust (`crates/hangar-server/src/`)

| Linha | Mecanismo | O que faz | Veredito |
|---|---|---|---|
| `lib.rs:20` | `INTERNAL_PROTOCOL = 13` | portão do contrato | **muda** → 14 |
| `lib.rs:31-66`, `main.rs:24-34`, `config.rs:41-62` | partida, `runtime_ready`, saída quando o stdin fecha, códigos de saída | ciclo de vida do filho | **fica** |
| `runtime/gateway.rs:194-243` | `/runtime/op`, `/runtime/events`, portão, 503 com `error_code`/`message` | canal privado | **fica** |
| `runtime/gateway.rs:61-118` | `adopt` (sem terminal): trava, fila, `cano::connect`, ator, espera `initialized`/`ready` até 180 s; mescla `carry["runtime_state"]` (`:90`) | **muda**: vira `open`, sem `carry`, roda `Recover`, responde antes do `initialize` |
| `runtime/gateway.rs:119-140` | `adopt_terminal` | **muda**: vira `open` do terminal |
| `runtime/gateway.rs:141-161` | `detach` (comentários de "voltar ao Python" em `:150-152`) | **muda**: vira `close` |
| `runtime/cano.rs:123-128` | `peek` | **some** (só `tests/runtime_cano.rs:36` usa) |
| `runtime/actor.rs:133-138`, `:313` | `error` do ator sem terminal nunca é limpo | **muda**: a sessão em erro é reaberta pelo Rust |
| `runtime/actor.rs:616` | snapshot leva só o código do erro | **muda**: leva código e frase, publicados como `problema` |
| `runtime/actor.rs:796-817` | falha de política cosmética vira `problem`; comentário "a falha deles continua levando-a ao Python" (`:802-804`) | **fica** o comportamento; o comentário sai |
| `runtime/terminal.rs:279-286` | entrega incerta → `enter_error("terminal_delivery_unknown")` | **fica** (o Python deixa de passar a sessão por isso) |
| `routes.rs:50-87`, `:47`, `:104` | `Fallback` (`FALLBACK_AFTER=4`): 4 falhas por (sessão, rota) mandam a rota ao Python até reiniciar; log "parte passou para o Python" | **some** |
| `routes.rs:158-182` | `internal_refused`, `warn_if_internal_refused` | **some** como contador; o log vira o do erro |
| `routes.rs:296-368` | `history`: falha de `info`, E/S ou pânico repassa ao Python | **muda**: 503 com `error_code` (`internal_info`, `history_io`, `history_panic`); sessão inexistente e provedor fora do Rust continuam no Python |
| `routes.rs:379-412` | `events`: idem para a abertura do SSE | **muda**: idem |
| `side.rs:30-32`, `:335-386` | provedor fora do Rust fecha o hub; ao reconectar vai ao Python | **fica** (divisão por provedor); o texto do comentário muda |
| `actor.rs:13-55` | `PolicyClient` (Rust chama `/internal/runtime/policy`) | **fica** |

## 5. PR #30 — Git e arquivos

Dois sentidos: o Rust atende o dono e repassa ao Python quando falha; as funções Python
(`git_ops`, `filetree`, `filesearch`, `fs`, `transcript`, citações) chamam o Rust pela ponte
privada e rodam o próprio corpo quando a ponte devolve `None`.

| Linha | Mecanismo | O que faz | Veredito |
|---|---|---|---|
| `workspace_routes.rs:254-276`, `:304-312`, `routes.rs:255,293,376` | `x-hangar-workspace-fallback`, `strip_client_fallback`, `fallback_request`, `to_python` | carimba o repasse para o Python não delegar de volta | **some** |
| `workspace_routes.rs:313-315` | `on_python(session,"workspace")` | depois de 4 falhas, Git/arquivos da sessão no Python até reiniciar | **some** |
| `workspace_routes.rs:316-327` | contexto (`/internal/workspace/context`) falhou → repassa com `contexto` | **muda**: 503 `workspace_context` com o motivo; 404 de sessão inexistente continua |
| `workspace_routes.rs:69-83`, `:335-340` | vagas (4 escrita, 8 leitura, 4 metadados) cheias → repassa com `ocupado` | **muda**: 503 `workspace_busy` com `Retry-After`, nada roda |
| `workspace_routes.rs:341-349` | `workspace_unavailable` → conta e repassa | **muda**: 503 com o motivo |
| `workspace_routes.rs:350-363` | erro ≥ 500 conta para o 3+1 (inclusive erro de Git do usuário) | **muda**: só sai o contador |
| `workspace_routes.rs:90-99` | escrita que pode ter rodado nunca repete (500/503) | **fica** |
| `workspace_routes.rs:385-392`, `:440` | corpo inválido ou operação não mapeada → Python responde o 4xx | **fica**: o Python só valida, nada roda nele |
| `workspace_routes.rs:102-124`, `routes.rs:200-205` | rota privada `/__hangar_server/workspace` | **fica** (canal Python → Rust) |
| `workspace_bridge.py:16-46`, `api.py:622-624`, `:647` | `_fallback`, `_HANDOFF_CODES`, `take_over`, `release`; diário `workspace.reserva_python` | **some** |
| `workspace_bridge.py:73` | `request()` devolve `None` com a ponte desligada | **fica**: ponte desligada = Python dono da porta |
| `workspace_bridge.py:76-86`, `:102-104` | argumento não serializável, pedido > 4 MiB, sem vaga, `workspace_unavailable`/`busy` → `None` → corpo Python | **muda**: erro com código |
| `workspace_bridge.py:106-113` | conexão recusada → `None`; escrita com outro erro → `workspace_action_uncertain` | **fica** (recusada = Rust caindo, janela da reserva) |
| `workspace_bridge.py:131-150` | `delegate(..., python_args=...)` | **some** (só o teste usa) |
| `api.py:8436-8452` | citações com `rows` em memória (Codex transferido) sempre no Python | **fica**: capacidade que o Rust não tem, dono fixo por tipo de pedido; sai na parte 7 |

## 6. Parte 2C — observador do terminal

O reducer nunca foi reserva: o estado é sempre calculado no Python (`state.py:897`). A
passagem é só da **captura** do pane.

| Linha | Mecanismo | O que faz | Veredito |
|---|---|---|---|
| `state.py:735-738` | captura do Rust devolveu `None` → `tmux capture-pane` do Python | **muda**: com a ponte ligada, erro vira problema visível; `None` só para ponte desligada, Windows e provedor fora do Rust |
| `terminal_observer.py:24-26`, `:38-47`, `:83-109` | disjuntor por sessão (3 falhas, pausa 1→30 s), `fallback`, `paused`, `recovered` | **some** (o Rust já tem pausa própria, `terminal_control.rs:381-387`) |
| `terminal_observer.py:117-119` | `_available`: ponte ligada, não Windows, nome válido, sem pausa | **muda**: sai a pausa; Windows e ponte desligada ficam; nome fora de `[A-Za-z0-9._-]{1,64}` fica no Python (dono fixo pela sessão) |
| `terminal_observer.py:187-231`, `:414-438` | erro, quadro inválido ou exceção → `None` | **muda**: erro tipado |
| `terminal_observer.py:255-411` | lease, batimento, época, `/clear` | **fica** |
| `preview.py:675-683` | prévia Python quando falta análise do Rust | **muda**: fica só para a faixa de mods (capacidade); falha do Rust não troca de fonte |
| `terminal_control.rs:173` | Windows: controle indisponível | **fica** (plataforma) |
| `terminal_control.rs:44-47`, `terminal_routes.rs:11-20` | log "observação terminal usa reserva Python" | **muda** o texto |

## 7. Parte 2D — envio ao Claude com terminal

| Linha | Mecanismo | Veredito |
|---|---|---|
| `terminal_input.rs` inteiro | driver Rust: toda falha vira `deferred`/`unknown` com estágio e código, sem Python | **fica** |
| `runtime/terminal.rs:70-96` | Rust pede fatos e publicação do plugin ao Python | **fica** (serviço) |
| `runtime_coordinator.py:773-780` | entrega `unknown` → `_hand_to_python("entrega_incerta")` | **some** |
| `runtime_terminal.py:28-187` | `outside_scope`, `resolve_binding`, `validate_binding` | **fica** |
| `runtime_terminal.py:190-217` | `quiesce` / `reconnect` do terminal | `quiesce` **some**; `reconnect` **fica** para a reserva |
| `runtime_terminal.py:220-253` | `assert_writer`: Python não escreve no pane com o Rust dono | **fica**, essencial |
| `runtime_terminal.py:354-396` + `terminal_input.py:2605-2606`, `btw.py:380`, `permission_mode.py:325-326` | `run_admin`: `detach` → Python digita no pane → `adopt` | **muda** — pergunta 1 em `desenho.md` |
| `runtime_terminal.py:398-731` | entrega completa pelo Python (`reserve_op`) | **fica** só para a reserva |

## 8. Nascimento hoje

**Claude sem terminal:** `POST /api/sessions` → `registry._create_headless` grava o sidecar
(`registry.py:2285-2337`) → `acordar` (`api.py:2534`) → `wake` (`runtime_adapter.py:1147`) →
`ensure_running` → `prepare_session` registra em `Python` → `_subir_cano` sobe o processo e
**conecta o cliente Python** (`adapter.py:1024-1110`) → `initialize` pelo Python → drain do fim
do initialize → `prepare_session` de novo → `adopt` (`quiesce` cancela o drain no meio). A
primeira mensagem entra pelo Python (`deferred`) e corre contra a adoção: é a corrida de
`../parte2d/corrida-nascimento.md`.

**Codex sem terminal:** `_create_codex_headless` (`registry.py:2339-2383`) → aquecimento →
`sem_terminal.subir` → cliente Python. Adoção provavelmente nunca (achado 4).

**Claude com terminal:** o Python cria o pane (`registry.py:2256`); a primeira chamada a um
driver embrulhado → `prepare_session` → `_await_birth` → `register` em `Python` → `adopt`.

**O que o Rust já faz e o Python não precisa fazer antes:** `initialize` do Claude e
`thread/start|resume` do Codex quando o cano ainda não inicializou (`actor.rs:318-323`,
`claude.rs:260-267`, `codex.rs:592-599`) e o drain da fila quando a sessão fica entregável
(`actor.rs:515-535`).

**O que só o Python sabe fazer:** argv/env/conta/motor do CLI, lançador `hangar-cano` ou
`cano.py` em escopo systemd com `setsid` (`adapter.py:2467-2533`), gravar `cano` no sidecar,
parar o cano, estacionar sessão ociosa (só as que estão em `self._sessions` do Python —
sessões do Rust não estacionam hoje).

## 9. Testes atingidos

Apagar (só cobrem a passagem): `test_runtime_ownership.py` `:146`, `:415`;
`test_runtime_adapter.py` `:224`, `:371`, `:504`, `:586`, `:608`, `:650`, `:705`;
`test_runtime_queue.py:597`; `test_workspace_bridge.py` `:115`, `:132`, `:145`;
`crates/hangar-server/tests/workspace_routes.rs:386`; `routes.rs:567-602`;
`test_terminal_observer.py` `:1012`, `:1058`, `:1297`, `:1353`, `:1634`.

Reescrever (o cenário fica, a asserção vira erro visível e a sessão continua no Rust):
`test_runtime_ownership.py` `:241`, `:286`, `:336`, `:355`, `:373`, `:321`;
`test_runtime_terminal_failure_policy.py` `:103`, `:220`; `test_workspace_bridge.py` `:87`,
`:161`; `workspace_routes.rs` `:255`, `:273`; `test_terminal_observer.py` `:80`, `:513`, `:527`,
`:922`; `test_runtime_routing.py:92`; os de adoção/`detach` (`test_runtime_ownership.py` `:68`,
`:81`, `:127`, `:172`, `:194`) passam a provar "nunca dois donos" no `open`/`close`.

Ficam (reserva do processo inteiro, fila, contenção, plataforma): `test_runtime_lifecycle.py`
`:152`, `:186`; `test_runtime_failure_matrix.py`; `test_runtime_final_fixes.py` `:467`, `:494`;
`test_runtime_process.py`; `test_runtime_ownership.py` `:57`, `:96`, `:113`, `:219`, `:391`,
`:431`; os de diário do legado em `test_runtime_adapter.py`; `test_terminal_observer.py` `:64`,
`:259`, `:1859`; `tests/terminal_control.rs:489` (Windows).

Os testes de `test_terminal_observer.py` não citados acima e os `test_runtime_terminal*.py`
foram classificados pelo nome; cada Task confere o corpo antes de apagar.
