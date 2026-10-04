# Dono único — plano de implementação

> Execução só depois da aprovação do dono. Decisões das três perguntas registradas em
> `desenho.md` (04/10/2026): teclado emprestado, Codex sem terminal no Python, observação com
> erro visível. Revisão adversarial (`ecc:architect`, sobre `800e7c47`) incorporada; ver
> "Achados da revisão" no fim.

**Objetivo:** com o Rust de pé, o que migrou é só dele do nascimento ao fim; falha vira erro com
código e motivo; o Python atende o migrado só quando é dono da porta inteira.
**Desenho:** `desenho.md`. **Inventário:** `inventario.md`.
**Base:** a execução só começa depois que a `main` for trazida para a `hangar-server-parte1`; a
branch de execução nasce dessa ponta (que já inclui o PR #43, `cb244ccc`). As linhas citadas foram
conferidas em `665fac8e`; o PR #43 desloca as de `runtime_coordinator.py` a partir de `:1043`
(+7) e acrescenta `terminal_life`/`reborn_binding` em `runtime_terminal.py:938-953`. Cada Task
reconfere as linhas dela na base antes de codar.

## Restrições globais

- **Testes automatizados em cada Task** (decisão do coordenador `migracao-rust-2`, como nas
  demais execuções da migração; o `CLAUDE.md` raiz pede autorização para rodá-los): escritos
  primeiro, vistos falhar sem o código da Task, rodados só os dos arquivos tocados.
- **Nada vai para `main` nem para o canal de testes (`CP_UPDATE_BRANCH`) antes da Task 11.**
  Mesmo assim, cada Task deixa a branch coerente: os testes dos arquivos tocados passam e nenhum
  caminho fica sem dono entre uma Task e a seguinte (a ordem abaixo foi escolhida para isso).
- **Cada Task corrige, no mesmo commit, a regra de `CLAUDE.md` ou de `docs/` que ela torna
  falsa** (listada na própria Task). A Task 10 só cuida do que sobra.
- Contrato interno: 14 (Task 1), 15 (Task 4), 16 (Task 6), 17 (Task 8), Python e Rust no mesmo
  commit. As Tasks juntam na branch em série, na ordem do plano; se a ordem mudar, cada uma toma
  o próximo número livre na hora da junção, nunca reaproveita.
- Supervisor, subprocesso, trava de arquivo ou teste que simula `os.name`: ler antes "Regras
  vigentes" de `docs/decisoes/windows.md`; conferir o job Windows do CI pelo log, job por job.
- Nenhuma sessão real, serviço ou instalador. tmux de teste com `-L` próprio; backend de prova
  isolado, com o lançador desligando `matar_orfaos` (um segundo backend no mesmo usuário mata
  canos reais).
- Identificador novo em inglês; texto de tela por `m.<chave>()` em `messages/pt.json` e
  `messages/en.json`. Log e diário só com código, motivo curto e identidade.
- `git add` por caminho; commits em inglês.

## Lotes — o que corre em paralelo

- **Serial (mesmo coordenador Python):** Task 1 → 2 → 3 → 4 → 5 → 6.
- **Desenvolvimento em paralelo depois da Task 1:** Task 7 (rotas públicas do Rust), Task 8
  (Git/arquivos) e Task 9 (observação do terminal), cada uma na sua worktree; não tocam
  `runtime_coordinator.py`. Juntam na branch na ordem 7, 8, 9, depois da Task 6 (a 8 leva o
  contrato 17).
- **Por último:** Task 10 (documentação restante) e Task 11 (prova de uso real).

## Foco da revisão

- Nenhum caminho com o modo `rust` ou `pending` abre cliente Python em cano de sessão migrada,
  abre `QueueStore` com trava no Python ou roda drain legado (Codex sem terminal é não migrado).
- Falha do Rust nunca troca o dono da sessão nem reexecuta a operação no Python.
- Efeito possível nunca é repetido; repetir o mesmo `operation_id` só onde o Rust deduplica.
- Nunca dois processos de cano no mesmo `.jsonl`: relançar só com o `pid` do sidecar morto.
- Na tomada da porta, cada sessão é retomada uma vez, sem duplicar entrega.
- Divisão por plataforma (Windows) e por provedor continua intacta.

### Task 1: Contrato 14 — `open`, `close`, diário vindo do Rust e o segundo `initialize`

**Arquivos:** `crates/hangar-server/src/lib.rs`, `runtime/gateway.rs`, `runtime/cano.rs`,
`runtime/actor.rs`, `runtime/claude.rs`, `backend/app/rust_server.py`, `backend/app/internal_api.py`,
`backend/app/runtime_coordinator.py` (nomes das chamadas), testes Rust de runtime,
`backend/tests/test_internal_api.py` (ou o de rota interna existente), `docs/decisoes/harnesses.md`.

**Falha sem ela:** Rust `open_runs_queue_recover` (entrada em `dispatching` vira incerta ao
abrir), `open_answers_before_initialize` (cano falso que só responde o `initialize` depois de
2 s: `open` volta antes; `submit` volta `deferred`; a entrada sai uma vez depois),
`open_waits_for_lease_then_refuses` (trava presa por 1 s: abre; por 4 s: `runtime_lease`),
`close_releases_lease`, `reopen_of_initialized_cano_becomes_deliverable` (cano falso que responde
ao segundo `initialize` como o Claude real respondeu na medição do Step 2: a sessão fica
entregável e não grava `headless_nao_subiu`); Python `test_rust_diag_route_records_event` (com o
segredo grava no diário; sem ele, 404 — mesmo vindo de 127.0.0.1) e a conferência do protocolo
14 nos dois lados.

**Código morto que sai:** `cano::peek` e `tests/runtime_cano.rs:36`; a espera de 180 s por
`ready` dentro do `adopt`. O `carry` fica opcional até a Task 4.

- [x] **Step 1: Ler "Regras vigentes" de `docs/decisoes/windows.md` e de `docs/decisoes/harnesses.md`**
- [x] **Step 2: Medir com o Claude CLI real (Haiku, `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`, cano isolado, sem backend) a resposta a um segundo `initialize` no mesmo processo; registrar em `docs/decisoes/harnesses.md` (verificação manual)**
- [x] **Step 3: Testes acima, vistos falhar**
- [x] **Step 4: `adopt` → `open` (`Recover` + `EnsureProjection` ao abrir, espera da trava até 3 s, resposta antes do `initialize`) e `detach` → `close`; `adopt_terminal` vira o `open` do terminal**
- [x] **Step 5: Segundo `initialize` tratado pelo que o Step 2 mediu (recusa "já inicializado" = sucesso, como no Codex)**
- [x] **Step 6: `POST /internal/diag` no `router` de `/internal` (atrás do `require_internal`) e cliente no Rust limitado por `warn_limit`**

**Registro da execução (Task 1).** Step 2: o Claude 2.1.289 responde `success` ao segundo
`initialize` no mesmo processo, logo depois do primeiro e depois de um turno (`harnesses.md`,
"segundo `initialize`"); o Step 5 não precisou de código, e `reopen_of_initialized_cano_becomes_deliverable`
fica como regressão (passa também na base, assim como `close_releases_lease`, que é o mesmo
comportamento com o nome novo). Os outros três testes Rust falham com o `open` antigo (sem
`Recover`, com uma tentativa só na trava, esperando o `initialize`). O `Recover` roda em
`open_store`, antes do ator, para sem e com terminal (o ator do terminal ainda roda o dele; os
dois são idempotentes). Resposta do `open`/`close` passa a `opened`/`closed`; os nomes dos
métodos Python (`adopt`/`detach`) ficam até as Tasks 3 e 4. O cliente do diário é
`crates/hangar-server/src/diag.rs` (`DiagClient::report`), com teste próprio; a rota Python grava
`detalhe` (o `motivo` do fio), como os demais eventos `runtime.*`.
Revisão (`ecc:rust-reviewer`, `ecc:python-reviewer`, `ecc:silent-failure-hunter`): sem achado
crítico. Entraram: `wait_lease` só espera "trava ocupada" (outro erro de E/S responde na hora com o
tipo), falha ao fechar a fila não troca o erro da conexão, `open` que acha o ator morto fecha a
entrada antes de responder, o `motivo` do diário é `&'static str`, corpo inválido do `/internal/diag`
responde 400 (inclusive `RecursionError`) e o cano falso dos testes ignora conexão sem o token
(algum processo desta máquina sonda portas efêmeras e o derrubava). Ficaram para a Task 3, que
torna a falha visível: o `open` responde antes do `initialize`, então `headless_nao_subiu` aparece
depois, pelo evento `problem`, e não mais como recusa da adoção.
- [x] **Step 7: `INTERNAL_PROTOCOL` e `RUST_SERVER_PROTOCOL` = 14; `RuntimeTransport` com os nomes novos; remover o código morto; testes focados; revisar**

### Task 2: Sessão sem terminal nasce direto no Rust

**Arquivos:** `backend/app/runtime_coordinator.py` (`prepare_session`, `ensure_open`),
`backend/app/runtime_adapter.py` (`wake`), `backend/app/adapters/claude_headless/adapter.py`
(lançar processo sem conectar), `backend/app/api.py` (`_criar_sessao`, `_send_managed`,
`_send_one_headless`), `backend/tests/test_runtime_routing.py`, `backend/tests/test_runtime_adapter.py`.

**Falha sem ela:** `test_new_headless_session_never_opens_python_client` (criar + `acordar` com
transporte: o processo do cano sobe, o sidecar recebe `cano` com `versao`, nenhum `_conectar`,
`open` uma vez), `test_first_message_right_after_create_is_delivered_once` (cano falso lento no
`initialize`: mensagem enviada 0,1 s depois de criar sai uma vez),
`test_open_connect_failure_kills_launched_cano` (o `open` falha em `cano_connect`: o grupo do
processo lançado morre, `cano` sai do sidecar, o erro sobe com o código e conta no teto de
subidas), `test_never_relaunch_while_sidecar_pid_alive`,
`test_migrated_send_never_falls_to_legacy_path` (`prepare_session` de sessão migrada nunca devolve
`False`: registra ou levanta com código; `_send_one_headless` não chega ao socket nativo direto).

**Código morto que sai:** em `wake`, o caminho "sobe pelo Python e adota" para sessão nova; no
`prepare_session`, o gatilho de adoção para sessão sem terminal recém-criada.

- [x] **Step 8: Testes acima, vistos falhar**
- [x] **Step 9: Lançar o processo do cano sem cliente (argv/env/`engine_models`, escopo, `setsid`), gravar o sidecar, teto de subidas e nunca com `pid` vivo**
- [x] **Step 10: `ensure_open`: slot direto no Rust (sem `WriterLease`/`QueueStore` no Python), `open`, serializado por nome; falha de conexão mata o processo lançado; opção de esperar o `initialized` com teto**
- [x] **Step 11: `wake`, `_send_managed` e `prepare_session` de sessão migrada por `ensure_open`; o caminho antigo só para não migrado**
- [x] **Step 12: Remover o código morto; testes focados; revisar**

**Registro da execução (Task 2).** `ClaudeHeadlessAdapter.launch_process` sobe o processo do cano
(o mesmo `_lancar_cano` que o `_subir_cano` passou a usar: argv, ambiente, conta/motor, escopo) e
grava `cano` com `versao` do lançador, sem conectar; respeita o teto de subidas e devolve o cano
existente quando o `pid` do sidecar vive. `RuntimeCoordinator.ensure_open` (serializado pelo
`registration_locks`, como o `prepare_session`) registra o slot já em `Rust`, sem `WriterLease`
nem `QueueStore`, só depois de o `open` responder; falha `cano_connect`/`cano_auth`/`cano_timeout`
num cano recém-lançado mata o grupo (`discard_launch`) e limpa o sidecar; sucesso zera o teto.
`wait_initialized` espera a vista com teto de 185 s e devolve a frase de `headless_nao_subiu`.
O `wake` embrulhado passou a aceitar `engine_models` (antes, criar sessão sem terminal com conta de
motor levantava `TypeError` no embrulho) e, com o Rust de pé, chama `ensure_open` para o Claude.
O caminho antigo do `wake` (sobe pelo Python e adota) ficou só para o Codex e para o Python dono;
o gatilho de adoção do `prepare_session` ficou só para sessão já registrada no Python no boot, que
a Task 5 tira. `test_registered_v2_is_adopted_without_opening_python_reader` saiu: o cenário dele
virou `test_new_headless_session_never_opens_python_client`. O código do erro do Rust ainda não vai
ao diário como `codigo` (é o tipo da exceção): a Task 3 muda o `failure_reason`.
Revisão: só quem pode subir processo abre no Rust (`prepare_session(launch=True)`: envio,
`acordar`, `ensure_open`); leitura e parada pelo embrulho (`deliverable`, `parar`, `list_models`…)
nunca lançam, e sem cano vivo seguem o registro Python sem processo de antes. Registro Python sem
cano vivo é solto sob `freeze` e reaberto no Rust no primeiro envio. O `ensure_running` embrulhado
(Claude, Rust de pé, sem `so_reconectar`/`transfer_id`) vira `ensure_open` com `engine_models` e a
espera do `initialize` (`esperar_pronta`/`require_initialize`). Falha da abertura grava o problema
`headless_nao_subiu` com `<código>: <frase>` no sidecar e na faixa (como o caminho antigo); o teto
esgotado mantém o problema da queda que o esgotou; o `acordar` volta a zerar o teto. Ficam com a
Task 4 a troca de conta (ainda faz `parar` → `change`) e com a 3 a reabertura de slot do Rust com
cano morto. O teto zera quando o `open` responde, antes do `initialize`: a recusa do `initialize`
vira problema visível pelo Rust, e cada nova rodada é ação do usuário.

### Task 3: Falha vira erro visível, com reabertura única no Rust

**Arquivos:** `backend/app/runtime_coordinator.py`, `backend/app/runtime_adapter.py`,
`backend/app/internal_api.py`, `backend/app/api.py` (`_send_managed`), `frontend/src/lib/problema.ts`,
`mobile/src/chat/SessionProblem.tsx`, `messages/pt.json`, `messages/en.json` (o nativo já mostra
`problema_detalhe` cru, `desktop-native/src/app.rs:5442`), `backend/tests/test_runtime_ownership.py`,
`backend/tests/test_runtime_terminal_failure_policy.py`, `docs/migracao-rust/parte2d/spec.md`.

**Falha sem ela:** `test_rust_failure_raises_with_code_and_session_stays_rust` (operação recusada
quatro vezes: cada uma sobe `RustOpError` com o código, a fase continua `Rust`, nenhuma chamada a
`legacy.op`), `test_headless_in_error_reopens_in_rust_once` (próxima operação: `close` +
`ensure_open` uma vez; se a reabertura falha, o erro sobe e o problema fica),
`test_terminal_unknown_delivery_reopens_in_rust` (sessão com terminal em
`terminal_delivery_unknown`: a próxima operação reabre o terminal no Rust — vínculo relido, ator
novo — sem passar ao Python), `test_history_reopens_dead_session_once` (`ensure_projection` de
sessão cujo ator morreu reabre uma vez antes de responder 503), `test_invalid_cache_waits_for_resync`
(cache invalidado pela oscilação do canal de eventos: a operação espera o snapshot até 5 s e
segue), `test_transport_loss_on_send_is_uncertain_not_failed` (conexão caída no `submit`: resposta
`uncertain`, nunca `erro_envio_falhou`), `test_problem_event_reaches_session_problem` (`problem`
vira `problema="runtime_falhou"`, `problema_detalhe="<código>: <frase>"`).

**Código morto que sai:** `_hand_to_python`, laço de tentativas de `op`, `_RUST_TRIES`,
`_RETRY_PAUSE_S`, `_PRE_EFFECT_CODES`, `_TERMINAL_PRE_EFFECT_ERRORS`, `_safe_to_repeat`,
`Slot.rust_refused`, `adopt_failures`, a passagem de `_settle_rust`, a regra de entrega incerta em
`op` (`:773-780`), diário `runtime.parte_para_python`. Testes: `test_runtime_ownership.py:146`,
`:415` apagados; `:241`, `:286`, `:321`, `:336`, `:355`, `:373` e
`test_runtime_terminal_failure_policy.py:103`, `:220` reescritos.

**Regra corrigida:** `docs/migracao-rust/parte2d/spec.md`, seção "Falhas do Rust" (o "três +
uma" e "só a sessão passa ao Python") vira nota apontando para `dono-unico/desenho.md`.

- [x] **Step 13: Testes acima, vistos falhar**
- [x] **Step 14: `op` com uma tentativa; erro do Rust sobe com `failure_reason`; diário `runtime.rust_op_failed {codigo, detalhe, kind}`; `_ANSWER_CODES` não conta como defeito**
- [x] **Step 15: Reabertura única no Rust para sem terminal e com terminal, disparada por operação e por `ensure_projection`; diário `runtime.reopened`/`runtime.reopen_failed`**
- [x] **Step 16: Espera de reposição do cache (5 s) e envio com perda de transporte respondido como incerto (`runtime.send_uncertain`)**
- [x] **Step 17: `apply_event` publica `runtime_falhou`; texto na web e no app, chaves em pt e en; conferir que o nativo mostra a frase**
- [x] **Step 18: Remover o código morto e corrigir a regra da 2D; testes focados; revisar**

**Registro da execução (Task 3).** `op` faz uma tentativa: erro do Rust sobe com o código
(`failure_reason` passa a usar o `code` do `RustOpError` em `codigo`) e vai ao diário
`runtime.rust_op_failed` com `etapa` = tipo da operação; respostas de `_ANSWER_CODES` sobem sem
diário. Antes de qualquer operação (inclusive `ensure_projection`, que o `/info` e o histórico usam),
`_reopen_if_failed` consulta o snapshot quando a vista está inválida ou em erro e, se o ator está em
erro, o cano caiu (`alive: false`) ou o Rust não tem mais o ator (`runtime_binding`/`runtime_closed`/
`runtime_panic`), faz uma reabertura sob `freeze`: `close`, relança o processo do cano só se o `pid`
morreu (sem terminal) ou relê o vínculo (com terminal), `open`, mesmo slot e vista nova
(`runtime.reopened`/`runtime.reopen_failed`). Erros de manutenção do terminal (`terminal_facts`,
`receipt_scan`) não reabrem: o Rust os resolve no `confirm`/`drain`, que também não esperam a
reposição. Vista inválida espera o snapshot até 5 s (`_RESYNC_WAIT_S`) antes de recusar. Erro de
transporte (sem resposta do Rust) num envio volta `unknown` com `transport_lost`, e o `_send_managed`
responde `ok`, `delivered: false`, `uncertain: true` (`runtime.send_uncertain`); a resposta
`unknown` do próprio Rust continua erro com a entrada conservada. O `problem` guarda
`"<código>: <frase>"` na vista; `runtime_problem(name)` alimenta a fachada (`problema="runtime_falhou"`),
o cartão da lista e o estado do chat de sessão com terminal (decorado no SSE na próxima emissão
de estado); o snapshot com erro conserva a frase do mesmo código. Web (`problema.ts`) e app
(`SessionProblem.tsx`, que mostra a frase) ganharam `problema_runtime_falhou` em pt e en; o nativo já
mostra o `problema_detalhe` cru. Saíram `_hand_to_python`, `_settle_rust`, o laço de tentativas,
`_RUST_TRIES`, `_RETRY_PAUSE_S`, `_PRE_EFFECT_CODES`, `_safe_to_repeat`, `Slot.rust_refused`,
`adopt_failures`, a regra de entrega incerta do `op` e o diário `runtime.parte_para_python`;
`_TERMINAL_PRE_EFFECT_ERRORS` ficou, só para a manutenção. Testes: os da lista; apagados os de
passagem (`test_runtime_ownership.py` "quarta falha", "Rust em erro passa", "quatro tentativas",
"pré-efeito repetido", "quarta recusa", "falha do Python não conta"; `test_runtime_terminal_failure_policy.py`
"orçamento de tentativas" e "geração nova na repetição"); reescritos o de efeito possível, resposta
normal, entrega incerta (vira `test_terminal_unknown_delivery_reopens_in_rust`) e manutenção do
terminal. `test_invalid_cache_waits_for_resync` passava na base por acaso (a pausa de 2 s das
tentativas); falha sem a espera nova. Sem `node_modules` na pasta, o front não teve
`npm run check`; a mudança é uma chave nova nos dois JSON e um `case`.
Revisão: recusa de conexão (`ConnectionRefusedError`, nada saiu) é erro, não incerto; perda de
transporte invalida a vista para a próxima operação reler; ator sumido grava a frase na faixa;
a reabertura reconfere o erro dentro do `freeze` (duas operações juntas não reabrem duas vezes);
operações de fundo (`snapshot`, `drain`, `confirm`, `queue`) nem reabrem nem esperam a reposição
(senão um erro persistente vira laço de reabertura a cada evento); reabertura sem terminal zera o
teto (`open_succeeded`) e mata o cano recém-lançado que o Rust não alcançou; todo evento `problem`
pede um snapshot, que tira a faixa quando a falha foi cosmética (política). O chat web mostra a
frase inteira do `runtime_falhou` (até 300), como o app.

## Ponto de parada (sessão `dono-unico-exec`, 04/10/2026)

Tasks 1, 2 e 3 feitas e no CI (hashes no recado à `migracao-rust-2` e no `git log`); a próxima
sessão começa na Task 4, Step 19, sobre esta branch. Contrato interno ainda 14.

Armadilhas achadas nesta execução:

- **Teste Python que registra coordenador termina com `close_python_leases()`.** O `register`
  grava o `_current` global; sem soltar, os testes seguintes de `test_tmux.py` (e outros) falham
  por ordem com `MuxIndisponivel`/"Claude terminal sem vínculo". Não use `monkeypatch` no
  `_current` depois do `register`: ele restaura o coordenador velho no fim.
- **Rust falso nos testes devolve revisão crescente.** `apply_event` ignora snapshot com revisão
  menor que a guardada; um falso com revisão fixa congela a vista e o teste espera 5 s.
- **Nesta máquina algum processo sonda portas efêmeras.** Cano falso dos testes Rust aceita em laço
  e ignora conexão sem o token (`tests/runtime_open.rs`).
- **CI:** o resumo do `server.yml` fica `success` com job vermelho; conferir job por job. Vistos
  intermitentes: macOS `tests/terminal_routes.rs:165` (porta pública aceitou conexão logo depois de
  parar; a repetição passou) e o pytest `test_three_crashes_in_a_minute_hand_the_public_port_to_python`
  sob carga (passa isolado).
- **Push:** o hook pede revisores uma vez por commit e bloqueia o comando inteiro; faça o commit e
  o push em comandos separados (o segundo push passa).
- **Worktree sem `node_modules`:** checagem de front não roda aqui; `uv run` cria `.venv` própria
  (o aviso de `VIRTUAL_ENV` é inofensivo).
- **O que ficou para as próximas Tasks:** sessão registrada no Python no boot com cano vivo ainda
  passa pelo `adopt` (Task 5 tira); troca de conta ainda faz `parar` → `change` antes do
  `ensure_open` (Task 4); a faixa do runtime na sessão com terminal aparece no chat na próxima
  emissão de estado do monitor do pane (decoração no SSE), não na hora; `prepare_session(launch=…)`
  decide quem pode subir processo, e leitura de histórico (`ensure_projection`) reabre e relança
  cano morto por desenho do plano.

### Task 4: Administração da sessão sem terminal por `close`/`open`

**Arquivos:** `backend/app/runtime_coordinator.py` (`change`, `lifecycle_call`, `shutdown`,
`_rebind`, `queue_gate`, `Phase`), `backend/app/runtime_adapter.py` (`quiesce`, `assert_legacy`,
`native_slot`, `owner_state_stream`, `install_adapter`, `registry_method`, fachada
`ensure_running`), `backend/app/runtime_queue.py` (`route_queue`), `backend/app/internal_api.py`,
`backend/app/registry.py` (transferência), `backend/app/conversation_transfer.py`,
`backend/app/api.py` (troca de conta/motor, troca para sem terminal),
`crates/hangar-server/src/runtime/gateway.rs` (`carry`), testes de runtime e de transferência.

**Antes de codar:** tabela no topo da Task com cada caminho e a classe dele ("só
arquivo/processo/pane" ou "precisa da CLI" → `control` do Rust): as dez ações do `change`
(`kill`, `rename`, `para_terminal`, `para_headless`, `parar`, `recarregar`, `restart`,
`open_terminal`, `open_headless`, `set_permission_mode_sem_terminal`), a transferência Claude →
Codex (`registry.py:2611-2700`, `conversation_transfer.py:766`), a troca de conta/motor
(`api.py:2936-2964`, sem e com terminal), a troca para sem terminal (`api.py:2770`) e o
renascimento do terminal dentro de uma troca (PR #43: `terminal_life` guarda a prova da vida antes
da ação, `reborn_binding` herda a chave da sessão, a fila só esvazia com `session_id` novo).

**Falha sem ela:** `test_rename_keeps_session_in_rust_without_python_client`,
`test_kill_closes_in_rust_and_stops_cano`, `test_mode_switch_to_terminal_closes_rust_first`,
`test_reload_reopens_in_rust`, `test_account_switch_reopens_with_engine_models_and_waits_initialize`
(`initialize` recusado: `headless_nao_subiu` com a frase),
`test_switch_to_headless_opens_in_rust`, `test_transfer_source_idle_reads_runtime_view` (hoje
quebra com `AttributeError` em `.vivo`), `test_transfer_stops_source_without_python_client`,
`test_shutdown_leaves_canos_alive_and_touches_no_session`, `test_state_stream_picks_source_once`.
Regressão do PR #43, mantidos e passando pelo caminho novo (`close` → ação → `open` do terminal):
`test_account_move_reborn_terminal_keeps_key_and_queue`,
`test_account_move_waits_for_agent_of_reborn_terminal`, `test_pending_terminal_reborn_again_keeps_key`
(`test_runtime_terminal.py`). Riscos anotados no PR #43, cada um com teste que falha hoje:
`test_reborn_terminal_with_other_conversation_does_not_inherit_key` (o pane novo roda outra
conversa — `session_id` diferente do que a troca prometeu: não herda a chave sem conferir; vira
vínculo novo com a fila da conversa certa), `test_session_proof_survives_tmux_server_gone`
(`_session_proof` com o servidor tmux morrendo entre o `display-message` e o
`psutil.Process(...).create_time()`: `NoSuchProcess` vira "sem prova", nunca 500 no `change`),
`test_respawn_pane_in_same_tmux_session_keeps_key` (a troca recria o agente com `respawn-pane`
na mesma sessão tmux: a prova da sessão não muda, `reborn_binding` hoje devolve `None` e o
`change` volta a recusar a chave; a vida tem que ser reconhecida pelo pane/agente novo).

**Código morto que sai:** `Phase.PreparingRust`; `adopt` antigo; `LegacyBridge.quiesce` e o
`quiesce` do terminal; `_WRITE_WAIT_S`; `finish_wire(settling=...)`; ramos `finishing`/`continuing`
de `assert_legacy`; ramo `PreparingRust` de `native_slot`; desvio de leitura `_SYNC` durante a
passagem; espera por dono de `owner_state_stream` e `runtime.state_owner_stuck`;
`TransferInProgress` fora do `RecoveringPython`; leitura da projeção em `route_queue` na
passagem; `detach` + `quiesce` por sessão no `shutdown`; `carry` nos dois lados (contrato 15);
`drain_claims` e a devolução de reivindicação do adapter; diários `runtime.unclaim_*` e
`runtime.write_uncertain` da passagem; `_peek`. Testes apagados: `test_runtime_adapter.py`
`:224`, `:371`, `:504`, `:586`, `:608`, `:650`, `:705`; `test_runtime_queue.py:597`; os de
adoção/`detach` de `test_runtime_ownership.py` (`:68`, `:81`, `:127`, `:172`, `:194`) reescritos
para "nunca dois donos" no `open`/`close`; `test_runtime_routing.py:92` reescrito para `open`.

**Regra corrigida:** a nota "desligou logo depois de religou é esperado" de
`docs/migracao-rust/parada-e-posse.md` ganha a observação de que deixou de valer.

- [ ] **Step 19: Tabela de caminhos e testes acima, vistos falhar**
- [ ] **Step 20: `change` = barreira → `close` → ação sem cliente → `ensure_open`; ações que precisam da CLI viram `control` do Rust**
- [ ] **Step 21: Transferência, troca de conta/motor (sem e com terminal) e troca para sem terminal pelo mesmo caminho; `_check_source_idle` pela vista do Rust; renascimento do terminal herda a chave só com a mesma conversa, `_session_proof` sem exceção solta e `respawn-pane` reconhecido**
- [ ] **Step 22: `shutdown` sem nada por sessão; `_rebind` pelo novo `change`**
- [ ] **Step 23: Tirar `carry` dos dois lados com o contrato 15**
- [ ] **Step 24: Remover todo o código morto listado e os testes de passagem; testes focados; revisar**

### Task 5: Modo do processo, restart e a guarda do cliente legado

**Arquivos:** `backend/app/runtime_coordinator.py`, `backend/app/rust_server.py`,
`backend/app/api.py` (lifespan, recuperação de transferência), `backend/app/adapters/claude_headless/adapter.py`
(`reconectar_todas`, `apos_entrega`, vigia), `backend/app/runtime_adapter.py` (guarda),
`backend/tests/test_runtime_lifecycle.py`, o arquivo de testes do Supervisor,
`CLAUDE.md` (marcador do `hangar-server`), `docs/decisoes/plataforma.md` (entradas
"hangar-server" e "Observação terminal Rust", parágrafo do endereço torto).

**Falha sem ela:** `test_lifespan_with_rust_expected_opens_no_cano_client`,
`test_rust_up_opens_live_canos_in_rust`, `test_dead_cano_with_pending_queue_is_relaunched_after_rust_up`
(reboot: cano morto e entrada não entregue → `ensure_open` relança e entrega uma vez),
`test_crash_before_limit_keeps_sessions_out_of_python` (1ª queda: `pending`, nenhum `recover`;
Rust novo → `open`), `test_send_during_crash_waits_and_repeats_same_operation`
(perda de transporte em `pending`: espera até `PENDING_WAIT_S = 30` s e repete o mesmo
`operation_id` uma vez; o Rust novo não duplica), `test_pending_wait_has_ceiling`
(`runtime_starting` depois de 30 s), `test_third_crash_recovers_each_session_once`,
`test_stop_is_decided_before_any_action` (parada: nem `deactivate_runtime` nem `recover`),
`test_invalid_private_address_is_startup_failure`, `test_transfer_recovery_waits_for_owner`,
`test_legacy_client_refused_while_rust_owns` (abrir cliente Python em sessão migrada com modo
`rust`/`pending` levanta erro), `test_codex_headless_stays_python_while_rust_owns` (decisão 2:
cliente Python permitido, nunca `open`, aquecimento roda em qualquer modo).

**Código morto que sai:** `adopt_registered` como readoção; `recover` por queda em
`deactivate_runtime` fora da tomada; no lifespan, `reconectar_todas`/`apos_entrega` e a
recuperação de transferência incondicionais (passam para a entrada no modo `python`, ou para
depois do `open` no modo `rust`); o "liga com as pontes desligadas" de `rust_server.py:338-341`;
o gatilho de adoção do Codex sem terminal em `prepare_session` (inalcançável, achado 4).

**Regra corrigida:** marcador do `hangar-server` no `CLAUDE.md` (modo do processo; queda 1–2 não
passa pelo Python); `plataforma.md`: "Endereço ausente/torto desliga a ponte com aviso" vira
"…é falha de partida: o Python assume a porta inteira", e a entrada do `hangar-server` ganha o
modo do processo.

- [ ] **Step 25: Ler "Regras vigentes" de `docs/decisoes/windows.md`; testes acima, vistos falhar**
- [ ] **Step 26: Modo `pending`/`rust`/`python` com `PENDING_WAIT_S = 30`; esperas internas longas esperam o modo antes de contar o próprio prazo**
- [ ] **Step 27: Supervisor: parada decidida antes de qualquer ação; queda 1–2 → `pending` sem `recover`; desistência (inclusive endereço privado inválido) → `python` + retomada única**
- [ ] **Step 28: Lifespan registra só metadados com o Rust esperado; ao entrar em `rust`, `open` dos canos vivos, `ensure_open` dos mortos com fila pendente, depois a recuperação de transferência; ao entrar em `python`, o que o lifespan fazia**
- [ ] **Step 29: Envio em `pending`: espera e repete o mesmo `operation_id` uma vez; senão incerto**
- [ ] **Step 30: Guarda no cliente legado por tipo migrado (Codex sem terminal fora)**
- [ ] **Step 31: Remover o código morto, corrigir as regras; testes focados; job Windows do CI conferido pelo log; revisar**

### Task 6: Terminal nasce no Rust e o teclado emprestado

**Arquivos:** `backend/app/runtime_coordinator.py` (`prepare_session` do terminal),
`backend/app/runtime_terminal.py` (`run_admin`, `quiesce`), `terminal_input.py`, `btw.py`,
`permission_mode.py`, `crates/hangar-server/src/runtime/terminal.rs`, `runtime/gateway.rs`.

**Falha sem ela:** `test_terminal_session_registers_in_rust_without_python_phase` (pane recém-criado:
`_await_birth` → `open` terminal, nunca fase Python), `test_admin_borrows_keyboard_without_moving_queue`
(o Rust pausa as escritas, o Python digita, fila e trava não mudam de dono),
`test_keyboard_loan_expires_and_fails_with_code` (prazo vencido: o Rust retoma o teclado, a
operação do Python falha com código e o `assert_writer` volta a recusar),
`test_rust_queue_waits_during_keyboard_loan` (entrada da fila não é digitada durante o
empréstimo e sai uma vez depois).

**Código morto que sai:** caminho `detach` → Python → `adopt` de `run_admin`; `quiesce` do
terminal; `test_runtime_terminal.py:286`, `:301` reescritos.

- [ ] **Step 32: Testes acima, vistos falhar**
- [ ] **Step 33: Registro do terminal direto no Rust**
- [ ] **Step 34: Operação de teclado emprestado no Rust (pedir, prazo, devolver) e `run_admin` digitando só dentro dela; contrato 16**
- [ ] **Step 35: Remover o código morto; `test_runtime_terminal*.py` tocados e testes Rust do terminal; revisar**

### Task 7: Rotas públicas do Rust (histórico e eventos) sem repasse por falha

**Arquivos:** `crates/hangar-server/src/routes.rs`, `crates/hangar-server/tests/runtime_diagnostics.rs`,
cliente de diário da Task 1.

**Falha sem ela:** `history_io_error_answers_503_with_code` (E/S quebrada: 503
`{error_code:"history_io", message}`, o Python não recebe o pedido), `events_without_info_answers_503`,
`missing_session_still_reaches_python_404`, `other_provider_history_still_goes_to_python`,
`route_failure_is_sent_to_diary`.

**Código morto que sai:** `Fallback`, `FALLBACK_AFTER`, `MAX_FALLBACK`, `AppState.fallback`,
`internal_refused`, o texto "Python atendeu" de `warn_if_internal_refused`, `routes.rs:567-602`.

- [x] **Step 36: Testes acima, vistos falhar**
- [x] **Step 37: Erros de `history`/`events` respondem 503 com código e vão ao diário; provedor fora do Rust e sessão inexistente seguem para o Python**
- [x] **Step 38: Conferir no chat web que o 503 aparece como erro de carregamento com a frase (verificação manual)**
- [x] **Step 39: Remover o código morto; `cargo test -p hangar-server` focado; revisar**

**Registro da execução (Task 7).** Os testes ficaram em `tests/conversations.rs` (a casa do
histórico e do chat ao vivo com o Python falso; `runtime_diagnostics.rs` é de um teste só por causa
do assinante de log global). `history_io_error_answers_503_with_code`, `events_without_info_answers_503`
e `route_failure_is_sent_to_diary` falharam antes do código (200 do repasse);
`missing_session_still_reaches_python_404` já passava e fica como regressão;
`other_provider_history_still_goes_to_python` já existia como
`history_without_owner_or_supported_provider_goes_to_python` (e, no `/events`,
`provider_outside_rust_resets_and_next_connection_goes_to_python`). O `fetch_info` passou a separar
404 (sessão inexistente ou segredo recusado, que o Python já registra como `internal.recusado`) de
falha (sem resposta, outro status, corpo inválido): só a falha vira 503 `internal_info`. Corpo do
503: `{ok:false,error_code,message}` mais `detail:{code,msg}`, que o `lerErro` do app já mostra sem
mudar o front. Diário: `rust.history_failed` e `rust.events_failed`. **Desvio aprovado pela
`migracao-rust-2`:** o `Fallback` (struct, `FALLBACK_AFTER`, `MAX_FALLBACK`, `AppState.fallback`) fica
só para o Git/arquivos, que ainda o usa em `workspace_routes.rs:313-360`; a Task 8 apaga o struct
junto com os contadores de workspace (sem mexer nas mesmas linhas aqui, a junção não conflita).
Step 38 em backend isolado (HOME temporário, porta 19765, `tmux` próprio, `matar_orfaos`, reconciliação
de conta e portas do convite/Connect neutralizadas, sessão sem terminal sem conta): fila virada pasta →
503 `internal_info` no histórico e nos eventos; transcript virado pasta → 503 `history_io`; o chat
mostrou "Não deu pra carregar o histórico." com "503: a leitura do histórico falhou — history_io", e
o diário recebeu as três linhas `rust.*`.
Revisão (`ecc:rust-reviewer`, `ecc:silent-failure-hunter`): sem achado crítico. Entraram: o
`/history` só guarda no cache do `info` a sessão encontrada (o 404 guardado fazia o `/events`
seguinte repassar sem perguntar) e os testes do 503 conferem CORS, `content-type` e a linha do
diário do `history_io`. Ficaram de fora: `reset`/`Close`/atraso do canal ao vivo não são falha do
Rust (troca de provedor, aparelho lento ou cliente que saiu); o 503 do `/events` invisível ao
`EventSource` é o combinado do desenho (o histórico mostra o erro); o diário que não chega quando o
Python está fora fica inteiro no `hangar-server.log` (`diag.rs`, da Task 1).

### Task 8: Git e arquivos sem repasse por falha

**Arquivos:** `crates/hangar-server/src/workspace_routes.rs`, `routes.rs` (chamadas de
`strip_client_fallback`), `crates/hangar-server/tests/workspace_routes.rs`,
`backend/app/workspace_bridge.py`, `backend/app/api.py` (middleware), `backend/app/rust_server.py`
e `lib.rs` (contrato 17), `backend/tests/test_workspace_bridge.py`, textos dos códigos no front
(`packages/core/src/errosApi.ts`).

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

**Regra corrigida:** `docs/migracao-rust/git-arquivos/spec.md:38-42` (vaga cheia ou
indisponível → repasse ao Python, registrado no diário).

- [x] **Step 40: Testes acima, vistos falhar**
- [x] **Step 41: Rust responde 503 com código e motivo para ocupado, contexto e indisponível; envia ao diário**
- [x] **Step 42: Ponte Python: `None` só com a ponte desligada; os outros casos levantam o erro de domínio; pedido acima de 4 MiB vira erro com código; contrato 17**
- [ ] **Step 43: Textos dos três códigos na web e no app; conferir no painel do repositório (verificação manual)**
- [x] **Step 44: Remover o código morto e corrigir a regra; testes focados Rust e Python; revisar**

**Registro da execução (Task 8).** O 503 sai em `refuse` (`workspace_routes.rs`) no formato combinado
com a Task 7 (`{ok:false,error_code,message,detail:{code,msg}}`), com `detail.params.motivo` para o
front montar a frase; `Retry-After: 2` só no ocupado. Diário: `rust.workspace_busy`,
`rust.workspace_failed` (contexto, indisponível e também 5xx comum, que inclui a escrita cujo git não
iniciou e por isso responde 500). `DiagClient` virou campo `diag` do `AppState`. O teste
`unavailable_answers_503_with_reason` mora em binário próprio (`tests/workspace_unavailable.rs`)
porque esvazia o `PATH` do processo. A ponte Python devolve `None` só com ela desligada **ou com a
conexão recusada** (Rust fora do ar, inventário §5 `:106-113` "fica"); os demais casos viram erro:
`workspace_busy` (sem vaga local ou no Rust), `workspace_unavailable` (leitura sem resposta),
`workspace_request_too_large` (413), `workspace_invalid_request` (500). O `detail` do
`GitError`/`FsError` continua texto (dicionário quebrava `bastao`, `worktrees` e `_erro_arq`); o
código vai em `exc.code`, e um `exception_handler(GitError)` no `api.py` responde o status com o
envelope quando o erro escapa da rota (citação, resolver), em vez de 500. `head_info`, `branch_of`,
`git_summary`, `git_diffstat` e `git_log_since` prometem não levantar (a listagem depende disso):
falha da ponte devolve o vazio delas (`quiet=`), com log e diário em `_failed`. O `Fallback` de
`routes.rs` fica nesta base porque `history`/`events` ainda o usam; ele e o teste `routes.rs:567-602`
saem na junção depois da Task 7 (combinado com `migracao-rust-2`). `scripts/medir-rust.sh` perdeu o
`git_ms`: com o Rust de pé a medição do lado Python passa pela ponte. O teste do diário
(`diag.rs`, Task 1) passou a ignorar conexão sem segredo: sonda de porta desta máquina o derrubava.
Revisão (`ecc:rust-reviewer`, `ecc:python-reviewer`, `ecc:silent-failure-hunter`): nenhum caminho em
que falha do Rust rode no Python; entraram os consertos acima, a chave do limite de log com o motivo
e as traduções dos dois códigos novos da ponte. Step 43: textos na web e no app pelo mapa comum
(`errosApi.ts`, teste `errosApi.test.ts`); a conferência no painel do repositório fica para a prova
de uso real (Task 11).

### Task 9: Observação do terminal sem troca de fonte por erro

**Arquivos:** `backend/app/state.py`, `backend/app/terminal_observer.py`, `backend/app/preview.py`,
`crates/hangar-server/src/terminal_control.rs`, `terminal_routes.rs` (texto do log),
`backend/tests/test_terminal_observer.py`, `crates/hangar-server/tests/terminal_diagnostics.rs`,
`CLAUDE.md`, `docs/decisoes/plataforma.md`.

**Falha sem ela (decisão 3):** `test_rust_capture_error_is_reported_not_replaced`
(captura com erro: nenhum `tmux.capture_pane` do Python, problema `terminal_observacao_falhou`
publicado, estado anterior mantido; a rodada seguinte pergunta ao Rust de novo),
`test_windows_and_bridge_off_still_capture_in_python`.

**Código morto que sai:** disjuntor por sessão (`MAX_FAILURES`, pausa, `fallback_since`,
`_success`/`recovered`, diários `fallback`/`paused`); o desvio de `state.py:735-738` e a prévia
Python por falha em `preview.py`. Testes `test_terminal_observer.py` `:1012`, `:1058`, `:1297`,
`:1353`, `:1634` apagados; `:80`, `:513`, `:527`, `:922` reescritos; os demais conferidos pelo
corpo.

**Regra corrigida:** no `CLAUDE.md`, "Erro usa a captura/reducer Python com a mesma memória e os
mesmos fatos; Windows fica nesse caminho" vira "Erro vira problema visível e a rodada seguinte
pergunta ao Rust; Windows e ponte desligada usam a captura Python"; em `plataforma.md`, o título
"Observação terminal Rust com reserva Python" e o parágrafo da pausa por falhas.

- [ ] **Step 45: Testes acima, vistos falhar**
- [ ] **Step 46: Erro tipado do observador; problema visível; texto novo do log Rust**
- [ ] **Step 47: Remover o código morto e corrigir as regras; testes focados; revisar**

### Task 10: Documentação restante

**Arquivos:** `docs/decisoes/superado.md` (passagem por sessão, 3 + 1, `Fallback`, cabeçalho de
repasse, `quiesce`/`carry`, readoção a cada boot), `docs/migracao-rust/README.md` (estado das
partes), `docs/decisoes/plataforma.md` (entrada nova com a medição da Task 11).

- [ ] **Step 48: Mover o que deixou de existir para `superado.md` e atualizar o README da migração**

### Task 11: Prova de uso real

Backend desta branch isolado como unit transiente do systemd de usuário (`systemd-run --user`,
`TimeoutStopSec=10`): `HOME` temporário, `hangar-server` e `hangar-cano` desta branch em porta
própria, convite e Connect em portas próprias, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio por
embrulho no `PATH`, `claude` embrulhado com `--model claude-haiku-4-5` e
`CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200` só no processo do agente, lançador com
`matar_orfaos` desligado. Entregas contadas no transcript cru por marcador único; quem atendeu,
pelo log do Python (pedido do Rust aparece lá só como `/internal/*`).

- [ ] **Step 49: Criar sessão e mandar na hora — sem terminal 10 de 10 e com terminal 10 de 10 com uma entrega; nenhum "religou"/"desligou" no log; nenhum `runtime.*` de passagem no diário (verificação manual)**
- [ ] **Step 50: Fila com o Claude ocupado — 3 mensagens durante um turno longo saem em ordem, uma vez cada (verificação manual)**
- [ ] **Step 51: `/clear` com o chat aberto — o chat não cai, a sessão continua no Rust, a mensagem seguinte sai uma vez (verificação manual)**
- [ ] **Step 52: Restart com fila — mensagem enfileirada antes do `systemctl restart` sai uma vez depois; o mesmo com o cano morto antes da subida (`kill` no cano com a unit parada); nenhum cliente Python aberto; parada sem SIGKILL (verificação manual)**
- [ ] **Step 53: Queda do Rust — `kill -9` uma vez: a sessão continua no Rust novo, sem Python no meio, e uma mensagem mandada durante a queda sai uma vez; três vezes em 60 s: o Python assume tudo, cada sessão retomada uma vez, nenhuma entrega duplicada (verificação manual)**
- [ ] **Step 54: Falha forçada de operação — trava de escrita no estado da fila da sessão (`chmod`): o envio volta erro com código na tela, a sessão segue no Rust; desfeita a trava, a próxima mensagem sai uma vez. Sessão com terminal com `terminal_delivery_unknown` forçado: faixa na tela e a próxima operação reabre no Rust. Git ocupado com 4 pushes lentos: 503 visível no painel, nada no Python (verificação manual)**
- [ ] **Step 55: Troca de conta numa sessão sem terminal e noutra com terminal (a chave e a fila ficam; a mensagem seguinte sai uma vez) e transferência Claude → Codex: concluem sem cliente Python (verificação manual)**
- [ ] **Step 56: Registrar a tabela de casos em `docs/migracao-rust/dono-unico/prova-real.md` e na entrada de `plataforma.md`**

## Achados da revisão (`ecc:architect`, sobre `800e7c47`)

Todos conferidos no código; nenhum descartado.

| Achado | Onde entrou |
|---|---|
| Passagens que faltavam: transferência, troca de conta/motor, troca para sem terminal, envio pelo caminho antigo, `_check_source_idle` já quebrado | inventário §9; desenho (Administração); Tasks 2 e 4 |
| Task 1 tirava a recuperação antes do substituto; terminal sem reabertura | Task 3 traz a reabertura (sem e com terminal) junto com a remoção de `_hand_to_python`, depois do `ensure_open` da Task 2 |
| Histórico dependente do ator vivo | `ensure_projection` também reabre (Task 3) |
| O que o `carry` levava; segundo `initialize` do Claude nunca medido | desenho; medição e tratamento na Task 1, antes de tudo |
| Ordem das Tasks quebrava a branch | ordem nova (contrato → nascimento → erro + reabertura → administração → modo e guarda); nada vai ao canal antes da prova |
| Task 1 sozinha piorava o uso (oscilação do canal de eventos) | espera de reposição de 5 s (Task 3); `open` sem a espera de 180 s (Task 1) |
| Lançar sem conectar perdia partes do `_subir_cano` | Task 2: mata o processo lançado quando o `open` não conecta, teto de subidas, nunca relança com `pid` vivo |
| Restart com fila e cano morto; parada decidida depois do `deactivate_runtime` | Task 5 |
| Envio duplicado na queda 1–2 | envio com perda de transporte é incerto (Task 3) e repete o mesmo `operation_id` depois de `pending` (Task 5) |
| Teto do `pending` sem número | `PENDING_WAIT_S = 30` s, com a conta no desenho |
| Windows/`LockFileEx` | espera da trava no `open` (Task 1); leitura de `windows.md` e job Windows pelo log nas Tasks 1 e 5 |
| `/internal/diag` "só loopback" | atrás do `require_internal` (segredo) |
| Contrato 14/15 inconsistente | tabela única de números (desenho e restrições globais) |
| Regra contraditada só corrigida no fim | cada Task corrige a sua |
| Testes automatizados x `CLAUDE.md` | decisão do coordenador, registrada nas restrições globais |

## PR #43 (`cb244ccc`, entrou depois da revisão)

Troca de conta em sessão com terminal: `terminal_life` guarda a prova da vida antes da ação,
`reborn_binding` herda a chave, a fila só esvazia por `session_id`. A Task 4 reescreve esse
caminho; os três testes do PR ficam como regressão, e os três riscos anotados nele (herdar a
chave sem conferir a conversa, `NoSuchProcess` em `_session_proof`, `respawn-pane` na mesma
sessão tmux) ganham teste e conserto na mesma Task. A Task 11 (Step 55) prova a troca de conta
também numa sessão com terminal.
