# Lista de sessões + estado no Rust — plano de implementação

> Execução só depois da aprovação do dono. Desenho: `desenho.md` (com a revisão adversarial
> incorporada). Inventário: `inventario.md`. Medição de partida: `medicao.md`. Linhas conferidas em
> `f98798166` (merge de `origin/hangar-server-parte1` em `5c2f0c637`); cada Task reconfere as dela
> na base antes de codar.

**Objetivo:** com o Rust de pé, ele é o único dono da descoberta, da resolução do transcript, da
lista (`GET /api/sessions`, `/api/sessions/events`) e, na Fase C, do estado do chat das sessões
Claude com terminal; o Python fornece fatos do que ainda é dele e lê a descoberta do Rust. Antes de
tudo, o Supervisor para de varrer a máquina.

## Restrições globais

- **Testes em cada Task:** escritos primeiro, vistos falhar sem o código da Task, rodados só os dos
  arquivos tocados (`cd backend && uv run pytest tests/<x>.py`;
  `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test <x>` ou `--lib list::`).
  No máximo 2 `cargo` na máquina; cada worktree com o próprio `target/`, apagado ao terminar.
- **Paridade por entradas gravadas:** descoberta, decoração e classificação são funções puras sobre
  entradas (processos, panes, arquivos, relógio, caches). O gerador Python
  (`backend/tests/fixtures/contract/gen_list.py`) roda o código atual com `procinfo` e `tmux`
  trocados por fakes que **registram as entradas**, em sequências de tiques com relógio injetado, e
  grava entradas + saídas; o Rust repete as mesmas entradas e compara tique a tique. Nenhum dado real
  de conversa. Mudou a regra no Python durante a execução → regenerar no mesmo commit.
- **Contrato interno:** sobe nos dois lados (`backend/app/rust_server.py:35`,
  `crates/hangar-server/src/lib.rs:25`) na Task 14 (Fase B) e na Task 19 (Fase C), cada uma no
  próximo número livre na hora da junção (hoje 22 → 23 e 24), nunca reaproveitando.
- **Cada Task corrige, no mesmo commit, a regra de `CLAUDE.md` ou `docs/decisoes/` que ela torna
  falsa**; a Task 23 só cuida do que sobra.
- Windows: ler "Regras vigentes" de `docs/decisoes/windows.md` antes de subprocesso, processo,
  trava de arquivo ou teste que simula plataforma; job Windows do CI conferido pelo log.
- Harnesses: ler "Regras vigentes" de `docs/decisoes/harnesses.md` antes de estado, hooks, plugin,
  observação ou entrega.
- Nenhuma sessão real nem backend real. Backend de prova isolado (`HOME` temporário, portas fora de
  8765/8766/8768, `tmux -L` próprio, lançador com `matar_orfaos` desligado); sessões Claude só
  Haiku em `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`.
- Log e diário só com código, motivo curto, nome da sessão e nome de campo; nunca texto de
  conversa, pergunta, rótulo ou statusline.
- Identificador novo em inglês; `git add` por caminho; commits em inglês; merge, nunca rebase.

## Lotes — o que corre em paralelo

- **Já:** Task 1 (Supervisor, só Python).
- **Fase A (o Rust monta a lista, nada muda para o usuário):** Tasks 2, 3 e 5 em paralelo. Depois da
  3 e da 5: Tasks 4, 6 e 7 em paralelo. Depois da 2 e da 5: Tasks 8, 9, 10 e 11 em paralelo. Task
  12 depois da 6 e da 8.
- **Fase B (o Rust vira dono):** Task 13 depois da 6, 7 e 12; Task 14 depois da 13; Task 15
  (sombra) depois da 14, e só ela vai ao canal de testes nesta fase; Task 16 (consumidores Python)
  depois da 15; Task 17 (rotas) depois da 16; Task 18 depois da 17.
- **Fase C (estado do chat):** Task 19 depois da 17; Task 20 depois da 19; Tasks 21 e 22 em paralelo
  depois da 20 (a 22 também depois da 4).
- **Por último:** Task 23 (documentação) e Task 24 (prova de uso real).
- Junção na ordem dos números; conflito de número de contrato resolve pela regra acima.

## Foco da revisão

