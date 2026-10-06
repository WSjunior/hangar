# Parte 4: estado, prévia e terminal real no Rust — plano de implementação

> Execução só depois da aprovação do dono; quem abre é a coordenação (`migracao-rust-3`).
> Desenho: `desenho.md` (revisão adversarial incorporada). Inventário: `inventario.md`. Linhas
> conferidas em `0eb41ec58`; cada Task reconfere as dela na base antes de codar. Substitui a Fase
> C e a Task 24 de `../lista-estado/plano.md`.

**Objetivo:** com o Rust de pé, ele é o único dono do estado ao vivo, da prévia, da pergunta
nativa, da sugestão e do `problema` das sessões Claude com terminal (qualquer porta de entrada) e
de todo PTY do terminal real, inclusive no Windows (decisão do dono, 06/10, opção A). O Python
fornece fatos e serviços do que ainda é dele e faz a porta de entrada de quem chega pelas portas
dele.

## Restrições globais

- **Testes em cada Task:** escritos primeiro, vistos falhar sem o código da Task, rodados só os dos
  arquivos tocados (`cd backend && uv run pytest tests/<x>.py`;
  `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test <x>` ou `--lib <mod>::`).
  No máximo 2 `cargo` na máquina (`pgrep -c -x cargo`); cada worktree com o próprio `target/`,
  apagado ao terminar; `rust-analyzer` desligado na worktree.
- **Paridade por entradas gravadas:** o redutor, a prévia e a pergunta são testados por sequências
  que o gerador Python (`backend/tests/fixtures/contract/gen_terminal.py`, `gen_golden.py`) grava
  rodando o código atual com relógio injetado; o Rust repete as entradas e compara rodada a rodada.
  Nenhum dado real de conversa. Mudou a regra no Python durante a execução → regenerar no mesmo
  commit.
- **Contrato interno:** sobe nos dois lados (`backend/app/rust_server.py:35`,
  `crates/hangar-server/src/lib.rs:26`, teste fixo `tests/proxy.rs:30`) nas Tasks 1, 5, 8 e 9,
  cada uma no próximo número livre na hora da junção (hoje 27), nunca reaproveitando.
- **Nenhum `Monitor` em produção antes da Task 5.** Tasks 2–4 e 7 entram com o código exercitado
  só por testes. Nada de rodada em sombra no backend do dono.
- **Tasks 8 e 9 vão juntas ao canal de testes:** a 8 sozinha deixaria o PTY do dono no Rust e o do
  convidado/Connect no Python (dois donos do painel único).
- **Cada Task corrige, no mesmo commit, a regra de `CLAUDE.md` ou `docs/decisoes/` que ela torna
  falsa**; a Task 11 só cuida do que sobra.
- Harnesses: ler "Regras vigentes" de `docs/decisoes/harnesses.md` antes de estado, hooks, plugin,
  observação ou entrega. Windows: ler "Regras vigentes" de `docs/decisoes/windows.md` antes de
  subprocesso, processo, PTY ou teste que simula plataforma; job Windows do CI conferido pelo log,
  job por job.
- **Desempenho:** cada Task confere o código contra "Desempenho: erros que já custaram"
  (`docs/migracao-rust/README.md`). Tasks que rodam a cada tique ou por byte (5, 6, 8) medem antes
  e depois em release, backend isolado e sem outro cliente; resultado no `medicao.md` desta pasta.
- Nenhuma sessão real nem backend real. Backend de prova isolado pela classe `Prova`
  (`scripts/prova-dono-unico.py`: `HOME` temporário, portas fora de 8765/8766/8768, `tmux -L`
  próprio, `matar_orfaos` desligado); Claude só Haiku em
  `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`, Codex em `gpt-6-luna`; nunca `claude-200-1`
  nem `claude-200-3`.
- Log e diário só com código, motivo curto, nome da sessão e nome de campo; nunca texto de
  conversa, pergunta, prévia, statusline nem tecla digitada.
- Identificador novo em inglês; `git add` por caminho; commits em inglês; merge, nunca rebase nem
  `push --force`. Proibido: backend real, instaladores, workflow do CI, comentar em PR.

## Lotes — o que corre em paralelo

