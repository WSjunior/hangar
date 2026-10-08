# Parte 5, metade Claude: análise

Levantamento sobre a `origin/main` em `dcae49227` (07/10/2026), contrato interno **36**
(`crates/hangar-server/src/lib.rs:30`, `backend/app/rust_server.py:35`). Pedido: recado da sessão
`hangar` de 07/10. A metade Codex está em `../parte5-codex/spec.md` (branch
`hangar-server-parte5-codex`, PC de casa, sem push); o que as duas dividem está em
`contrato-par.md`.

Abreviações: `api` = `backend/app/api.py`, `rc` = `runtime_coordinator.py`, `rt` =
`runtime_terminal.py`, `ra` = `runtime_adapter.py`, `PB` = `plugin_bridge.py`, `rs/` =
`crates/hangar-server/src/`.

## Onde a sessão Claude está hoje

Com o Rust de pé, a sessão Claude (com e sem terminal) já é do ator do Rust: fila, trava,
entrega, estado ao vivo, prévia e terminal real. O que sobra no Python é de três tipos:

1. **Porta de entrada das escritas.** As rotas chegam ao Python porque o Rust repassa tudo que
   `rust_route` (`rs/migration_status.rs:98`) não reivindica. O Python valida, e o
   `coordinator.op` (`rc:1242`) manda o comando ao Rust por `POST /runtime/op`
   (`rust_server.py:193` → `rs/runtime/gateway.rs:328`). Ou seja: cada escrita faz
   Rust → Python → Rust.
2. **Teclado emprestado.** Operações de administração do terminal digitam pelo Python durante um
   empréstimo do pane (`rt:427-473`, `rs/runtime/terminal.rs:253-290`).
3. **Serviços e fatos.** O ator e o `Monitor` perguntam ao Python o que ainda mora na memória
   dele (`/internal/runtime/policy`, `/internal/sessions/{n}/state-facts` e `state-service`).

## 1. Rotas de escrita

Todas com `require_auth`; quase todas com `_transfer_guard` (`api:526`, 409 em troca de conta).

| Rota | Handler | Provedores | Caminho Claude hoje |
|---|---|---|---|
| `POST /api/sessions` | `api:2279` (`_criar_sessao` `:2377`) | todos | Validação (`:2392-2503`: provedor, cwd, raiz do convidado 403, config_dir, motor, modo 409, `model_args`), worktree opcional (`:2504`), trava de conta `contas.ciclo_conta` (`:2046`, `contas.py:567`), `registry.create` numa thread (`:2568`). Sem terminal: `acordar` → `ensure_open` (`ra:1175-1203`) → `launch_process` + op `open`. Com terminal: o vínculo nasce no Rust por `_await_birth` (`rc:609`). |
| `POST …/input` | `api:4849` → `_send_one` (`:4354`) | todos | `_recusa_orq` 409, 404; gerenciada → `_send_managed` (`:4700`) → op `submit`. Sem terminal → `_send_one_headless` (`:4663`). `steer=true` → `steer_queue` (`:4878`). |
| `POST …/steer` | `api:4910` | Claude, Codex | control `steer` + op `confirm` (`:4950-4954`). |
| `POST …/interrupt` | `api:6110` | Claude, Codex (409) | control `interrupt {clear}` + `PB.interrompeu` (`:6127-6129`). |
| `POST …/select` | `api:5935` | todos | `perm:` só opção 1/2; `_recusa_se_painel_aberto`; control `select`. |
| `POST …/answer` | `api:9187` | Claude, Pi, Kimi, Codex | `rt.answer_sync` (`rt:1044`) → control `answer_questions`; resposta por chat = empréstimo + `submit`. |
| `POST …/keys` | `api:6463` | com terminal | control `navigation_key`. |
| `POST …/rename` | `api:3399` | todos | terminal: `coordinator.change` (`:3409`) + `_rename_session` (`:3430`: tmux, `registry.rename`, `PromptQueue.rename`, bastão); sem terminal: `registry.rename` → `lifecycle_call`. |
| `DELETE /api/sessions/{n}` | `api:2699` | todos | `registry.kill` (`registry.py:3053`): tmux kill, `close_sync` do sem terminal, atalhos, `_forget`, fila/par; depois `PB.esquecer`, revogação de convite, `_avisar_saida` aos peers. |
| `POST …/recarregar` | `api:2786` | Claude sem terminal, Codex | `hl.recarregar` → `lifecycle_call`. |
| `POST …/model-effort`, `GET …/model/options` | `api:9395`, `:9843` | Claude com terminal | empréstimo de teclado (seção 2). |
| `POST …/model`, `GET …/models`, `POST …/question/skip` | `api:6273`, `:6259`, `:9168` | **só Codex** | 400/409 para Claude. |
| `GET …/commands` | `api:10658` | todos | leitura, não escrita (`_commands_claude` `:10672`). |