- Com o modo `rust` ou `pending`, nenhum caminho roda a descoberta Python (`registry.list`,
  `resolve_tracked`, `list_with_state`, `_ListRefresher`) nem, depois da Fase C, `StateMonitor` de
  Claude com terminal.
- Erro da ponte ou dos fatos nunca vira lista vazia, lista do Python ou estado inventado: levanta,
  `list_error`, 503 com código ou `problema` na linha.
- Um produtor de lista por servidor; nenhum SSE ou captura por card.
- Entrega continua passando pelas travas de hoje (`adapter.drain`); o Rust só avisa.
- Divisão por plataforma (Windows sem `-C`) e por provedor (não migrados no Python) intacta.

---

## Fase 0

### Task 1: Supervisor sem varrer a máquina e sem regravar o registro à toa

**Arquivos:** `backend/app/runtime_process.py` (`refresh_members` e uma função nova de descida pela
árvore; `_group_members` intocada), `backend/tests/test_runtime_process.py`,
`docs/decisoes/plataforma.md` (entrada do hangar-server: a regra e a medida).

Cada volta de 0,25 s do vigia (`rust_server.py:378-382`) roda `_group_members`
(`runtime_process.py:148-158`: `psutil.process_iter` em todo processo da máquina, 9,5–11 ms) e
regrava `runtime-process.json` com `fsync` (`:248-282`). Muda só o `refresh_members`: no Linux,
desce a árvore do Rust por `/proc/<pid>/task/*/children` conferindo `os.getsid`; a varredura inteira
fica como reforço a cada 5 s (neto cujo pai morreu sai da árvore e segue na sessão) e sempre fora do
Linux; grava só quando `members` ganhou pid, na primeira volta, ou enquanto a última gravação falhou
(`:273`, `rust_server.py:383-393`). `cleanup` (`:189`) e `reconcile_startup` (`:321`) continuam com
a varredura inteira. No Windows (Job) só muda a regra de gravação.

**Falha sem ela:** `test_refresh_members_writes_only_when_members_change`,
`test_refresh_members_follows_tree_without_full_scan`, `test_full_sweep_catches_orphan_in_session`,
`test_windows_job_refresh_writes_once`, `test_refresh_members_retries_after_failed_write`,
`test_reconcile_still_finds_orphan_grandchild`.

- [x] **Step 1: Ler "Regras vigentes" de `docs/decisoes/windows.md`; conferir as linhas citadas**
- [x] **Step 2: Testes acima, vistos falhar**
- [x] **Step 3: Descida pela árvore só no `refresh_members`, reforço de 5 s, gravação só com mudança ou falha anterior**
- [x] **Step 4: Medida no isolado: zero sessões e sem cliente abaixo de 1% de um núcleo (era 4,2%), registro regravado só com processo novo; registrar em `medicao.md` e `plataforma.md`; testes focados; revisar**

---

## Fase A — o Rust monta a lista (nada muda para o usuário)

### Task 2: Tipo da linha da lista em `hangar-api`

**Arquivos:** `crates/hangar-api/src/session.rs` (novo: `SessionRow`, `ContextUse`, campos e ordem de
`backend/app/models.py:73-210`), `crates/hangar-api/src/lib.rs`,
`backend/tests/fixtures/contract/gen_api_samples.py` (`session_full`, `session_minimal`),
`crates/hangar-api/tests/contract.rs`.

**Falha sem ela:** `session_row_roundtrip` (amostra do Python lida, escrita e comparada canônica).

- [x] **Step 5: Amostras do `SessionInfo` real; teste visto falhar**
- [x] **Step 6: `SessionRow` com `null` no que falta e leitura que ignora campo desconhecido; testes focados; revisar**

### Task 3: Leitores de processos e panes no Linux

**Arquivos:** `crates/hangar-server/src/list/mod.rs` (pasta `list/` nova), `list/procs.rs` (traço `ProcessView`: pid, ppid,
argv, cwd, ambiente sob demanda, nascimento, fds; leitor `/proc` com mapa de filhos em cache de 3 s
como `procinfo.py:60` e leitura fresca sob pedido), `list/mux.rs` (`list-panes -a` com os 8 campos de
`backend/app/tmux.py:391-432`, prazo de 5 s).