- **Lote 1:** Tasks 1 e 8 em paralelo (estado e terminal não se tocam).
- **Estado:** Task 2 depois da 1; Tasks 3, 4 e 7 em paralelo depois da 2; Task 5 depois da 3, 4 e
  7; Task 6 depois da 5.
- **Terminal:** Tasks 9 e 10 em paralelo depois da 8.
- **Por último:** Task 11 (documentação) depois de todas; Task 12 (prova isolada) depois da 6, 9 e
  10; Task 13 (dono e VM) por último.
- Junção na ordem dos números dentro de cada trilha; conflito de número de contrato resolve pela
  regra acima.

## Foco da revisão

- Com o modo `rust` ou `pending`, nenhum caminho sobe `StateMonitor` ou `PreviewBroker` de Claude
  com terminal (8765, 8766, 8768), nem abre PTY no Python.
- Erro de fato, serviço ou ponte vira `problema` com código, 503 ou fechamento com código; nunca
  estado inventado, evento calado ou passagem ao Python.
- Uma captura por rodada por sessão; um `Monitor` por sessão; um painel por sessão em todas as
  portas.
- Entrega continua passando pelo `adapter.drain` do Python (`prepare_session`); o Rust só avisa
  a borda.
- Nenhuma chamada ao Python por tique: fatos por empurrão, retrato com prazo.
- Divisão por plataforma (Windows sem `-C`) e por provedor (Pi/omp/Kimi/Codex no Python) intacta.

---

## Estado ao vivo

### Task 1: Fatos do estado por empurrão e serviços (contrato)

**Arquivos:** `backend/app/state_facts.py` (novo: retrato por sessão com idades em ms — estado
do plugin, `waiter_aberto`, idade da batida, pergunta segurada com idade do `visto`, sugestão,
`bodyColumns`, âncora da faixa, `em_troca`, operação de permissão controlada —; sequência por
sessão; interesse registrado pelo retrato e vencido sem renovação em 30 s; envio só para sessões
com interesse; falha de envio no diário uma vez por queda), `backend/app/plugin_bridge.py`
(envio nos pontos de `_acordar` `:1367, 1380, 1427, 1495`, início e fim de long-poll
`:1105-1106, 1139-1141`, `/ask` limitado a 1 por 25 s, `/suggest` `:1155`, `transcript_columns`,
`band_anchor`), marcação de `em_troca` (`runtime_policy.py:206-212`, `registry.py:2553, 2684`),
operação controlada (`permission_mode.py`), `backend/app/internal_api.py`
(`GET /internal/sessions/{name}/state-facts`), `backend/app/runtime_policy.py` (serviços
`permission.observe(sid, name, mode)` → `(mode, previous_non_plan)`, `session.dead(name)` →
`ok`/`em_troca` com `esquecer` + `forget_frame` só no `ok`, `session.deliverable(name)` →
`adapter.drain`), `backend/app/list_bridge.py` (op `state.facts` para o Rust),
`crates/hangar-server/src/state/facts.rs` (novo: recebe o empurrão, descarta sequência menor,
aplica as validades com o relógio do Rust — `vivo` 35 s, plugin 90 s sem prazo com long-poll
aberto, pergunta 35 s —, cliente do retrato e dos serviços), `list/bridge.rs` (op `state.facts`),
contrato nos dois lados, testes.

**Falha sem ela:** Python `test_state_facts_pushed_only_on_change_and_for_interest`,
`test_state_facts_carry_ages_not_monotonic_instants`, `test_longpoll_and_ask_push_rate_limited`,
`test_dead_service_refuses_during_transfer`, `test_permission_observe_returns_previous_non_plan`,
`test_deliverable_service_runs_prepare_session`; Rust `state_facts::validity_with_own_clock`,
`state_facts::older_sequence_dropped`, `state_facts::snapshot_failure_is_error`.

- [x] **Step 1: Ler "Regras vigentes" de `harnesses.md`; testes, vistos falhar**
- [x] **Step 2: Fatos, interesse, serviços e op `state.facts`; contrato no próximo número livre; testes focados; revisar**

### Task 2: `Monitor` de estado no Rust (sem produção)

