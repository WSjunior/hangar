# Dono único — plano de implementação

> Execução só depois da aprovação do dono e das respostas às perguntas de `desenho.md`.
> Cada Task: teste escrito primeiro e visto falhar sem o código dela; o código que ficou morto
> sai no mesmo passo; revisão independente por Task.

**Objetivo:** com o Rust de pé, o que migrou é só dele do nascimento ao fim; falha vira erro com
código e motivo; o Python atende o migrado só quando é dono da porta inteira.
**Desenho:** `desenho.md`. **Inventário:** `inventario.md`.
**Base:** `origin/hangar-server-parte1` em `665fac8e`; branch de execução a criar a partir dela.

## Restrições globais

- Contrato interno: 14 na Task 2; qualquer mudança posterior de `/runtime/op`, `/internal/*`,
  `side-events` ou variável do filho toma o próximo número livre, Python e Rust no mesmo commit.
- Nenhuma sessão real, serviço ou instalador. tmux de teste com `-L` próprio; backend de prova
  isolado (HOME temporário, portas próprias), nunca outro backend no usuário sem o lançador com
  `matar_orfaos` desligado.
- Identificador novo em inglês; texto de tela por `m.<chave>()` em `pt.json` e `en.json`.
- Log e diário só com código, motivo curto e identidade; nunca texto de conversa.
- `git add` por caminho; commits em inglês; `HANGAR_SEM_PASSO=1` só em commit de documento.

## Lotes — o que corre em paralelo

- **Serial (mesmo coordenador Python):** Task 1 → Task 2 → Task 3 → Task 4 → Task 5 → Task 6.
- **Paralelo depois da Task 2:** Task 7 (rotas públicas do Rust) e Task 8 (Git/arquivos), cada
  uma na sua worktree; não tocam `runtime_coordinator.py`.
- **Paralelo depois da Task 3:** Task 9 (observação do terminal) e Task 10 (Codex sem terminal).
- **Por último:** Task 11 (documentação) e Task 12 (prova de uso real).

## Foco da revisão

- Nenhum caminho com o modo `rust` ou `pending` abre cliente Python em cano de sessão migrada,
  abre `QueueStore` com trava no Python ou roda drain legado.
- Falha do Rust nunca troca o dono da sessão nem reexecuta a operação no Python.
- Efeito possível nunca é repetido; entrada em despacho na morte do Rust fica incerta.
- Na tomada da porta, cada sessão é retomada uma vez, sem duplicar entrega.
- Divisão por plataforma (Windows) e por provedor continua intacta.

### Task 1: Falha de operação vira erro visível, sem passar a sessão

**Arquivos:** `backend/app/runtime_coordinator.py`, `backend/app/runtime_adapter.py`,
`frontend/src/lib/problema.ts`, `mobile/src/chat/SessionProblem.tsx`, `messages/pt.json`,
`messages/en.json` (o nativo já mostra `problema_detalhe` cru, `desktop-native/src/app.rs:5442`),
`backend/tests/test_runtime_ownership.py`, `backend/tests/test_runtime_terminal_failure_policy.py`.

**Falha sem ela:** `test_rust_failure_raises_with_code_and_session_stays_rust` (operação recusada
quatro vezes: cada uma sobe `RustOpError` com o código, a fase continua `Rust`, nenhuma chamada a
`legacy.op`), `test_rust_in_error_is_reported_not_handed_over` (`view.error` preenchido: a
operação mutável sobe o erro, sem `detach`), `test_problem_event_reaches_session_problem` (evento
`problem` vira `problema="runtime_falhou"` e `problema_detalhe="<código>: <frase>"` no snapshot),
`test_terminal_unknown_delivery_keeps_session_in_rust`.

**Código morto que sai:** `_hand_to_python`, laço de tentativas de `op`, `_RUST_TRIES`,
`_RETRY_PAUSE_S`, `_PRE_EFFECT_CODES`, `_TERMINAL_PRE_EFFECT_ERRORS`, `_safe_to_repeat`,
`Slot.rust_refused` e o desvio dele em `prepare_session`, a passagem de `_settle_rust`, a regra
de entrega incerta em `op` (`:773-780`), diário `runtime.parte_para_python`. `adopt_failures` e o
`rust_refused` da adoção saem também (a adoção falha vira erro com o motivo).