**Falha sem ela:** `procs::reads_children_and_argv`, `procs::fresh_read_sees_new_child` (o caso da
sessão criada há menos de 1 s), `mux::parses_list_panes_fields` (inclusive `@cp_hidden`,
`CP_PROVIDER`, nome com ponto), `mux::timeout_is_unavailable` (→ `MuxUnavailable`, nunca vazio).

- [ ] **Step 7: Testes, vistos falhar**
- [ ] **Step 8: Leitores; testes focados; revisar**

### Task 4: Leitores de processos e panes no Windows e no macOS

**Arquivos:** `crates/hangar-server/Cargo.toml` (`sysinfo`, só `cfg(not(target_os = "linux"))`),
`list/procs.rs`, `list/mux.rs` (psmux: endereço `=<sessão>:<janela>.<pane>`, `PSMUX_SESSION`, byte
inválido trocado), `docs/decisoes/windows.md` (regra nova, se houver).

**Falha sem ela:** `mux::parses_psmux_list_panes` (`%N` repetido entre sessões),
`procs::sysinfo_reads_argv_and_env` (nos três sistemas no CI).

- [ ] **Step 9: Ler "Regras vigentes" de `docs/decisoes/windows.md`; testes, vistos falhar**
- [ ] **Step 10: Leitor `sysinfo` e parse do psmux; jobs Windows e macOS do CI conferidos pelo log; revisar**

### Task 5: Gerador de entradas e saídas gravadas

**Arquivos:** `backend/tests/fixtures/contract/gen_list.py`, `golden/list_discovery.json`,
`golden/list_decorate.json`, `golden/list_state.json`, `backend/tests/test_contract_list.py` (todos novos).

Fakes de `procinfo`, `tmux` e relógio que registram cada leitura; casos como sequências de tiques.
Mínimo: Claude por fd aberto (inclusive o `_fd_locked` no meio de uma escrita), por marcador, por
`--session-id` com e sem irmão, depois de `/clear`, por pid, mais novo; semente na criação e
esquecimento; sessão criada há menos de 1 s; Pi, omp e Kimi por bilhete (e `PSMUX_SESSION`); Codex
com e sem thread; Claude sem terminal; par, encadeamento, loop, worktree e worktree sumida, motor,
conta; colisão; marcador `awaiting` velho; `_idle_conferido`; teto e validade da statusline;
limite; segunda captura do spinner; contexto 200k e 1M; plano com e sem pino; última resposta.

**Falha sem ela:** `test_list_golden_is_current` (regenera em memória e compara).

- [x] **Step 11: Fakes que registram, casos acima; teste visto falhar sem o arquivo**
- [x] **Step 12: Gerar; testes focados; revisar (nenhum texto real)**

### Task 6: Descoberta e resolução das sessões Claude

**Arquivos:** `crates/hangar-server/src/list/discover.rs` (pura: entradas + caches → linhas; pane do
agente, provider pelo argv, `resolve_tracked` na ordem de `registry.py:1064-1190`; caches de
resolução com semear, esquecer e renomear), `crates/hangar-server/tests/contract_list.rs` (novo).

**Falha sem ela:** `contract_list::discovery_claude_sequences`, `discover::seed_then_forget`.

- [ ] **Step 13: Testes, vistos falhar**
- [ ] **Step 14: Porte na ordem do Python, fd aberto só no Linux; testes focados; revisar**

### Task 7: Descoberta dos demais provedores e campos comuns

**Arquivos:** `list/discover_other.rs` (bilhetes Pi/omp/Kimi `registry.py:739-888`, sidecars Codex e
Claude sem terminal `:1438-1480`), `list/links.rs` (par, encadeamento, loop, worktree, motor, conta,
vida da sessão `:1376-1431`; colisões `:1219-1262`), `tests/contract_list.rs`. Linhas `orq` e de
transferência não entram: vêm dos fatos (Task 14).

**Falha sem ela:** `contract_list::discovery_other_sequences`, `contract_list::links_cases`.

- [ ] **Step 15: Testes, vistos falhar**
- [ ] **Step 16: Porte; testes focados; revisar**

### Task 8: Marcadores, registro nativo, pergunta aberta, statusline e assinatura

**Arquivos:** `list/facts_files.rs` (`.hangar-state/<sid>.json`, `<config>/sessions/<pid>.json` com a
regra de pid vivo de `hook_state.py:56-129`, `.hangar-askq`, `.hangar-status` com idade de 1 dia),
`list/sig.rs` (`_status_sig`, `_context_sig`, `_list_sig`, `sse.py:290-401`), `tests/contract_list.rs`.