**Arquivos:** `crates/hangar-server/src/state/monitor.rs` (novo: vida por assinante, época pelo
`rebind`, captura em processo no `TerminalPool`, rodada de estado a 0,75 s ou acordada pelo
fato, contagem de rodadas igual ao Python `state.py:1074-1078`, falha da observação repetindo o
último evento com `problema=terminal_observacao_falhou` e `has-session` só ao entrar em falha e na
nova tentativa do pool, `dead` pelo serviço, chave de repetição), `state/agent_pane.rs` (porte de
`agentpane.resolve_target`, cache 60 s, sobre `Pane::target` da lista), `state/shells.rs` (porte
de `procinfo.shells_de` sobre `list/procs.rs`; `sysinfo` fora do Linux), `state/permission.rs`
(porte de `parse_permission_mode` e a regra de quando chamar `permission.observe`),
`state/demote.rs` (mapa de rebaixados saído de `list/facts.rs`, compartilhado com a lista,
invalidado por `statusUpdatedAt`), `terminal_state.rs` (`reduce` vira produção: `retired`, log de
divergência do plugin, statusline, overlay/login/limite, loop), `gen_terminal.py` (sequências com
`dead`/`em_troca`, falha da observação, permissão com operação controlada, shells, loop, chave de
repetição, acordar pelo plugin, idades dos fatos), `tests/contract_terminal.rs`.

**Falha sem ela:** `contract_terminal::monitor_sequences` (spinner congelado, debounce, graça do
marcador, âncora e acordar do plugin, `em_troca`, `dead`, falha da observação, permissão,
shells, loop), `monitor::rounds_counted_like_python`,
`monitor::has_session_only_on_failure_entry_and_retry`, `demote::invalidated_by_file_version`,
`agent_pane::matches_python_fixture`, `permission::parse_matches_python`; medida em release de uma
rodada (captura + `reduce`) com tmux `-L`, contra 1,26 ms do Python por captura.

- [x] **Step 3: Testes e sequências novas, vistos falhar**
- [x] **Step 4: `Monitor`, pane do agente, shells, permissão, rebaixamento; testes focados; medida de uma rodada; revisar**

### Task 3: Prévia no `Monitor` (sem produção)

**Arquivos:** `state/preview.rs` (novo: arquivo do hook `.hangar-preview/<stem>.json` por mtime
na rodada, regra dos 600 s com marcador `working` `preview.py:448-458`; pane do mesmo quadro
cortado em `bodyColumns+5` e na âncora da faixa **antes** da análise; capturas de 0,15 s só
enquanto `working` sem arquivo e só para a prévia; não publica o que já está no transcript e limpa
quando o bloco entra nele, porte de `sse.py:54-72, 852-855`; vaga única; `text, md, full, vivo`;
`LiveBuffer` só na fonte arquivo), `side.rs` (gancho do `FileTail` para a supressão),
`backend/tests/fixtures/contract/gen_golden.py` (casos de prévia), testes.

**Falha sem ela:** `preview::hook_file_first_then_pane`,
`preview::old_file_valid_while_marker_working`, `preview::crop_before_analysis`,
`preview::suppressed_when_in_transcript`, `preview::cleared_on_commit`,
`preview::fast_captures_only_for_pane_preview`.

- [x] **Step 5: Testes e golden, vistos falhar**
- [x] **Step 6: Prévia; testes focados; revisar**

### Task 4: Pergunta nativa, sugestão, `problema` e entrega no `Monitor` (sem produção)