- [ ] **Step 1: Escrever os quatro testes acima e ver falhar na base**
- [ ] **Step 2: `op` com uma tentativa: erro do Rust sobe com `failure_reason`, diário `runtime.rust_op_failed {codigo, detalhe, kind}`; `_ANSWER_CODES` continua sem contar como defeito**
- [ ] **Step 3: `_settle_rust` lê o snapshot e sobe o erro em vez de passar a sessão; `adopt` falho sobe o erro sem contar**
- [ ] **Step 4: `apply_event` guarda a frase do `problem` e publica `runtime_falhou` em `problema`/`problema_detalhe`**
- [ ] **Step 5: Texto do código `runtime_falhou` na web e no app; chaves em pt e en; conferir que o nativo mostra a frase**
- [ ] **Step 6: Apagar o código morto listado e os testes de passagem (`test_runtime_ownership.py:146`, `:415`); reescrever `:241`, `:286`, `:321`, `:336`, `:355`, `:373` e `test_runtime_terminal_failure_policy.py:103`, `:220` para erro visível**
- [ ] **Step 7: Rodar os testes dos arquivos tocados e revisar**

### Task 2: Contrato 14 — `open`, `close` e diário vindo do Rust

**Arquivos:** `crates/hangar-server/src/lib.rs`, `crates/hangar-server/src/runtime/gateway.rs`,
`runtime/cano.rs`, `runtime/actor.rs`, `backend/app/rust_server.py`, `backend/app/internal_api.py`,
`backend/app/runtime_coordinator.py` (nomes das chamadas), testes Rust de runtime e
`backend/tests/test_internal_api.py` (ou o arquivo de testes da rota interna existente).

**Falha sem ela:** Rust `open_runs_queue_recover` (entrada em `dispatching` vira incerta ao
abrir), `open_answers_before_initialize` (cano falso que só responde o `initialize` depois de
2 s: `open` volta antes; `submit` volta `deferred`; a entrada sai uma vez depois do
`initialize`), `close_releases_lease`; Python `test_rust_diag_route_records_event` (segredo
certo grava no diário; sem segredo, 404) e a conferência de protocolo 14 nos dois lados.

**Código morto que sai:** `cano::peek` e `tests/runtime_cano.rs:36`; a espera de 180 s por
`ready` dentro do `adopt`. O `carry` fica **opcional** até a Task 5, que tira o último remetente.

- [ ] **Step 8: Testes Rust e Python acima, vistos falhar**
- [ ] **Step 9: `adopt` → `open` (`Recover` + `EnsureProjection` ao abrir, resposta antes do `initialize`) e `detach` → `close`; `adopt_terminal` vira o `open` do terminal**
- [ ] **Step 10: `POST /internal/diag` no Python (portão do segredo interno, só loopback) e cliente no Rust limitado por `warn_limit`**
- [ ] **Step 11: `INTERNAL_PROTOCOL` e `RUST_SERVER_PROTOCOL` = 14; `RuntimeTransport` com os nomes novos**
- [ ] **Step 12: `cargo test -p hangar-server` focado e pytest dos arquivos tocados; revisar**

### Task 3: Modo do processo e restart sem o Python religar os canos

**Arquivos:** `backend/app/runtime_coordinator.py`, `backend/app/rust_server.py`,
`backend/app/api.py` (lifespan), `backend/app/adapters/claude_headless/adapter.py`
(`reconectar_todas`, vigia), `backend/app/runtime_adapter.py` (guarda do cliente legado),
`backend/tests/test_runtime_lifecycle.py`, `backend/tests/test_rust_server.py` (ou o de Supervisor
existente).