**Falha sem ela:** `contract_list::decorate_markers_cases`, `contract_list::list_sig_cases`.

- [ ] **Step 17: Testes, vistos falhar**
- [ ] **Step 18: Leitores e assinatura; testes focados; revisar**

### Task 9: Contexto e modelo da sessão Claude

**Arquivos:** `list/context.rs` (porte de `claude_context.py:36-72` e `_claude_reading`,
`registry.py:905-918`), `tests/contract_list.rs`.

**Falha sem ela:** `contract_list::context_cases`.

- [ ] **Step 19: Teste, visto falhar**
- [ ] **Step 20: Porte com cache de 20 s por (nome, transcript); testes focados; revisar**

### Task 10: Progresso do plano

**Arquivos:** `list/plan.rs` (porte de `backend/app/planprog.py`), `tests/contract_list.rs`.

**Falha sem ela:** `contract_list::plan_cases`.

- [x] **Step 21: Teste, visto falhar**
- [x] **Step 22: Porte com cache por mtime e descoberta de 3 s; testes focados; revisar**

### Task 11: Última resposta da sessão parada

**Arquivos:** `list/reply.rs` (histórico Rust existente, `transcript/history.rs`; corte de 160 como
`archive._texto_simples`), `tests/contract_list.rs`.

**Falha sem ela:** `contract_list::last_reply_cases` (Claude, Claude sem terminal, Codex).

- [ ] **Step 23: Teste, visto falhar**
- [ ] **Step 24: Porte com cache por (transcript, mtime); testes focados; revisar**

### Task 12: Classificação da lista

**Arquivos:** `list/classify.rs` (pura sobre quadros e relógio: marcador primeiro; pane sem marcador,
com `awaiting` ou ocioso velho; segunda captura em 0,15 s; statusline com teto de 2 e 20 s; limite
30 s; pedido de rebaixar `awaiting` como **efeito devolvido**, executado pelo serviço do Python na
Fase B; Claude sem terminal pelo `RuntimeRegistry`; `registry.py:1691-1847`), traço `CaptureSource`
(subprocesso `capture-pane`; fake nos testes), `tests/contract_list.rs`.

**Falha sem ela:** `contract_list::state_sequences`, `classify::asks_demote_after_grace`,
`classify::headless_from_runtime`.

- [ ] **Step 25: Testes, vistos falhar**
- [ ] **Step 26: Classificação com `terminal_state::analyze`; testes focados; revisar**

---

## Fase B — o Rust vira dono

### Task 13: Ponte privada `list.*` (Python → Rust)

**Arquivos:** `crates/hangar-server/src/list/bridge.rs` + rota na porta privada (ao lado de
`/__hangar_server/workspace`, `routes.rs:187-192`, mesmo segredo e loopback),
`backend/app/list_bridge.py` (novo, cliente no molde de `workspace_bridge.py`), testes nos dois lados.

Operações: `list.discover` (cache 1 s e um por vez; `newer_than` + leitura fresca de processos para a
sessão criada há menos de 1 s, `api.py:1025-1028`), `list.snapshot` (decorado até 2 s; mais velho,
invalidado ou sem cliente → produz na hora), `list.invalidate`, `list.resolve`, `list.seed`,
`list.forget`, `list.rename`. Erro **levanta**; `mux_unavailable` → `tmux.MuxIndisponivel`.

**Falha sem ela:** Rust `list_bridge::requires_secret`, `list_bridge::discover_is_single_flight`,
`list_bridge::newer_than_forces_fresh`; Python `test_list_bridge_returns_rows`,
`test_list_bridge_error_raises_never_empty`, `test_list_bridge_mux_unavailable_raises_mux_error`.

- [ ] **Step 27: Testes, vistos falhar**
- [ ] **Step 28: Rota privada e cliente; testes focados; revisar**

### Task 14: Fatos por pergunta e resposta e rebaixamento pelo Python (contrato 23)