**Achado:** o `DELETE` de uma sessão Claude com terminal não fecha o vínculo no Rust; o ator
descobre pela ausência no tmux (`session.dead`, `rs/state/monitor.rs:338`). Nenhum
`coordinator.close`/`change` no caminho (`registry.py:3053-3095`).

### O coordenador (`rc`)

- Modo do processo: `pending`/`rust`/`python` (`rc:28-45`, `:255-318`); `await_mode` espera até
  30 s (`rc:261`).
- `_born_in_rust` (`rc:515`): `transport and provider == "claude" and binding.headless`. Os
  "Codex é sempre do Python" estão em `rc:635-641`, `:679`, `:1026`, `:1248-1250`.
- `op` (`rc:1242`): espera o modo → `prepare_session` → `/clear` sob `freeze` → reabre vínculo
  falho → `_op_once` (`queue_gate`; Rust → `_rpc`; Python → `legacy.op`) → submit perdido é
  repetido com o mesmo `operation_id` (`rc:1307`).
- Comandos que viajam: `open`, `close`, `submit`, `control`, `queue`, `snapshot`, `drain`,
  `confirm`, `ensure_projection`. O gateway Rust aceita cada um com campos fechados
  (`gateway.rs:362-415`).
- O motor Claude sem terminal (`rs/runtime/claude.rs:345-416`) já trata `Input`, `Steer`,
  `Interrupt`, `Select`, `AnswerQuestions`, `SkipQuestion`, `SetPermissionMode`, `SetModel`,
  `SetEffort`, `ListModels`, `Compact`, `Commands`, `Cwd`, `Detach`; `SteerQueue` no ator
  (`actor.rs:421`). Os controles de terminal (`terminal.rs:907-921`) cobrem `input`, `steer`,
  `key`, `interactive_key`, `terminal_input`, `select`, `answer_questions`, `submit_selected`,
  `interrupt` e o empréstimo.
- Roteamento público: `routes.rs:222-261`, padrão `route(path, method(handler).fallback(pass_any))`
  com `gate()` (`:275`) decidindo dono; hoje os únicos POST de sessão reivindicados são os de
  `plugin/*`.

Conclusão: **quase todo o "executar" já existe no Rust**. O que falta para a rota nascer lá é a
validação de entrada (`_recusa_orq`, transferência, painel aberto, convidado, `perm:` 1/2) e os
efeitos colaterais que ainda moram no Python (plugin, par, convite, peers).

## 2. Teclado emprestado

Mecanismo: `Executor.loan` (`terminal.rs:161`), teto 120 s (`:192`), id `loan:{geração}:{seq}`,
idempotente por `request_id`; enquanto emprestado, entrada vira `Deferred keyboard_loan` e mods
recusam (`terminal.rs:537-616`). Lado Python: `_LOAN_S=60` (`rt:28`), prazo local
`pedido + 60 - 1`, cobrado só em `send_keys`/`paste` (`tmux.py:1073,1148`, `:296`) — capturas e
esperas não conferem o prazo.

