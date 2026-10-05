# Lista de sessões + estado: inventário (05/10/2026)

Tudo que monta a lista e o estado hoje, conferido em `920473435`. Serve de base quando a lista for
ao Rust (ver `medicao.md` para o porquê de não ser agora). Caminhos relativos à raiz do repositório.

## 1. Rotas e o produtor

| O quê | Onde | Notas |
|---|---|---|
| `GET /api/sessions` | `backend/app/api.py:1824-1868` | serve `sse.recent_list(2.0)` (retrato decorado do produtor, `:1851`); sem ele, `_guardar_snap` + `registry.list_with_state` em cópias (`:1860-1866`); filtros de convidado `guest.sees`, `guest_users.filter_visible`, `guest_safe` (`:1854-1865`) |
| Retrato cru com TTL e um por vez | `api.py:999` (`_LIST_TTL = 1.0`), `:1011` (`_list_lock`), `:1014-1031` (`_guardar_snap`), `:1034-1039` (`_invalidate_lists`), `:1042-1057` (`_cached_info`) | invalidado na criação, rename e troca de modo (`:2400, 3228, 3469, 8273, 10039`) |
| `GET /api/sessions/events` | `api.py:3953-3960` → `sse.list_events` (`backend/app/sse.py:578-671`) | eventos `sessions`, `list_error`, `shortcut_terminals` (só dono), `nav` (só dono: `nav_pump` a cada 1 s sobre o mapa em memória, `sse.py:644-660, 266`; o mapa vem de `~/.hangar/nav/_pendentes.json`, lido uma vez, `:200-278`), `ping` a cada 8 s; `plugin_bridge.app_entrou/app_saiu` (`:593`, `:669`) |
| Produtor único da lista | `sse.py:409-555` (`_ListRefresher`), instância `:558` | poll 1,5 s depois do trabalho (`:418`); contagem de clientes, para com o último (`:542-555`); falha mantém o retrato e emite `list_error` uma vez (`:494-520`); `latest` a cada tique (`:524`) |
| Assinatura da lista | `sse.py:343-401` (`_list_sig`), `:294-340` (`_status_sig`, `_context_sig`) | ignora `last_activity`; statusline e contexto em baldes de 5% |
| Segundo leitor: vigia de travada | `backend/app/stall_watch.py:108-114`, laço `:166-174` (30 s, `config.py:228`) | roda `list_with_state` próprio, mesmo sem cliente; avisa caída por diferença de nomes (`:124-143`), `stalled`, `limited`, auto-resume |
| Outros chamadores de `list_with_state` | `api.py:2591` (`_motivo_ocupada`), `api.py:6387` (`_auto_update_loop`) | — |
| Roteamento no Rust | `crates/hangar-server/src/routes.rs:194-208` | as duas rotas caem no `.fallback(pass_any)` → `proxy::forward` (`routes.rs:227-267`): **nenhuma é atendida pelo Rust** |

## 2. Descoberta: `SessionRegistry.list` (`backend/app/registry.py:1299-1497`)

Só resolve; o estado fica no padrão `idle`.

1. Mapa de processos de `/proc` com TTL de 3 s (`:1304`, `procinfo.py:60, 76-116`); psutil fora do
   Linux (`procinfo.py:40-45, 424-434`).
2. Um `tmux list-panes -a` com 8 campos (`:1308`, `tmux.py:391-432`).
3. Por sessão tmux: pane do agente (`:1265-1297`), provider pelo argv dos descendentes
   (`:517-569`) ou `CP_PROVIDER`; transcript por provider — Pi/omp por bilhete
   (`:759-832`), Kimi por bilhete (`:846-888`), Claude por `resolve_tracked` (`:1064-1190`: fd
   aberto, marcador do hook, `--session-id`, pid, cache, mais novo). `_cwd_has_siblings` faz outro
   `tmux list-panes -a` por sessão nesse ramo (`:1044-1048`).
4. Grupo e encadeamento (`.hangar-pair`, `.hangar-chain`, `:1376-1377`), worktree e branch sem
   subprocesso (`worktrees.locate`, `:1379`), vida da sessão (`:1381`), motor e conta pelo
   `environ` (`:1394-1431`).
5. Linhas de sidecar: Codex (`:1438-1455`), Claude sem terminal (`:1460-1480`), orq (`:1483-1491`,
   `adapters/orq/runs.py:75-93`), transferências (`:1492`, `:165-206`).
6. **Efeito colateral:** varredura de pares mortos (`:1494`, `:2964-3000`) — desfaz par ausente há
   5 s, revoga compartilhamento e manda `DELETE /api/pair` ao par externo.

Caches de classe `registry.py:946-990`; `_STATUS_TTL = 20`, `_STATUS_BUDGET = 2` (`:921-922`).