**Falha sem ela:** `test_lifespan_with_rust_expected_opens_no_cano_client` (lifespan com binário
esperado: nenhum `ensure_running`/`_conectar`; registro só de metadados),
`test_rust_up_opens_live_canos_in_rust` (Supervisor confirma → `open` de cada sidecar com cano
vivo, nenhum cliente Python), `test_crash_before_limit_keeps_sessions_out_of_python` (1ª queda:
modo `pending`, nenhum `recover`; Rust novo → `open`), `test_third_crash_recovers_each_session_once`
(3ª queda: modo `python`, `recover` + `reconnect` uma vez por sessão, `reconectar_todas` uma vez),
`test_stop_does_not_recover_sessions` (parada: Rust morre e nada é religado),
`test_invalid_private_address_is_startup_failure` (o Python assume a porta inteira),
`test_legacy_client_refused_while_rust_owns` (guarda: abrir cliente Python em sessão migrada com
modo `rust`/`pending` levanta erro).

**Código morto que sai:** `adopt_registered` na forma de readoção; o `recover` por queda em
`deactivate_runtime` fora da tomada; no lifespan, `reconectar_todas`/`apos_entrega` incondicionais
(passam para a entrada no modo `python`); o "liga com as pontes desligadas" de `rust_server.py:338-341`.

- [ ] **Step 13: Testes acima, vistos falhar**
- [ ] **Step 14: Modo `pending`/`rust`/`python` no coordenador, com espera de desfecho para tarefas internas (`runtime_starting` no teto)**
- [ ] **Step 15: Supervisor: queda 1–2 → `pending` sem `recover`; desistência (inclusive endereço privado inválido) → `python` + retomada única; parada → nada**
- [ ] **Step 16: Lifespan registra só metadados com o Rust esperado; ao entrar em `rust`, `open` de cada sessão viva; ao entrar em `python`, o que o lifespan fazia**
- [ ] **Step 17: Guarda no cliente legado (`LegacyBridge.reconnect`, `ensure_running`, drain) por tipo migrado**
- [ ] **Step 18: Remover o código morto listado; rodar os testes tocados; revisar**

### Task 4: Sessão sem terminal nasce e reabre direto no Rust

**Arquivos:** `backend/app/runtime_adapter.py` (`wake`), `backend/app/runtime_coordinator.py`
(`prepare_session`, `ensure_open`), `backend/app/adapters/claude_headless/adapter.py` (lançar
processo sem conectar), `backend/app/api.py` (`_criar_sessao`), `backend/tests/test_runtime_routing.py`,
`backend/tests/test_runtime_adapter.py`.

**Falha sem ela:** `test_new_headless_session_never_opens_python_client` (criar + `acordar` com
modo `rust`: o processo do cano sobe, o sidecar recebe `cano` com `versao`, nenhum `_conectar`,
`open` chamado uma vez), `test_first_message_right_after_create_is_delivered_once` (cano falso
lento no `initialize`: mensagem enviada 0,1 s depois de criar sai uma vez),
`test_session_in_error_reopens_in_rust_once` (Rust em erro: a próxima operação faz `close` +
`ensure_open` uma vez; se a reabertura falha, o erro sobe e o problema fica),
`test_dead_cano_is_relaunched_on_reopen`.

**Código morto que sai:** em `wake`, o caminho "sobe pelo Python e adota"; em `prepare_session`,
o gatilho de `adopt` para sessão sem terminal; `drain_claims` e o registro de reivindicação do
drain no adapter do Claude sem terminal (só existiam para a passagem); `_peek`.

- [ ] **Step 19: Testes acima, vistos falhar**
- [ ] **Step 20: Lançar o processo do cano sem cliente e gravar o sidecar (versão do lançador)**
- [ ] **Step 21: `ensure_open`: slot direto no Rust, `open`, serializado por nome; `wake` e `_send_managed` passam por ele no modo `rust`**
- [ ] **Step 22: Reabertura única de sessão em erro, relançando o cano morto; diário `runtime.reopened`/`runtime.reopen_failed`**
- [ ] **Step 23: Remover o código morto listado e os testes que só cobriam a adoção do nascimento (`test_runtime_routing.py:92` reescrito para `open`); rodar e revisar**

### Task 5: Administração da sessão sem terminal por `close`/`open`

**Arquivos:** `backend/app/runtime_coordinator.py` (`change`, `lifecycle_call`, `shutdown`,
`_rebind`, `queue_gate`, `Phase`), `backend/app/runtime_adapter.py` (`quiesce`, `assert_legacy`,
`native_slot`, `owner_state_stream`, `install_adapter`, `registry_method`),
`backend/app/runtime_queue.py` (`route_queue`), `backend/app/internal_api.py`,
`crates/hangar-server/src/runtime/gateway.rs` (`carry`), testes de runtime.