| Operação | Python | Tamanho | O que faz no pane | Dificuldade |
|---|---|---|---|---|
| `/model` + esforço | `terminal_input.py:2072-2333`, `model_picker.py` (242) | ~500 | abre o seletor (`/model`, Enter, 2º Enter só se nada desenhou), navega com Down/Up, Right até o esforço, confirma `s`/Enter, lê a linha de resultado | média: regex puras com 3 fixtures (`tests/fixtures/pane_model_picker_*.txt`) |
| motor (`set_engine_model`) | `:2183` + `api:10040-10068` | ~60 | `/model <id>` literal e confere o resultado; **dois empréstimos** quando também troca esforço | baixa |
| modo de permissão | `permission_mode.py:262-379` | ~120 | `BTab` até 6×, lendo o rodapé; `listar_modos` dá a volta e retorna | baixa: `parse_permission_mode` já no Rust (`rs/state/permission.rs:26`) |
| `/btw` | `btw.py:136-245` + buffers `:82-133` | ~190 | esvazia o composer (C-u ×12), digita `/btw`, espera "c to copy", copia por buffer do tmux sob trava global, fecha com Esc | **alta**: buffers do psmux, trava global, leitura do composer |
| resposta por chat | `rt:1063-1078`, `api._espera_picker_fechar` (`:5769`) | ~40 | Esc e espera o rodapé sumir; o texto vai pela fila do Rust | trivial: `interrupt()` e o regex do rodapé já no Rust |
| clique de mod (Python) | `plugin_click.py:183` | — | só para sessão que o Rust não serve | sai com a interface dos mods |

**Achado (defeito provável):** `btw.perguntar` espera a resposta até 60 s (`btw.py:148`), mais
6 + 3 + 6 s de abrir/copiar/fechar, dentro de um empréstimo de 59 s. Resposta lenta faz a tecla
`c` ou o Esc levantar `KeyboardLoanExpired` com o overlay aberto, e o Rust retoma o pane com ele
na tela. Some quando o `/btw` passa ao Rust.

**Paridade da resposta por opção:** o Python tem o atalho de dígito 1–9 e o Enter conferido duas
vezes quando a pergunta tem painel de prévia (`terminal_input.py:986-1127`); o Rust só navega com
Down/Up (`rs/terminal_input.rs:765-863`).

## 3. Plugin do Claude (`PB`, 1596 linhas)

Rotas já no Rust (interface dos mods): `ui`, `toast`, `pressed`, `copied`, `press-start`,
`opened`, `focus-target`, `focused`, `scroll`, `…/plugin/press|close|show|input`
(`rs/mods/*`, `migration_status.rs:115-116`).

Ainda no Python: `whoami` (`PB:1055`), `pull` (`:1123`, long-poll de 25 s), `suggest`
(`:1212`), `ask` (`:1428`), `ask-fim` (`:1490`), `filled` (`:1523`), `submitted` (`:1548`),
`state` (`:1562`), `rate` (`:1591`). O lado do plugin está em `plugins/hangar/hooks/`
(`input.ts`, `perm.ts`, `ask.ts`, `state.ts`, `suggest.ts`, `rate.ts`, `ui.ts`, `bridge.ts`).

Estado todo em memória, sob uma trava (`PB:51`): `_waiters`, `_donos`, `_publications`,
`_estados`, `_sugestoes`, `_bands`, `_perguntas`, `_fechadas`, `_batidas`, `_confirmacoes`,
`_recusas`. Em disco só `~/.hangar/plugin.json` e `~/.hangar/plugin-dir`. O token é HMAC
determinístico, igual ao `mint` do Rust (`rs/mods/bridge.rs:57`), por isso sobrevive ao restart.

Fluxos que dependem dessa memória:

- **Entrega pelo plugin** (`publish_terminal`, `PB:93`): o Rust pede pelo serviço
  `terminal_publish` (`terminal.rs:148`, prazo 40 s) e o Python enfileira no long-poll.
- **Permissão segurada** (`perm:<tool_use_id>`): fica com o plugin só com app presente, sem
  modo `auto`/`dontAsk` e ninguém no terminal (`PB:1436`, regra `harnesses.md:197`).