**Arquivos:** `state/ask.rs` (novo: `.hangar-askq` por `facts_files.rs`, casamento com o menu do
pane, porte de `sse.py:75-184`; uma vez por pergunta; zera no `/clear` e na troca de provider),
`state/monitor.rs` (`suggest` do fato quando muda; `problema` do runtime lido do `view` do ator
com o mapa `_STALLED_INPUT` portado, na chave de repetição, eventos do ator acordam; borda "voltou
a aceitar texto" → `session.deliverable` uma vez por borda e uma ao nascer), códigos novos de
`problema` (`state_facts_unavailable`, `permission_observe_failed`) em
`frontend/src/lib/problema.ts`, `mobile/src/features/chat/SessionProblem.tsx`, i18n do nativo e
`messages/pt.json`/`en.json`, `gen_golden.py`, testes.

**Falha sem ela:** `ask::matches_menu_python_golden`, `monitor::ask_question_once_per_prompt`,
`monitor::deliverable_edge_calls_service_once`, `monitor::runtime_problem_in_key_and_wakes`,
`monitor::suggest_on_change`; vitest dos três mapas de `problema` com os códigos novos.

- [x] **Step 7: Testes e golden, vistos falhar**
- [x] **Step 8: Porte; códigos nos três clientes; testes focados; revisar**

### Task 5: Troca de dono do estado (contrato)

**Arquivos:** `side.rs` (cria o `Monitor` com o primeiro assinante, fonte própria de `state`,
`preview`, `ask_question`, `suggest` saindo pelo `SideCache::record` e a regra de repetido,
publica depois do `rebind`; descarta esses quatro vindos do Python para sessão do Rust e registra
uma vez por sessão), `routes.rs` (canal privado `GET /__hangar_server/state/{name}/events`,
segredo e loopback, conta como assinante), `backend/app/sse.py` (modo `rust`: Claude com terminal
não sobe `StateMonitor` nem `PreviewBroker` em nenhuma porta; `merged_events` não-interno lê o
canal privado; modo interno mantém `info`, `message`/`queue_confirmed`, `nav`, `ping`,
`plugin_toast`, `plugin_ui`, `stats`, `pensamento`/`ferramenta`, desliga o `tail_pump`, não emite
`suggest`/`ask_question` nem dispara `drain` para essas sessões), `scripts/medir-estado.py` (novo,
sobre a `Prova`: panes falsos com tela de Claude parada e com spinner, N chats abertos), contrato
nos dois lados, `docs/decisoes/harnesses.md` ("Observação terminal tem uma captura canônica por
rodada"), `CLAUDE.md` ("Observação terminal Rust"), `docs/decisoes/plataforma.md`,
`docs/decisoes/superado.md`, `medicao.md` (desta pasta).

**Falha sem ela:** Python `test_no_python_state_monitor_for_rust_terminal` (interno, convidado e
Connect), `test_guest_chat_reads_rust_channel`, `test_side_events_keeps_info_queue_nav_ping_toast`;
Rust `side::monitor_publishes_after_rebind`, `side::python_state_for_rust_session_dropped_once`,
`side::private_channel_counts_as_subscriber`, `monitor::one_monitor_per_session`.

- [x] **Step 9: Testes, vistos falhar; medida "antes" (`medir-estado.py`, release, 1/5/20 chats, parado e trabalhando: CPU do Python e do Rust, latência marcador → `state`, RSS)**
- [x] **Step 10: Hub com fonte própria, canal privado, Python calado para essas sessões; contrato; regras corrigidas; testes focados**
- [x] **Step 11: Medida "depois" no mesmo método; `medicao.md`; revisar**

### Task 6: Lista lê o `Monitor` e o cartão de permissão do Claude com terminal

**Arquivos:** `list/classify.rs` (com `Monitor` vivo, lê o estado dele pelo mapa compartilhado em
vez de capturar), `state/monitor.rs` e `list/classify.rs` (conserto do cartão), sequência gravada
do caso real em `gen_terminal.py`, `docs/decisoes/harnesses.md` (regra do cartão, se a causa
virar regra), `medicao.md`.

**Falha sem ela:** `classify::uses_monitor_state_when_alive` (nenhuma captura registrada),
`classify::permission_card_after_bash_is_awaiting`,
`contract_terminal::permission_card_after_bash` (lista e `Monitor` dizem `awaiting_input`).

- [ ] **Step 12: Reproduzir com sessão real isolada (Haiku, modo padrão, `Bash` que pede permissão): lista `working` com cartão na tela; provar a causa (superpowers:systematic-debugging) e gravar a sequência**
- [ ] **Step 13: Testes, vistos falhar; conserto e leitura do `Monitor` pela lista; testes focados**
- [ ] **Step 14: Medida antes/depois do tique da lista com 5 chats abertos (capturas evitadas, CPU); `medicao.md`; revisar**

### Task 7: Windows — captura avulsa pelo psmux

**Arquivos:** `state/capture.rs` (novo: fonte de captura do `Monitor`, `-C` no Linux/macOS e
subprocesso psmux no Windows, `terminal_control.rs:173` segue desligado; lê a saída, não só o
código de retorno; "não existe" × "falhou" por `has-session`; quadro com U+FFFD não vale como
bom), `list/classify.rs:375-417` (mesma fonte; hoje `rc != 0` vira `capture_refused` e o
`from_utf8_lossy` não marca), testes `cfg(windows)` com programa falso,
`docs/decisoes/windows.md`.

**Falha sem ela:** `capture::psmux_reads_output_not_exit_code`,
`capture::replacement_char_frame_not_good`, `capture::missing_vs_failed_by_has_session`,
`monitor::windows_uses_subprocess`.

- [x] **Step 15: Ler "Regras vigentes" de `windows.md`; testes, vistos falhar**
- [x] **Step 16: Fonte por subprocesso no `Monitor` e na lista; job Windows do CI pelo log; revisar**

---

## Terminal real

### Task 8: PTY no Rust para o dono (contrato)

**Arquivos:** `crates/Cargo.toml` (feature `ws` do axum; `portable-pty` 0.9),
`crates/hangar-server/src/term/mod.rs` (novo: `WS /api/sessions/{name}/term` com `?shortcut=`,
`WS /api/hangar-terminals/{ident}/term`; só `?token=`, IP bloqueado e falha pelo `auth.rs`;
recusa antes do upgrade com os códigos de hoje), `term/origin.rs` (pergunta
`POST /internal/term/origin` com Origin e `Host` original, prazo 1 s, falha recusa com código),
`term/resolve.rs` (`has-session`; porte de `shortcut_terminals.find`/`find_hangar` por um
`list-sessions -F`; mux fora → 1013), `term/pty.rs` (`tmux attach -t ={name}:` com
`TERM=xterm-256color`, `COLORTERM=truecolor`, `CLAUDE_CODE_TMUX_TRUECOLOR=1`, sem
`NOTIFY_SOCKET/INVOCATION_ID/LISTEN_*`/`PSMUX_SESSION`; leitor numa thread, blocos de 64 KiB,
canal de 1 MiB; escrita; resize com clamp 20..500 / 5..200; ping 20 s / 20 s; um painel por
sessão com espera do escritor antigo e 1000 "outra conexao assumiu"; desmontagem com
`detach-client -t <tty>`, SIGHUP, 3 s, SIGKILL, espera do `list-clients`, tamanho reposto;
`@hangar_term_size` ao anexar e reposição ao subir o Rust; teto de 64 painéis com as threads),
`routes.rs`, `backend/app/internal_api.py` (`/internal/term/origin` chamando `_origem_aceita`),
`scripts/medir-terminal.py` (novo: MB/s com `cat` de 50 MB no pane, eco de tecla, CPU por MB, RSS
por painel, Python × Rust), contrato nos dois lados, `medicao.md`.

**Falha sem ela:** Rust (tmux `-L`) `term::echo_roundtrip`, `term::resize_clamped`,
`term::second_connection_takes_over`, `term::teardown_detaches_own_client_and_restores_size`,
`term::backpressure_pauses_reader`, `term::ping_timeout_closes_and_tears_down`,
`term::size_restored_after_crash`, `term::query_token_only`,
`term::origin_refused_before_upgrade`, `term::shortcut_resolves_owner`,
`term::panel_cap_refuses_1013`; Python `test_internal_term_origin_same_rules`.

- [x] **Step 17: Ler "Regras vigentes" de `windows.md` e as entradas do terminal em `frontend.md`; testes, vistos falhar; medida "antes" (`medir-terminal.py`, release)**
- [x] **Step 18: Rotas, porta de entrada, PTY, desmontagem, reposição; contrato; testes focados**
- [x] **Step 19: Medida "depois"; `medicao.md`; revisar**

### Task 9: Python como porteiro, 409 e capacidade (contrato)

**Arquivos:** `backend/app/termsock.py` (modo `rust`: depois da porta de entrada de hoje —
convidado de convite, convidado com login, dono pelo Connect —, liga os bytes a
`/__hangar_server/term` na porta privada; `share_gate.watch`/4410 fecha os dois lados; nunca abre
PTY), `crates/hangar-server/src/term/mod.rs` (rota privada com segredo e loopback, painel único
compartilhado com a 8765), `list/bridge.rs` (op `term.active`), `backend/app/api.py`
(`_recusa_se_painel_aberto` `:5782-5794` pergunta ao Rust no modo `rust`, 503 com código no erro;
`/api/config.terminal_panel` e `/run-code` `:7767` leem a saúde), saúde do `hangar-server`
(`terminal_panel`), `backend/app/rust_server.py` (lê o campo), contrato nos dois lados,
`CLAUDE.md` ("Terminal real no rodapé e no celular"), `docs/decisoes/frontend.md` (terminal real:
motores e capacidade), `docs/decisoes/superado.md`.

**Falha sem ela:** Python `test_guest_terminal_pipes_to_rust_no_pty`,
`test_connect_owner_terminal_pipes_to_rust`, `test_revoked_guest_closes_upstream_4410`,
`test_409_asks_rust_and_503_on_bridge_error`, `test_terminal_panel_from_rust_health`; Rust
`term::private_route_requires_secret`, `term::one_panel_across_owner_and_guest`,
`bridge::term_active`.

- [x] **Step 20: Testes, vistos falhar**
- [x] **Step 21: Porteiro, 409, capacidade; contrato; regras corrigidas; testes focados; revisar**

### Task 10: Windows — ConPTY

**Arquivos:** `term/pty.rs` (Windows pelo `portable-pty`; matar o filho antes de soltar o
pseudoconsole; sem reposição de tamanho no psmux, como o Python), `term/conpty.rs` (só se a VM
reprovar: porte de `conpty.py` em `windows-sys`, flags 0, `STARTF_USESTDHANDLES`, pipe de entrada
duplex), testes `cfg(windows)`, `docs/decisoes/windows.md`.

**Falha sem ela:** `conpty::spawn_echo_and_resize` (`cmd.exe` no runner Windows),
`conpty::child_killed_before_close`.

- [ ] **Step 22: Ler "Regras vigentes" de `windows.md`; testes, vistos falhar; job Windows do CI pelo log**
- [ ] **Step 23: VM DELPHI-02 com web e nativo: abre, digita, resize, Claude Code em tela cheia, nenhum travamento no pedido de posição do cursor (`INHERIT_CURSOR`); reprovou → `conpty.rs` com flags 0 e repetir (verificação manual)**
- [x] **Step 24: Regra em `windows.md`; revisar**

---

## Fechamento

### Task 11: Documentação restante

**Arquivos:** `CLAUDE.md`, `docs/migracao-rust/README.md` (parte 4 e "Ainda no Python"),
`docs/decisoes/superado.md`, `inventario.md`, `../lista-estado/plano.md` (Fase C e Task 24
apontando para esta pasta).

- [ ] **Step 25: Varrer `CLAUDE.md` e `docs/decisoes/` por regra que ficou falsa; corrigir; revisar**

### Task 12: Prova de uso real isolada

**Arquivos:** `scripts/prova-parte4.py` (sobre a `Prova`), `prova-real.md` (desta pasta).

- [ ] **Step 26: Claude Haiku com terminal: trabalhando/parada/esperando, pergunta nativa, cartão de permissão, prévia (arquivo e pane), sugestão, `/clear`, sessão morta, troca de conta (`em_troca`), convidado olhando o chat; nenhum `StateMonitor`/`PreviewBroker` Python (contador) e uma captura por rodada**
- [ ] **Step 27: Codex `gpt-6-luna` com e sem terminal fora do YOLO: cartão de aprovação no sem terminal, menu na TUI com terminal, lista em `awaiting_input` nos dois; Claude sem terminal com cartão**
- [ ] **Step 28: Terminal: dono, convidado e Connect; atalho e `term-<nome>`; 409 com painel aberto; duas conexões; queda de rede (ping); queda do Rust com painel aberto (tamanho reposto)**
- [ ] **Step 29: Isolado com 1, 10 e 20 sessões (método de `../lista-estado/medicao.md`): CPU de Python e Rust em repouso e ativo; reserva `CP_RUST_SERVER=0` e Rust derrubado 3 vezes, o estado e o terminal voltam pelo Python igual**

### Task 13: Uso real com o dono e VM Windows

**Arquivos:** `prova-real.md`.

- [ ] **Step 30: Celular/PWA, app Expo e nativo, dois servidores: estado, prévia, pergunta nativa, permissão segurada no app, terminal no rodapé e no celular, convidado (verificação manual)**
- [ ] **Step 31: VM DELPHI-02: estado do chat, lista, psmux lento, terminal real (verificação manual)**