**Arquivos:** `backend/app/list_facts.py` (novo), `backend/app/internal_api.py`
(`POST /internal/list/facts`), `backend/app/runtime_policy.py` (`hooks.demote_awaiting` →
`hook_state.demote_awaiting`, `hook_state.py:163-189`), `crates/hangar-server/src/list/facts.rs`
(cliente, prazo de 1 s, último valor + `problema = list_facts_unavailable` na falha),
`crates/hangar-server/tests/fake/mod.rs`, `rust_server.py:35`, `lib.rs:25`, testes nos dois lados.

Pedido: linhas descobertas + contagem de clientes do dono. Resposta: estado das linhas Codex, Pi, omp
e Kimi (código de hoje, `registry.py:1572-1690`, `1764-1780`, só sobre essas linhas); substituições
por nome e linhas sintéticas de transferência (`:165-206`); linhas `orq` (`:1483-1491`);
`shared`/`owner`/escondida do dono; mapa de navegador vivo (`sse.py:254-263`); terminais de atalho
(última leitura boa, sem esperar); a presença do app ajustada pela contagem (`sse.py:593-594,
669-670`).

**Falha sem ela:** Python `test_facts_compute_only_non_migrated_rows`,
`test_facts_transfer_overrides_by_name`, `test_facts_owner_count_drives_app_presence`,
`test_facts_nav_expired_not_returned`, `test_demote_service_updates_map_and_file`; Rust
`list_facts::timeout_keeps_last_and_marks_problem`, `list_facts::transfer_rows_not_classified`;
contrato 23 nos dois lados.

- [ ] **Step 29: Testes, vistos falhar**
- [ ] **Step 30: Rota de fatos, serviço de rebaixamento, cliente Rust; contrato 23 nos dois lados**
- [ ] **Step 31: Testes focados; revisar**

### Task 15: Rodada em sombra

**Arquivos:** `crates/hangar-server/src/list/shadow.rs` (com `CP_LIST_SHADOW=1`: produz a cada
tique sem servir, compara a assinatura de cada linha com a do Python, que o Python manda junto no
pedido de fatos, e grava `list.shadow_diff` no diário com nome da sessão e nome do campo),
`backend/app/list_facts.py` (anexa as assinaturas do `_ListRefresher` atual), testes.

O Python continua dono da lista nesta Task; nada do Rust é entregue.

**Falha sem ela:** `shadow::diff_names_fields_only` (diferença no `label` grava `label`, nunca o
texto), `shadow::off_by_default`.

- [ ] **Step 32: Testes, vistos falhar**
- [ ] **Step 33: Sombra; testes focados; revisar**
- [ ] **Step 34: Canal de testes na máquina do dono por 2 dias de uso normal; zerar as diferenças ou registrar cada uma aceita em `desenho.md` (verificação manual)**

### Task 16: Consumidores Python pela ponte

**Arquivos:** `backend/app/registry.py` (`list()`, `list_with_state()` e `resolve_tracked()` pela ponte
no modo `rust`/`pending`; sementes e esquecimento — `:2302, 2359, 2448, 2599, 2682, 2786, 2819,
3166` e `_forget` — chamam `list.seed`/`list.forget`/`list.rename`; varredura de pares mortos sai de
`list()` para laço próprio de 2 s que levanta em erro), `backend/app/sse.py` (`_ListRefresher` não
sobe no modo `rust`; convidado lê `list.snapshot`), `backend/app/stall_watch.py`,
`backend/app/api.py` (`_on_hook_transition` `:1500-1559`, `list_sessions` do convidado,
`_invalidate_lists` chama `list.invalidate`), `backend/app/prune.py` (`:198-207`: erro da ponte não
vira "ninguém vivo"), testes.

Auditoria antes de codar: todo leitor dos caches de instância do `registry` (`_jsonl_cache`,
`_agent_pid`, `_fd_locked`, `_status_cache`, `_label_cache`) e todo chamador de `resolve_tracked`
(`runtime_terminal.py:115`, `plugin_bridge.py:936`, `api.py:4023, 4120, 4257`).

**Falha sem ela:** `test_registry_list_uses_bridge_in_rust_mode` (descoberta Python trocada por uma
que falha), `test_resolve_tracked_uses_bridge_in_rust_mode`, `test_create_seeds_rust_cache`,
`test_registry_waits_pending_then_fails_with_code`, `test_list_refresher_never_starts_in_rust_mode`,
`test_stall_watch_reads_bridge_snapshot_without_clients`, `test_prune_keeps_files_on_bridge_error`,
`test_guest_list_filters_bridge_snapshot`, `test_dead_pair_sweep_outside_discovery`.