## 3. Decoração: `list_with_state` (`registry.py:1524-1979`)

| Campo(s) | Linhas | Fonte | Cache |
|---|---|---|---|
| orq `state`, `last_activity` | 1536-1542 | mtime da linha do tempo e `advance.lock`, `/proc/locks` (`orq/runs.py:103-128`) | — |
| Kimi: correção de ocioso, aprovação | 1572-1604, 1681-1690 | fim do `wire.jsonl` (`state.py:589-706`); `capture-pane` só com pedido pendente | — |
| Codex `state`, `label`, perguntas, aprovação, `problema` | 1612-1656 | app-server em memória (`adapters/codex/adapter.py:1574-1698`), marcador, `codex_turno_aberto` | — |
| Codex sem thread: etapas, menu | 1614-1630, 1764-1780 | `capture-pane` a cada tique | — |
| Claude sem terminal: estado, pergunta, statusline, `problema` | 1657-1675 | `ClaudeHeadlessAdapter.snapshot` (`public_state` calculado no Rust e lido em `runtime_adapter.py:757-767`) | — |
| Claude com terminal: `problema` do runtime | 1677-1680 | `runtime_adapter.runtime_problem` (`:605-619`) | memória |
| `state` pelo marcador (caminho rápido) | 1691-1725 | `hook_state.get_state` (`hook_state.py:56-62`): registro nativo `<config>/sessions/<pid>.json` primeiro, depois `.hangar-state/<sid>.json` | mapa mantido por `watchfiles` (`hook_state.py:212-234`) |
| `state`, `label`, pergunta, limite (sessões sem marcador, aguardando ou idle velho) | 1726-1763 | `capture-pane -S -200` + `classify`; segunda captura após 0,15 s para confirmar spinner | grava `_status_cache`, `_label_cache`, `_limit_cache` |
| rebaixar marcador `awaiting` | 1751-1755 | escreve `.hangar-state/<sid>.json` (`hook_state.py:163-189`) | — |
| pergunta aberta fora da tela | 1786-1805 | `.hangar-askq/<stem>.json` (`askquestion.py:112+`) | — |
| statusline (varredura) | 1811-1847 | `capture-pane` | TTL 20 s, no máximo 2 por tique |
| statusline Codex / sidecar | 1856-1878 | fim do rollout; `.hangar-status/<stem>.json` (`statusline.py:54-75`) | 20 s |
| `context`, `model` | 1886-1908 | últimos 512 KB do transcript (`claude_context.py:36-72`) | 20 s |
| `stalled` | 1911-1917 | `last_activity` × `stall_seconds` | — |
| `limited`, `limit_reset` | 1918, 1499-1522 | `capture-pane` | 30 s |
| `last_reply`, `last_reply_at` (só ociosas) | 1921-1944 | `merged_history(limit=8)` (`pqueue.py:1048`) | por (transcript, mtime) |
| `git_*` | 1955-1967 | último valor por pasta; atualização em segundo plano um por pasta (`:85-95`), que chama o `hangar-workspace` no Rust (`git_ops.py:1325-1327`, `crates/hangar-workspace/src/git.rs:175-249`) | 3 s / 10 s |
| `plan_*` | 1969-1975 | `planprog.plan_progress` (`planprog.py:89-93, 499+`) | mtime + 3 s |
| `loop_*` | 1976-1977 | `.hangar-loop/<nome>.json` (`loop.py:39-63`) | — |
| `shared`, `owner` | 1543-1551 | `share_store.active_sessions`; `guest_users.owner_name` pode lançar `tmux display-message` (`share_life.py:36-45`) | 2 s |

Campos da linha: `backend/app/models.py:73-210` (`SessionInfo`).

## 4. Estado por sessão (chat aberto) — fora da lista

- `classify` (`backend/app/state.py:388-423`) é função pura do texto do pane; nunca devolve `dead`.
- `StateMonitor` (`state.py:794-1080`): poll 0,75 s; quadros de `shared_capture` (`:760-783`),
  que pede a captura ao Rust (`terminal_observer.py:171`) e usa o tmux do Python só no Windows ou
  com nome fora de `[A-Za-z0-9._-]{1,64}`; `dead` por pane vazio + `sessao_existe`; âncora do
  plugin (`plugin_bridge.estado_recente`, `:996-1011`) e do marcador (`:1013-1026`); a lista não usa
  o plugin.
- Compartilhado por chave `(nome, provider, transcript)` no `Difusor` (`backend/app/difusor.py:34-79`,
  `sse.py:206, 719-728`).
- No Rust: `terminal_state::analyze` (`crates/hangar-server/src/terminal_state.rs:329-351`) roda em
  toda captura, mas só `preview.py:682-688` usa o resultado; `reduce` (`:366-429`) é referência,
  conferida contra o Python em `crates/hangar-server/tests/contract_terminal.rs`.