**Antes de codar:** classificar as dez ações (`kill`, `rename`, `para_terminal`, `para_headless`,
`parar`, `recarregar`, `restart`, `open_terminal`, `open_headless`,
`set_permission_mode_sem_terminal`) em "só arquivo/processo/pane" e "precisa da CLI"; a segunda
vira `control` do Rust. Registrar a tabela no topo da Task antes do Step 25.

**Falha sem ela:** `test_rename_keeps_session_in_rust_without_python_client`,
`test_kill_closes_in_rust_and_stops_cano`, `test_mode_switch_to_terminal_closes_rust_first`,
`test_reload_reopens_in_rust`, `test_shutdown_leaves_canos_alive_and_touches_no_session`,
`test_state_stream_picks_source_once` (o monitor não espera troca de dono com o Rust de pé).

**Código morto que sai:** `Phase.PreparingRust`; `adopt` antigo; `LegacyBridge.quiesce` e o
`quiesce` do terminal; `_WRITE_WAIT_S`; `finish_wire(settling=...)`; ramos `finishing`/`continuing`
de `assert_legacy`; ramo `PreparingRust` de `native_slot`; desvio de leitura `_SYNC` durante a
passagem; espera por dono de `owner_state_stream` e `runtime.state_owner_stuck`;
`TransferInProgress` fora do `RecoveringPython`; leitura da projeção em `route_queue` na
passagem; `detach` + `quiesce` por sessão no `shutdown`; `carry` no Rust e no Python (contrato:
próximo número livre); diários `runtime.unclaim_*` e `runtime.write_uncertain` da passagem.
Testes apagados: `test_runtime_adapter.py` `:224`, `:371`, `:504`, `:586`, `:608`, `:650`, `:705`;
`test_runtime_queue.py:597`; os de adoção/`detach` de `test_runtime_ownership.py` reescritos para
"nunca dois donos" no `open`/`close`.

- [ ] **Step 24: Tabela das dez ações e testes acima, vistos falhar**
- [ ] **Step 25: `change` = barreira → `close` → ação sem cliente → `ensure_open`; ações que precisam da CLI viram `control` do Rust**
- [ ] **Step 26: `shutdown` sem nada por sessão; `_rebind` pelo novo `change`**
- [ ] **Step 27: Tirar `carry` dos dois lados com o próximo número de contrato**
- [ ] **Step 28: Remover todo o código morto listado e os testes de passagem; rodar os testes tocados; revisar**

### Task 6: Terminal nasce no Rust e a administração dele sem passagem

**Arquivos:** `backend/app/runtime_coordinator.py` (`prepare_session` do terminal),
`backend/app/runtime_terminal.py` (`run_admin`, `quiesce`), `terminal_input.py`, `btw.py`,
`permission_mode.py`, e, conforme a pergunta 1, `crates/hangar-server/src/runtime/terminal.rs`.

**Falha sem ela:** `test_terminal_session_registers_in_rust_without_python_phase` (pane recém-criado:
`_await_birth` → `open` terminal, nunca fase Python), e conforme a pergunta 1:
A) `test_admin_borrows_keyboard_without_moving_queue` (o Rust pausa as escritas, o Python digita,
a fila e a trava não mudam de dono; prazo vencido devolve o teclado e a operação falha com código);
B) um teste por comando portado; C) `test_admin_refused_while_rust_owns_terminal`.

**Código morto que sai:** caminho `detach` → Python → `adopt` de `run_admin`; `quiesce` do
terminal; `test_runtime_terminal.py:286`, `:301` reescritos.

- [ ] **Step 29: Testes da opção escolhida, vistos falhar**
- [ ] **Step 30: Registro do terminal direto no Rust**
- [ ] **Step 31: Administração do terminal pela opção escolhida (contrato: próximo número livre, se mudar)**
- [ ] **Step 32: Remover o código morto; rodar `test_runtime_terminal*.py` tocados e os testes Rust do terminal; revisar**

### Task 7: Rotas públicas do Rust (histórico e eventos) sem repasse por falha