- [ ] **Step 35: Auditoria; testes, vistos falhar**
- [ ] **Step 36: Delegação no modo `rust`/`pending`; o código atual só no modo `python`**
- [ ] **Step 37: Testes focados; revisar**

### Task 17: `ListHub` — as rotas da lista no Rust

**Arquivos:** `crates/hangar-server/src/list/hub.rs` (produtor único por servidor: tique de 1,5 s
depois do trabalho, descoberta + decoração + classificação + fatos + rebaixamentos pelo serviço,
assinatura, JSON só quando muda; retrato para o `GET` e para a ponte), `routes.rs` (as duas rotas
para o dono em `GET`; demais métodos e convidados ao Python), `crates/hangar-server/tests/list_routes.rs` (novo),
`CLAUDE.md` (regra: lista do dono é do Rust, Python fornece fatos), `docs/decisoes/plataforma.md`.

Contrato público igual a `sse.py:578-671`: `sessions`, `list_error` uma vez na transição,
`shortcut_terminals`, `nav` por cliente, `ping` 8 s, comentário 15 s, `Cache-Control: no-store`;
`GET` com retrato de até 2 s, fresco depois de invalidação; 503 `erro_mux_indisponivel`
(`api.py:574-590`); linha escondida do dono não sai.

**Falha sem ela:** `list_routes::one_producer_for_many_clients`,
`list_routes::error_then_recovery_reemits_same_list`, `list_routes::nav_once_per_client`,
`list_routes::get_after_invalidate_is_fresh`, `list_routes::mux_unavailable_is_503`,
`list_routes::guest_token_goes_to_python`, `list_routes::hidden_from_owner_not_listed`,
`list_routes::facts_down_marks_rows_not_list`.

- [ ] **Step 38: Testes, vistos falhar**
- [ ] **Step 39: `ListHub` e rotas; diário `rust.list_failed` com código**
- [ ] **Step 40: Regra no `CLAUDE.md` e em `plataforma.md`; testes focados; revisar**

### Task 18: Lista acordada por arquivo

**Arquivos:** `list/hub.rs` (`notify` nas pastas `.hangar-state`, registro nativo e `.hangar-askq` das
contas; mudança reclassifica a sessão e publica na hora, coalescida em 150 ms), testes.

**Falha sem ela:** `list_routes::marker_change_publishes_without_tick`,
`list_routes::burst_of_writes_coalesces`.

- [ ] **Step 41: Testes, vistos falhar**
- [ ] **Step 42: Observador e coalescência; testes focados; revisar**

---

## Fase C — estado do chat das sessões Claude com terminal

### Task 19: Fatos e serviços do estado do chat (contrato 24)

**Arquivos:** `backend/app/list_facts.py` (por sessão Claude com terminal: `plugin_bridge.estado_recente`
`:832`, `pergunta_pendente` `:1273`, `vivo` `:807`, `sugestao` `:511`, `em_troca`
(`backend/app/adapters/claude_headless/sessions.py:88`), transferência ativa, operação de permissão controlada),
`backend/app/runtime_policy.py` (`permission.observe` → `permission_mode.py:90-104`; `session.dead` →
`plugin_bridge.esquecer` + `forget_frame`; `session.deliverable` → o `adapter.drain` de
`sse.py:1224-1237`), `backend/app/list_bridge.py` (`state.wake(name)`: o evento do plugin acorda o
`Monitor`), `crates/hangar-server/src/list/facts.rs`, contrato nos dois lados, testes.

**Falha sem ela:** Python `test_plugin_event_wakes_rust_monitor`,
`test_permission_observe_controlled_then_released`, `test_deliverable_service_goes_through_drain_guards`,
`test_dead_service_forgets_plugin_state`; Rust `list_facts::state_facts_roundtrip`.

- [ ] **Step 43: Testes, vistos falhar**
- [ ] **Step 44: Fatos, serviços e aviso; contrato 24 nos dois lados; testes focados; revisar**

### Task 20: `Monitor` de estado no Rust