- **AskUserQuestion segurado** (`ask:<id>`): corrida entre o diálogo da TUI e o `/ask`.
- **Resposta**: `responder_pergunta` (`PB:1376`), idempotente por id e recibo; o Rust chega a ela
  pelo serviço `terminal_plugin_control` (`terminal.rs:636-645` → `rt:366-410`).
- **Fatos do `Monitor`** (`state-facts`, `internal_api.py:282` → `state_facts.py:38-46`):
  `plugin_state`, `waiter_open`, `heartbeat_age_ms`, `question`, `suggestion`, `body_columns`,
  `band_anchor`, `in_transfer_ms`, `transfer_active`, `permission_op`.

**Achados:**
- A regra `harnesses.md:381` diz "sem long-poll por mais de 3 s"; o código usa
  `SEM_POLL_S = 5.0` (`PB:47`).
- O fato `question` que o Rust recebe vem cru: o Python aplica `interrompida` e o teto de 5 s sem
  poll em `pergunta_pendente` (`PB:1358`), o Rust só aplica os 35 s (`rs/state/facts.rs:84`).
  Os dois discordam sobre quando uma pergunta interrompida some.

Quem lê essa memória no Python além das rotas: `sse.py:1097-1313` (porta do convidado e Connect),
`preview.py`, `state.py` (modo `python`), `terminal_input.py:688-699`, `list_facts.py:166-172`.

## 4. Serviços que o Rust pede ao Python

Chamada: `PolicyClient::run_for` (`rs/runtime/actor.rs:37-60`) → `POST /internal/runtime/policy`
(`internal_api.py:64`) → `runtime_policy.run` (`runtime_policy.py:147-254`).

| Serviço | Python | Quem chama no Rust e quando | Dependência | Destino |
|---|---|---|---|---|
| `terminal_facts` | `rt:308-350` | `terminal.rs:135-140`; antes de **cada entrega** e de cada drenagem | captura e classificação do composer, plugin, `hook_state`, pergunta, socket nativo, clipboard do Windows | porta (seção 5C da spec) |
| `terminal_publish` | `rt:353-362` | `terminal_input.rs:697` | long-poll do plugin | some com o plugin |
| `terminal_plugin_control` | `rt:365-403` | `terminal.rs:640` | `responder_pergunta` | some com o plugin |
| `prepare_prompt` | `runtime_policy.py:159-170` → `_blocos_do_prompt` (`adapter.py:2213-2243`) | `actor.rs:822`, **cada** Input/Steer, Claude e Codex | leitura de imagem + `uds_messaging.separar_prefixo` | porta (pura) |
| `format_status` | `:179-190` → `status_line` (`adapter.py:1908-1933`); Codex `format_status_line` (`codex/adapter.py:319-345`) | `claude.rs:232` (com `FormatGate`, `protocol.rs:18-33`), `:681`; `codex.rs:273` | texto puro; **janelas de cota** vêm de `cotas.listar_cotas()` (HTTP + OAuth, parte 6) | texto porta; cota vira fato do Python |
| `last_usage` | `:191-199` → `adapter.py:2359` | `claude.rs:328` | cauda do `.jsonl` | porta |
| `reload_stamp` | `:200-203` → `_marca_config` (`adapter.py:2482`) | `claude.rs:329`, `:857` (**a cada 10 s** por sessão sem terminal) | `.claude.json` + `settings.json`, SHA1 | porta, hash idêntico |
| `native_message` | `:79-101` | `actor.rs:852` | socket nativo; origem `from=uds:<INBOX>` é o socket do **Python** | porta com inbox próprio |
| `unknown_private` | `:58-76` | `claude.rs:636,704`, `codex.rs:971` | anexa em `privado/*-desconhecidos.jsonl`, tetos | porta (trivial) |
| `session.patch_meta` | `:210-240` | `claude.rs:549,590,715,742,848`; `codex.rs` | sidecar JSON lido pelo registro Python | porta com cuidado (leitores Python) |
| `skill_catalog` | `:171-174` | `codex.rs:800` | puro | porta (metade Codex) |
| `answer_body` | `:175-178` | **nenhum** (Rust tem o seu, `claude.rs:931`) | — | apagar |
| `session.marker` | `:241-247` | **nenhum** | — | apagar |
| `diag.error` | `:248-253` | **nenhum** (Rust usa `/internal/diag`) | — | apagar |
| `quota` | `:204-205` | **nenhum emissor** | — | apagar |