**Arquivos:** `crates/hangar-server/src/routes.rs`, `crates/hangar-server/tests/runtime_diagnostics.rs`,
cliente de diário da Task 2.

**Falha sem ela:** `history_io_error_answers_503_with_code` (E/S quebrada: 503
`{error_code:"history_io", message}`, o Python não recebe o pedido), `events_without_info_answers_503`,
`missing_session_still_reaches_python_404`, `other_provider_history_still_goes_to_python`,
`route_failure_is_sent_to_diary`.

**Código morto que sai:** `Fallback`, `FALLBACK_AFTER`, `MAX_FALLBACK`, `AppState.fallback`,
`internal_refused`, o texto "Python atendeu" de `warn_if_internal_refused`, `routes.rs:567-602`.

- [ ] **Step 33: Testes acima, vistos falhar**
- [ ] **Step 34: Erros de `history`/`events` respondem 503 com código e vão ao diário; provedor fora do Rust e sessão inexistente seguem para o Python**
- [ ] **Step 35: Conferir no chat web que o 503 aparece como erro de carregamento com a frase (verificação manual)**
- [ ] **Step 36: Remover o código morto; `cargo test -p hangar-server` focado; revisar**

### Task 8: Git e arquivos sem repasse por falha

**Arquivos:** `crates/hangar-server/src/workspace_routes.rs`, `routes.rs` (chamadas de
`strip_client_fallback`), `crates/hangar-server/tests/workspace_routes.rs`,
`backend/app/workspace_bridge.py`, `backend/app/api.py` (middleware e `_resolver_citado`),
`backend/tests/test_workspace_bridge.py`, textos dos códigos no front (`packages/core/src/errosApi.ts`).

**Falha sem ela:** Rust `full_write_slots_answer_busy_without_python` (4 pushes lentos + commit:
o commit recebe 503 `workspace_busy` com `Retry-After` em menos de 1 s e o Git não roda),
`broken_context_answers_503_with_reason`, `unavailable_answers_503_with_reason`; Python
`test_bridge_failure_raises_instead_of_running_python` (ponte ligada e `workspace_unavailable`:
exceção 503, o corpo Python não roda), `test_bridge_off_runs_python` (ponte desligada = Python dono).

**Código morto que sai:** `FALLBACK_HEADER`, `strip_client_fallback`, `fallback_request`,
`to_python`, contadores de `workspace`; `workspace_bridge._fallback`, `_HANDOFF_CODES`,
`take_over`, `release`, `python_args`, diário `workspace.reserva_python`; as três linhas do
middleware. Testes: `test_workspace_bridge.py:115`, `:132`, `:145` apagados; `:87`, `:161`
reescritos; `workspace_routes.rs:386` apagado, `:255`, `:273` reescritos, `Handoffs` da fixture.

- [ ] **Step 37: Testes acima, vistos falhar**
- [ ] **Step 38: Rust responde 503 com código e motivo para ocupado, contexto e indisponível; envia ao diário**
- [ ] **Step 39: Ponte Python: `None` só com a ponte desligada; os outros casos levantam o erro de domínio; pedido acima de 4 MiB vira erro com código**
- [ ] **Step 40: Textos dos três códigos na web e no app; conferir no painel do repositório (verificação manual)**
- [ ] **Step 41: Remover o código morto; testes focados Rust e Python; revisar**

### Task 9: Observação do terminal sem troca de fonte por erro

**Arquivos:** `backend/app/state.py`, `backend/app/terminal_observer.py`, `backend/app/preview.py`,
`crates/hangar-server/src/terminal_control.rs`, `terminal_routes.rs` (texto do log),
`backend/tests/test_terminal_observer.py`, `crates/hangar-server/tests/terminal_diagnostics.rs`.

**Falha sem ela (opção A da pergunta 3):** `test_rust_capture_error_is_reported_not_replaced`
(captura com erro: nenhum `tmux.capture_pane` do Python, problema `terminal_observacao_falhou`
publicado, estado anterior mantido; a rodada seguinte pergunta ao Rust de novo),
`test_windows_and_bridge_off_still_capture_in_python`. Na opção B, só os testes do disjuntor saem.