**Arquivos:** `crates/hangar-server/src/state_monitor.rs` (novo: aluguel em processo no
`TerminalPool`, 0,75 s ou ao aviso, `reduce` com os fatos, `shells`, loop, overlay, login, limite,
`dead` respeitando `em_troca`, permissão pelo serviço quando a leitura muda ou há operação
controlada), `terminal_state.rs` (deixa de ser referência), `side.rs` (fonte própria de `state`),
`backend/app/sse.py` (`StateMonitor` não sobe para Claude com terminal no modo `rust`, `:730, 1038`),
`gen_terminal.py` (sequências → eventos do `StateMonitor`), `tests/contract_terminal.rs`,
`docs/decisoes/harnesses.md`.

**Falha sem ela:** `contract_terminal::monitor_sequences` (spinner congelado, debounce, graça do
marcador, âncora do plugin, `em_troca`, `dead`), `state_monitor::one_lease_per_session`; Python
`test_no_python_state_monitor_for_rust_terminal`.

- [ ] **Step 45: Ler "Regras vigentes" de `docs/decisoes/harnesses.md`; testes, vistos falhar**
- [ ] **Step 46: `Monitor` e fonte própria no hub; Python para de produzir `state` dessas sessões**
- [ ] **Step 47: Regra em `harnesses.md`; testes focados; revisar**

### Task 21: Pergunta nativa, sugestão, problema e aviso de entrega

**Arquivos:** `state_monitor.rs` (`ask_question` uma vez por pergunta, porte de `sse.py:104-183`;
`suggest` pelo fato; `problema` do runtime; borda "voltou a aceitar texto" → serviço
`session.deliverable`), `side.rs`, `gen_golden.py`, testes.

**Falha sem ela:** `state_monitor::ask_question_once_per_prompt`,
`state_monitor::deliverable_edge_calls_service_once`, `state_monitor::runtime_problem_in_state`.

- [ ] **Step 48: Testes, vistos falhar**
- [ ] **Step 49: Porte; o caminho Python correspondente só no modo `python`; testes focados; revisar**

### Task 22: Windows — captura avulsa pelo psmux

**Arquivos:** `list/classify.rs`, `state_monitor.rs` (`CaptureSource` por subprocesso no psmux;
`-C` continua desligado, `terminal_control.rs:173`), testes `cfg(windows)` com programa falso,
`docs/decisoes/windows.md`.

**Falha sem ela:** `capture::psmux_reads_output_not_exit_code`, `state_monitor::windows_uses_subprocess`.

- [ ] **Step 50: Ler "Regras vigentes" de `docs/decisoes/windows.md`; testes, vistos falhar**
- [ ] **Step 51: Fonte por subprocesso; job Windows do CI pelo log; revisar**

---

## Fase D

### Task 23: Documentação restante

**Arquivos:** `CLAUDE.md`, `docs/migracao-rust/README.md` (partes e "Ainda no Python"),
`docs/decisoes/superado.md` (`_ListRefresher`, `resolve_tracked` Python e `StateMonitor` de Claude
com terminal como donos), `inventario.md`.

- [ ] **Step 52: Varrer `CLAUDE.md` e `docs/decisoes/` por regra que ficou falsa; corrigir; revisar**

### Task 24: Prova de uso real

**Arquivos:** `docs/migracao-rust/lista-estado/prova-real.md`.

- [ ] **Step 53: Isolado com 1, 10 e 20 sessões (método de `medicao.md`): CPU do Python e do Rust em repouso e ativo, latência até a lista ver a mudança, zero descoberta Python no modo `rust` (contador)**
- [ ] **Step 54: Reserva: `CP_RUST_SERVER=0` e Rust derrubado 3 vezes; a lista volta pelo Python igual**
- [ ] **Step 55: Uso real com o dono: celular/PWA, app e nativo, dois servidores, convidado, sessão Pi/Codex, navegador com a sessão fora da tela, pergunta nativa, permissão segurada no app, troca terminal↔sem terminal (verificação manual)**
- [ ] **Step 56: VM Windows (DELPHI-02): lista, estado do chat e psmux lento (verificação manual)**

## Pergunta ao dono

Uma, fechada. A Fase C depende de entrega, permissão, plugin e trocas, que ainda são do Python;
cada um vira fato ou serviço novo (Task 19).

- **A) Fases 0–B agora; a Fase C entra na parte 4**, junto da prévia e da entrada no terminal. ➡️
- **B) Tudo agora**, em duas levas no canal de testes (Fases 0–B, depois a C).