- Sem terminal: o estado público sai do Rust (`runtime/claude.rs:137-179`, `runtime/codex.rs:187-200`)
  pelo canal `/runtime/events` (`runtime/actor.rs:839-842`) e volta ao cliente pelo `side-events`
  do Python (`crates/hangar-server/src/side.rs:401-446`).

## 5. Hooks, plugin e marcadores

- `backend/hooks/state_hook.py` grava `.hangar-state/<sid>.json` e `.hangar-active/<boot>.json` em
  `CLAUDE_CONFIG_DIR` (`:123-147`); instalado por `hook_installer.py:187-239`. Codex reaproveita o
  mesmo script (`codex_hook_installer.py:31-90`); Kimi tem o seu (`kimi_state_hook.py:37-100`);
  Pi/omp pela extensão `scripts/pi/hangar-state.ts` (`:17-18, 84, 488-545`).
- Plugin: `POST /api/plugin/state` (`plugin_bridge.py:1469-1485`), validade 90 s (`:748, 832-842`);
  consumido só pelo `StateMonitor` e `merged_events`.

## 6. O que o Rust já tem e serve à lista

| Peça | Onde | Serve para |
|---|---|---|
| Git por pasta com cache e trava por pasta | `crates/hangar-workspace/src/git.rs:137-249` | `git_*` da lista (já usado via ponte) |
| Leitura de transcript Claude/Codex, `InternalInfo` | `crates/hangar-server/src/transcript/history.rs:32-43`, `transcript/mod.rs:24-48` | `last_reply`, contexto |
| `tmux -C` por sessão com aluguel de 90 s | `crates/hangar-server/src/terminal_control.rs:125-429` | captura sem processo novo |
| `analyze`/`reduce` do pane | `crates/hangar-server/src/terminal_state.rs` | classificação |
| Estado público sem terminal | `runtime/claude.rs:137-179`, `runtime/codex.rs:187-200` | linhas sem terminal |
| `/proc` + `list-panes` (só para nome de remetente) | `crates/hangar-server/src/transcript/peer.rs:43-132` (só Linux) | modelo de descoberta |
| Exemplo de lista movida: worktrees | `crates/hangar-server/src/worktree_routes.rs:15-159` | contexto pedido ao Python por `/internal/worktrees/context` (`internal_api.py:174-188`), 503 com código, sem passar ao Python |
| Tipos compartilhados | `crates/hangar-api/src/state.rs:6-43` (`ShellVivo`, `StateEvent`) | **não há** tipo de linha da lista (`SessionInfo`) |

Contrato interno atual: **22** (`backend/app/rust_server.py:35`, `crates/hangar-server/src/lib.rs:25`).

## 7. Clientes da lista

- Web: `frontend/src/lib/sessionsStore.svelte.ts:161-342` (um stream por servidor, vigia de 25 s),
  `sessionListModel.svelte.ts:108-193`; `GET /api/sessions` direto em `Chat.svelte:854, 2435`,
  `CreateSessionSheet.svelte:716`, `ForwardSheet.svelte:33`, `PairSheet.svelte:114`,
  `ShortcutsSettings.svelte:311`, `newChatDraft.svelte.ts:324`, `term.ts:18`, `Login.svelte:91`.
- Núcleo: `packages/core/src/api.ts:405-422, 2780-2791`, `sessions.ts:35-123`,
  `types.ts:33-129` (`SessionInfo`), `statusline.ts:60-68`.
- App: `mobile/src/stores/sessions.ts:129-172`.
- Nativo: `desktop-native/src/api/mod.rs:174-177, 592-600`, `app.rs:1440-1461`,
  `app/servers.rs:136-148`, `api/dto.rs:17-126` (lê `context`, `pair_external`, `guest_kind`,
  `lifecycle_id`, `transfer_*`, que o TS não declara).

## 8. Windows

- `procinfo` troca `/proc` por psutil (`procinfo.py:40-45`); `_open_jsonl` é `None` fora do Linux
  (`:134-143`), então a resolução por fd aberto nunca acontece lá.
- Bilhetes Pi/Kimi por `PSMUX_SESSION` (`registry.py:739-756`); `.exe` tirado do argv (`:522-523`);
  `MuxIndisponivel` nasce de timeout do psmux (`tmux.py:132-145, 210-215`).
- Observação do terminal no Rust desligada no Windows (`terminal_control.rs:173`,
  `terminal_observer.py:115-116`): lá o chat captura pelo Python.
- `orq_runs._held` sem `/proc/locks` usa só mtime (`runs.py:103-112`).
- O Supervisor usa Job do Windows em vez de varrer o grupo (`runtime_process.py:121-131, 165-186`),
  mas grava o registro de contenção com `fsync` a cada 0,25 s igual (`:248-282`).