**Código morto que sai:** disjuntor por sessão (`MAX_FAILURES`, pausa, `fallback_since`,
`_success`/`recovered`, diários `fallback`/`paused`); na opção A, o desvio de `state.py:735-738`
e a prévia Python por falha em `preview.py`. Testes `test_terminal_observer.py` `:1012`, `:1058`,
`:1297`, `:1353`, `:1634` apagados; `:80`, `:513`, `:527`, `:922` reescritos; os demais
conferidos pelo corpo.

- [ ] **Step 42: Testes da opção escolhida, vistos falhar**
- [ ] **Step 43: Erro tipado do observador; problema visível; texto novo do log Rust**
- [ ] **Step 44: Remover o código morto; testes focados; revisar**

### Task 10: Codex sem terminal

Conforme a pergunta 2.

**Opção A (fica no Python):** `test_codex_headless_stays_python_while_rust_owns` (modo `rust`:
cliente Python permitido para Codex sem terminal, nunca `open`; a guarda da Task 3 trata o Codex
sem terminal como não migrado). Documentar na Task 11.

**Opção B (nasce no Rust):** gravar `cano.versao` no sidecar do Codex; `ensure_open` para o
Codex; `test_codex_headless_born_in_rust` e o caso Codex na Task 12.

- [ ] **Step 45: Teste da opção escolhida, visto falhar**
- [ ] **Step 46: Implementar a opção; rodar `test_codex_*` tocados; revisar**

### Task 11: Documentação

**Arquivos:** `CLAUDE.md` (regra do `hangar-server`), `docs/decisoes/plataforma.md` (entrada
nova com a medição da Task 12), `docs/decisoes/superado.md` (passagem por sessão, 3 + 1,
`Fallback`, cabeçalho de repasse), `docs/migracao-rust/README.md`, `docs/migracao-rust/parada-e-posse.md`
e `parte2d/spec.md` (nota apontando para cá), linha "Erro usa a captura/reducer Python" do
`CLAUDE.md` corrigida conforme a Task 9.

- [ ] **Step 47: Atualizar a regra e as decisões; mover o que deixou de existir para `superado.md`**

### Task 12: Prova de uso real

Backend desta branch isolado como unit transiente do systemd de usuário (`systemd-run --user`,
`TimeoutStopSec=10`): `HOME` temporário, `hangar-server` e `hangar-cano` desta branch em porta
própria, convite e Connect em portas próprias, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio por
embrulho no `PATH`, `claude` embrulhado com `--model claude-haiku-4-5` e
`CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200` só no processo do agente, lançador com
`matar_orfaos` desligado. Entregas contadas no transcript cru por marcador único; quem atendeu,
pelo log do Python (pedido do Rust aparece lá só como `/internal/*`).

- [ ] **Step 48: Criar sessão e mandar na hora — sem terminal 10 de 10 e com terminal 10 de 10 com uma entrega; nenhum "religou"/"desligou" no log; nenhum `runtime.*` de passagem no diário (verificação manual)**
- [ ] **Step 49: Fila com o Claude ocupado — 3 mensagens durante um turno longo saem em ordem, uma vez cada (verificação manual)**
- [ ] **Step 50: `/clear` com o chat aberto — o chat não cai, a sessão continua no Rust, a mensagem seguinte sai uma vez (verificação manual)**
- [ ] **Step 51: Restart com fila — mensagem enfileirada antes do `systemctl restart` sai uma vez depois; nenhum cliente Python aberto; parada sem SIGKILL (verificação manual)**
- [ ] **Step 52: Queda do Rust — `kill -9` uma vez: a sessão continua no Rust novo, sem Python no meio; três vezes em 60 s: o Python assume tudo, cada sessão retomada uma vez, nenhuma entrega duplicada (verificação manual)**
- [ ] **Step 53: Falha forçada de operação — trava de escrita no estado da fila da sessão (`chmod`): o envio volta erro com código na tela, a sessão segue no Rust; desfeita a trava, a próxima mensagem sai uma vez. Git ocupado com 4 pushes lentos: 503 visível no painel, nada no Python (verificação manual)**
- [ ] **Step 54: Registrar a tabela de casos (sem e com conserto) em `docs/migracao-rust/dono-unico/prova-real.md` e na entrada de `plataforma.md`**