`state-service` (`internal_api.py:292` → `runtime_policy.py:112-144`), só Claude com terminal:

- `permission.observe` → `permission_mode.observar_ou_confirmado` (`permission_mode.py:90`),
  memória por sid no Python; o `Monitor` chama quando o modo lido do pane vence
  (`monitor.rs:394`).
- `session.dead` → se `em_troca` responde `em_troca`; senão `PB.esquecer` + `forget_frame`
  (`monitor.rs:338`).
- `session.deliverable` → `get_adapter("claude").drain` — o adaptador de **terminal**
  (`adapters/claude.py:28`), que volta ao Rust como control `drain` (`monitor.rs:273`). Volta de
  ida e volta sem motivo: o Rust já tem `drain_once`.

`/internal/list/demote` (`internal_api.py:202` → `hook_state.demote_awaiting`,
`hook_state.py:163`): o pane contradiz o marcador `awaiting_input`; o Python rebaixa na memória e
regrava `.hangar-state/<sid>.json`. A leitura dos marcadores já é do Rust
(`rs/list/facts_files.rs`, `rs/state/demote.rs`); a memória e a drenagem disparada pela transição
(`api._on_hook_transition`, `api:322-324`) seguem no Python.

`/internal/list/facts` (`list_facts.py`) serve sobretudo provedores não migrados, contas,
transferências, orq e acesso: fica fora desta metade, exceto `held` (pergunta segurada), que passa
a vir do plugin no Rust.

### O que fica morto no `claude_headless/adapter.py`

Portar os serviços solta `_blocos_do_prompt`, `status_line` e seus rótulos, `_uso_da_ultima_chamada`
e os tetos de desconhecidos **só no modo `rust`**. No modo `python` (reserva do processo inteiro)
o adaptador inteiro segue vivo. `_marca_config` continua vivo enquanto o Python lança o cano
(`adapter.py:757,1191`). Nada sai do arquivo nesta parte; a remoção é da parte 7.

## 5. Itens do pedido "parte 5 ou parte 6"

| Item | O que é | Toca a escrita do Claude? |
|---|---|---|
| `model_picker.py` (242) | parser do seletor `/model` da TUI | sim: é o que obriga o empréstimo do `/model-effort` |
| `engines.py` (569) | motores em `engines.json`, env da subida (`registry.py:2352,2663,2694`), rotas `/api/engines*` (área "accounts" em `migration_status.rs:83`) | só na criação e no relançamento |
| `hook_state.py` (238) | memória sid → estado a partir dos marcadores | sim: demote e drenagem pela transição |
| `hook_installer.py` (438) | grava hooks no `settings.json` de cada conta no boot | não; é configuração de conta |
| `skill_bridge.py` (296) | links de skills para Pi, Kimi e Codex | não |

## 6. Windows

- Esc cru se perde no ConPTY: os dois lados mandam a sequência Win32
  (`tmux.py:1077-1080`, `rs/terminal_input.rs:451`; `docs/decisoes/windows.md:210-226`).
- Buffers do psmux são por sessão (`-t =<sessão>`, `-b` ignorado, OSC 52 cria dois buffers;
  `btw.py:82-133`, `windows.md:470-492`). Não conferido se o `raw` do Rust (`terminal_input.rs:487`)
  devolve a saída de `list-buffers`/`show-buffer`.
- Texto com `\` ou quebra de linha vai pelo clipboard com `clipboard.lock`; o fato
  `clipboard_available` compara a sessão do Windows (`rt:342-345`).
- `/btw` confere o composer antes do Enter porque a `/` se perdia no Windows (`btw.py:166-205`).
