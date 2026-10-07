# Parte 4: inventário (06/10/2026)

Tudo que o Python faz hoje no caminho do estado ao vivo, da prévia, da pergunta nativa e do
terminal real das sessões com terminal, quem consome e o que o Rust já tem. Linhas conferidas em
`0eb41ec58` (`origin/hangar-server-parte1`); cada Task reconfere as dela antes de codar.
Desenho: `desenho.md`. Ponto de partida anterior: Fase C de `../lista-estado/plano.md` (Tasks
19–24), cujas linhas ficaram velhas (tabela no fim).

## O que virou Rust (Task 11, 06/10/2026)

As tabelas abaixo são o retrato de partida. Depois das Tasks 1–10, com o Rust de pé (`rust` ou
`pending`), cada peça mudou de dono assim; no modo `python` tudo segue como nas tabelas.

| Seção | Agora no Rust | Continua no Python |
|---|---|---|
| 1 e 2. Estado ao vivo | `Monitor` por hub (`state/monitor.rs`, `state/live.rs`, `side.rs`): captura (`PoolCapture`, `-C`), `reduce`, morta com `em_troca`, permissão (`state/permission.rs`), shells, pane do agente, rebaixamento (`state/demote.rs`), dedupe; canal privado `/__hangar_server/state/{name}/events` para 8766/8768 | fatos por empurrão (`state_facts.py`: plugin, `em_troca`, operação de permissão) e os serviços `permission.observe` e `session.dead`; a ponte `terminal_observer` ficou sem consumidor |
| 3. Prévia | `state/preview.rs`: arquivo do hook, pane, corte, captura rápida, supressão pelo transcript do hub | — |
| 4. Pergunta, sugestão, `problema`, entrega | `state/ask.rs`, `state/edges.rs`: `ask_question` uma vez, `suggest` pelo fato, `problema` do runtime e da observação, borda de entrega | a entrega em si (`session.deliverable` → `adapter.drain`) |
| Lista (Task 6) | lê o último `state` do `Monitor` vivo (`state/published.rs`) e não captura essa sessão; cartão segurado pelo `held` dos fatos | — |
| 6. Outros provedores | — | Codex (app-server), Pi, omp e Kimi (`StateMonitor`/`PreviewBroker`): parte 5 |
| 7. Terminal real | todo PTY (`term/`: `pty.rs`, `resolve.rs`, um painel por sessão em todas as portas, contrapressão, desmontagem e tamanho reposto pela opção `@hangar_term_size`); capacidade na saúde (`terminal_panel`) | porta de entrada da 8766/8768 (`termsock`, ligada a `/__hangar_server/term`), a Origin (`/internal/term/origin`, `term/origin.rs`) e o 409, que pergunta `term.active` |
| 8. Windows | `Monitor` com captura avulsa pelo psmux (`state/capture.rs`, decide pela saída); ConPTY do `portable-pty` com entrada segurada até o psmux pintar | — |

Medidas em `medicao.md`, prova em `prova-real.md`, achados sem conserto em `achados-pendentes.md`.
Contrato interno: 35.

## 1. Como o `state` chega ao cliente hoje (Claude com terminal)

1. O cliente abre `/api/sessions/{name}/events` no Rust (`routes.rs:372-409`, hub em `side.rs`).
2. O hub assina `GET /internal/sessions/{name}/side-events?app=1` (`side.rs:345-351`), que no
   Python é `internal_api.side_events` (`internal_api.py:273-285`) → `merged_events(side=True)`.
3. Lá, o `StateMonitor` (`state.py:794-1080`) alimenta `pump("state")`; o laço
   (`sse.py:1199-1259`) emite `state`, `suggest`, `ask_question` e dispara o `drain`.
4. O hub guarda o último valor de `state, suggest, ask_question, stats, preview, pensamento,
   ferramenta, plugin_ui` (`side.rs:70`) e repassa (`side.rs:428-441`).

O Rust já lê o transcript (`FileTail`), a fila e os avisos de mod; **não tem fonte própria de
`state`, `preview`, `ask_question` nem `suggest`**.

## 2. Estado ao vivo — `StateMonitor` (`backend/app/state.py`)

Criado por `adapters/claude.py:25-26`; sobe em `sse.py:735-746` (`_monitor_de`) e `:1054`
(`state_task`); refeito no `__reprovider__` (`:1125`) e no `/clear` (`:1164-1167`); um por
`(name, prov, jsonl)` entre conexões pelo `_ESTADOS = Difusor()` (`sse.py:207`).

| Peça | Python | Rust hoje |
|---|---|---|
| Intervalo 0,75 s | `state.py:809`, `:1080` | nada (lista: tique 1,5 s) |
| Aluguel da observação | `:841-856` → `terminal_observer.lease` (HTTP privado, 0,25 s, batida 20 s) | `TerminalPool`, `terminal_control.rs:221-250` |
| Quadro compartilhado com a prévia | `shared_capture` `:760-783`, `_capture_and_store` `:730-757`, `FRAME_MAX_AGE=0.5` `:797` | nada |
| `/clear` aposenta o quadro | `retired` `:872-875`; carimbo `:879, 885, 1058` | época/geração no `terminal_observer` |
| Falha da observação | `:901-917`: repete o último evento com `problema=terminal_observacao_falhou` | nada |
| Morta, respeitando `em_troca` | `:888-900`: `has-session`; `em_troca` espera; senão `plugin_bridge.esquecer` + `forget_frame` + `dead` | nada |
| Classificação | `classify` `:388-427`, refeita em Python sobre o texto que o Rust devolveu | `terminal_state::analyze` `:336` (resultado descartado pelo monitor) |
| Pergunta fora da tela (`.hangar-askq`) | `:944-949`, `askquestion.pergunta_aberta` `:112` | leitor em `list/facts_files.rs:233` |
| Pergunta/permissão segurada pelo plugin | `:953-965` | só na referência `reduce` `:383-396` |
| Spinner congelado (`STALE_LIMIT=3`) | `:800`, `:972-978` | `reduce` `:400-406` (referência) |
| Debounce do idle (`IDLE_DEBOUNCE=4`) | `:804`, `:979-986` | `reduce` `:407-413` (referência) |
| Âncora do plugin (+ log de divergência) | `:996-1011` | `reduce` `:415-424`, sem o log |
| Âncora do marcador, graça 8 s | `:807`, `:1013-1026`, `_marcador` `:834-839` | `reduce` `:425-431` |
| Statusline | `:1030` (sidecar `statusline.read` `:54`, senão pane `:77`) | `facts_files.rs:538` |
| overlay / login / limite | `:1037-1042` | `analyze` |
| loop | `:1045-1049` | só lista, `list/links.rs:181` |
| shells (`procinfo.shells_de`) | `:1053-1054`, `hook_state.shells` `:64` | nada |
| Modo de permissão | `:918-924`: `parse_permission_mode` (`permission_mode.py:199`) + `observar_ou_confirmado` (`:90-104`); memória lida em `registry.py:2705, 2747`, `api.py:4145, 9268, 9333, 9349` | nada |
| Chave de dedupe e emissão | `:1055-1071` | nada |
| Acordar pelo plugin (idade 0) | `:1074-1078`, `plugin_bridge.esperar_evento` `:827`, `_acordar` `:837` | nada |

`reduce`/`reduce_with_diagnostics` (`terminal_state.rs:373-436`) é só referência
(`:1-3`), usada apenas por `tests/contract_terminal.rs`; as sequências de `gen_terminal.py`
(`:141-158`) não cobrem `dead`, `em_troca`, falha da observação, permissão, shells, loop, dedupe
nem o acordar pelo plugin.

**O `TerminalPool` não recebe a saída do pane** (`-f ignore-size,no-output`,
`terminal_control.rs:300`): cada captura é um `capture-pane` pedido pelo cliente `-C`, sem
subprocesso. Não há sinal de "a tela mudou"; o ritmo é por sondagem.

### Fatos do Python que o estado lê

| Fato | Onde (linha atual) | Notas |
|---|---|---|
| `estado_recente` | `plugin_bridge.py:843`, validade 90 s `:748` | também em `runtime_terminal.facts:313` |
| `pergunta_pendente` | `:1284` | também em `runtime_terminal.facts:319` |
| `vivo` | `:818` | |
| `sugestao` | `:511`, gravada em `/suggest` `:1155` (não acorda o monitor) | |
| `esquecer` | `:387` | chamado pelo monitor ao ver a sessão morta |
| Largura da prévia (`bodyColumns+5`) | `transcript_columns` `:591-595` | só com painel `dock` |
| Âncora da faixa | `band_anchor` `:723-746` | |
| Presença do app | `app_entrou/app_saiu` `:760/766`, `app_presente` `:778`, `terminal_preso` `:784` | decide segurar permissão (`:1361-1368`) |
| `em_troca` | `adapters/claude_headless/sessions.py:88` (inclui `transfer_active`, `conversation_transfer.py:411`) | marcado por `runtime_policy.py:206-212`, `registry.py:2553, 2684` |
| Rebaixamento do registro nativo | `hook_state.py:163-189`, **só em memória** (`:171-176`) | pedido pelo Rust via `/internal/list/demote` (`facts.rs:188`) |

## 3. Prévia (`preview.py`, `PreviewBroker`)

| Peça | Python | Rust hoje |
|---|---|---|
| Fonte por provider | `sse.py:758-761`: pane (`PreviewBroker`) para Claude com terminal, Pi, omp, Kimi; push para Codex, sem terminal, orq | — |
| 1ª fonte: arquivo do hook MessageDisplay `.hangar-preview/<stem>.json` | `hooks/preview_hook.py`; leitura `preview.py:427-473`, `md=True, full=True` `:667-669` | nada |
| Arquivo com mais de 600 s vale se o marcador diz `working` | `preview.py:448-458` | nada |
| 2ª fonte: pane | `:677-701`: `shared_capture` → `crop_to_transcript` → `frame_analysis` ou `extract_assistant_text` | `terminal_state.rs:304-334` (`preview()` dentro do `analyze`) |
| Corte `bodyColumns+5` e âncora da faixa | `preview.py:20-25`, `:698-701` | nada (quadro cortado ≠ quadro do Rust, `frame_analysis` cai no Python) |
| `⎿` colado = chamada de ferramenta | `:301-305` | `terminal_state.rs:300-302` (igual) |
| Ritmo 0,15 s trabalhando / 0,75 s parado, só publica quando muda | `:735-742` | — |
| Época no `/clear`, `retired` | `:603-617, 644-647, 725-731` | — |
| Vaga única, só o último vale | `sse.py:769-773, 824-832, 1171-1189` | cache do hub (`side.rs:70`) |
| Não publicar o que já está no transcript (piso 16 chars) | `sse.py:54-72`, `:852-855`, `:972` | hub tem `FileTail`, não suprime |
| Campo `vivo` | `sse.py:1188` | tipo em `hangar-api/src/preview.rs` |
| Acúmulo de deltas (150 ms) | regra de `harnesses.md` | `LiveBuffer`, `runtime/mod.rs:13-42` (já usado no sem terminal) |

## 4. Pergunta nativa, sugestão, `problema`, aviso de entrega

| Peça | Python | Rust hoje |
|---|---|---|
| Hook PreToolUse grava `<config>/.hangar-askq/<sid>.json` | `hooks/askq_capture.py` | leitor só para a lista (`facts_files.rs:231-243, 396-474`) |
| Ler/limpar arquivo | `askquestion.py:16-40, 112-147, 149` | tipo `hangar-api/src/ask.rs` |
| Casar arquivo com o menu do pane | `sse.py:75-184` (`_TUI_EXTRAS`, `_CAIXA_MULTI`, opção com prévia por prefixo) | nada |
| Uma vez por pergunta; zera no `/clear` e no `__reprovider__` | `sse.py:1027, 1230-1236, 1122, 1158` | hub só repõe com `state=awaiting_input` (`side.rs:101-119`) |
| Resposta | `POST /answer` (`api.py:8918-8941`) → `runtime_terminal.answer_sync` (Rust digita); permissão em `/select` (`api.py:5797`) | executor Rust |
| `suggest` | só memória do plugin; sai no tique do `state` quando muda (`sse.py:1203-1206`) | cache do hub |
| `problema` do runtime | `sse.py:1211-1216` → `runtime_adapter.runtime_problem` `:608-621` (`_STALLED_INPUT` `:603-605`) | o código **nasce** no Rust (`runtime/terminal.rs:191`), vai ao Python e volta |
| Aviso de entrega | `sse.py:1240-1254`; `prev_deliverable=False` → cada conexão drena uma vez (`:1031`) | `ClaudeAdapter.drain` já só manda `{"kind":"drain"}` ao Rust (`adapters/claude.py:28-37`); o executor drena sozinho quando `deliverable` (`actor.rs:531, 634`) |
| Fatos de entrada do executor | `runtime_terminal.facts` `:293-335` (captura própria por subprocesso, plugin, socket nativo, clipboard) | pedidos por `runtime/terminal.rs:89-94` |

## 5. Consumidores

| Cliente | `state` | `preview` | `ask_question` | `suggest` |
|---|---|---|---|---|
| web `Chat.svelte` | `:2196`, fecha pergunta fora de `awaiting_input` (`:2212`); `problema` `:2794-2804` | `:2264`: `text, md, full, vivo` | `:2231`, dedupe Codex por `request_id` | `:2308` |
| mobile `stores/chat.ts` | `:398` | `:435` (sem `vivo`) | `:522` (não fecha ao sair de `awaiting_input`) | não lê |
| nativo `app.rs:1776-1787`, `chat.rs:176` | `hangar_api::state::StateEvent` | `vivo, md, full` | `Ask::new` | `terminal_suggestion` |

Códigos de `problema` repetidos em três lugares: `frontend/src/lib/problema.ts`,
`mobile/src/features/chat/SessionProblem.tsx`, i18n do nativo. Código novo entra nos três.

## 6. Outros provedores com terminal

| Provider | Estado no chat | `StateMonitor`? | `/events` |
|---|---|---|---|
| Codex | eventos do app-server (`codex/adapter.py:1612`, `_state_stream` `:1794`) | não | Rust (side-events) |
| Pi / omp | `pi/adapter.py:41-49`, captura Python (`identity` None) | sim | Python |
| Kimi | `kimi/adapter.py:31-40` | sim | Python |

Cartão de aprovação do Codex só existe **sem** terminal (`_aprovacao_pendente`,
`adapter.py:1659-1665`); com terminal a aprovação fica na TUI e o menu é lido por
`state.menu_codex` (`state.py:296-299`). O Rust calcula `codex_menu`
(`terminal_state.rs:271-296`) e ninguém usa.

## 7. Terminal real (`termsock.py`)

| Peça | Python | Rust hoje |
|---|---|---|
| Rotas | `WS /api/sessions/{name}/term` (`api.py:852-860`, `?shortcut=<id>`), `WS /api/hangar-terminals/{ident}/term` (`:863-871`, recusa convidado) | passam pelo `forward_upgrade` (`proxy.rs:84-118`) |
| Porta de entrada | `_porta_de_entrada` `termsock.py:358-411`: IP bloqueado, token só por `?token=`, convidado de convite pula token e Origin, convidado com login passa pela Origin, `share_gate.watch` fecha com 4410 | `auth.rs:33-118` (Bearer, query, cookie em GET) |
| Origin | `_origem_aceita` `:163-198`: mesma origem, `public_url`, `CP_TERM_ORIGINS` + `runtime_config.term_origins` (`:124-137`), `base_url` do `peers.json` (`:201-212`) | nada |
| Alvo | `resolve` em thread (`MuxIndisponivel` → 1013, vazio → 1008), `has_session` (`:390-403`); atalho por `shortcut_terminals.find` `:293-298`, `find_hangar` `:428-430` (`list-sessions -F`, `:251-273`) | nada |
| O que roda | `tmux attach -t ={name}:` (sessão, não pane), `TERM=xterm-256color`, `COLORTERM=truecolor`, `CLAUDE_CODE_TMUX_TRUECOLOR=1`, sem `NOTIFY_SOCKET/INVOCATION_ID/LISTEN_*`/`PSMUX_SESSION` (`:231-248, 737, 742`) | nada |
| POSIX | `pty.fork` + `execvpe`, `set_inheritable(False)`, leitura 64 KiB, fila com `add_writer`, `TIOCSWINSZ` (`:414-672`, `:106-108`) | nada |
| Windows | ConPTY por ctypes (`conpty.py`): `STARTF_USESTDHANDLES` com handles nulos, flags 0, pipe de entrada duplex, matar o filho antes do `ClosePseudoConsole` (`:194+`, `:251-264`) | nada |
| Desmontar | `detach-client -t <tty>` (nunca `-s`), SIGHUP, `waitpid` até 3 s, espera o tty sair do `list-clients`, repõe tamanho (`resize-window` + `setw window-size latest`) (`:273-334`); Windows `:746-760` | `terminal_process.rs` (árvore de processo, reaproveitável) |
| Protocolo | servidor → binário cru; cliente → binário (teclas) ou texto `{"t":"resize","cols","rows"}` com clamp 20..500 / 5..200 (`:567-593`) | — |
| Fechamentos | 1008 antes do accept; 1013 mux; 1000 "outra conexao assumiu"; 1011 ConPTY; 4410 convidado revogado | 403 de upgrade conta falha de auth (`routes.rs:242-244`) |
| Um painel por sessão | `_ativos` por alvo resolvido (`:92-99`); a nova derruba a antiga depois de esperar o escritor (`:422-453`) | nada |
| Contrapressão | fila de saída 1 MiB, pausa o leitor (`:91, 489-514`); dreno de 1 s no fim (`:619-632`) | — |
| 409 com painel aberto | `_recusa_se_painel_aberto` (`api.py:5782-5794`) lê `termsock.clientes_ativos()`; chamado em `/select`, `/select/submit`, `/btw`, `/service-tier`, `/answer`, `/model-effort`, `GET /model/options`, `/engine/model`, `/kimi/model` | nada |
| Capacidade | `painel_disponivel()` (`termsock.py:72-84`) → `/api/config.terminal_panel`, `/run-code` (`api.py:7767`) | nada |

Clientes: web `lib/term.ts:29-96` (`TermSocket`, resize com debounce 150 ms), `TerminalPanel.svelte:313, 443`,
`ShortcutTerminalTab.svelte:51`, `TerminalMobile.svelte:244`, fundo `lib/xterm.ts:26-35`;
nativo `desktop-native/src/ws.rs:104-116` (cliente WS próprio, `MAX_MESSAGE = 1 MiB`, responde
ping); Expo **não tem PTY** (espelho por `getPane` + `/term-input`).

A porta 8766 (convidado de convite) é escutada pelo Python direto (`main.py:155`).

Dependências: axum `0.8.9` sem a feature `ws` (`crates/Cargo.toml:24`); nenhum PTY em `crates/`
ou `desktop-native/`; `libc` e `windows-sys` já no workspace. `portable-pty 0.9.0`
(`wezterm/pty`) cria o ConPTY com `PSEUDOCONSOLE_INHERIT_CURSOR | RESIZE_QUIRK |
WIN32_INPUT_MODE` fixos (`pseudocon.rs:85-87`, o Python usa 0), usa `STARTF_USESTDHANDLES` com
`INVALID_HANDLE_VALUE` (`:120-123`) e fecha o pseudoconsole no `Drop` (`:72-73`).

## 8. Windows

- `-C` desligado: `terminal_control.rs:173`, `terminal_observer.py:116`. Estado e prévia no
  Windows são do Python hoje.
- A lista no Windows já é do Rust, com captura avulsa (`list/classify.rs:375-417`, alvo
  `=s:w.p` de `mux.rs:29`, prazo 5 s). Frágil: `rc != 0` vira `capture_refused` (`:407`, contra a
  regra "código de retorno no Windows não se lê como falha", `windows.md:35`); `from_utf8_lossy`
  sem marcar U+FFFD (`:408`, contra `windows.md:31`).

## 9. Provas que faltaram

- Cartão de permissão do Claude **com terminal**: a lista ficou `working` com o marcador preso
  depois do `Bash` (`sombra-isolada.md:71-73`, `prova-troca.md:91-92`); mesmo sintoma do item 5
  do handoff.
- Codex com cartão: abriu em YOLO, sem cartão (`sombra-isolada.md:74`, `prova-troca.md:93`).
- `lista-estado/prova-real.md` não existe (Task 24 aberta).

## 10. Medição e contrato

- Linha de base: `../lista-estado/medicao.md:44-55`, `prova-troca.md:62-70`; chat aberto =
  1,26 ms de CPU do Python por captura, ~1,3 captura/s por chat (`harnesses.md`, "Custo da
  observação terminal").
- Ferramentas: `scripts/medir-rust.sh` (backend real, só leitura), `scripts/medir-lista-hub.py`
  (isolado, 20 sessões falsas), `scripts/prova-dono-unico.py` (classe `Prova`, isolamento
  completo; base de `prova-lista-*.py`).
- Contrato interno hoje **27** (`rust_server.py:35`, `lib.rs:26`).

## Linhas velhas no esboço da Fase C

| Esboço | Hoje |
|---|---|
| `plugin_bridge` `:832 / :1273 / :807` | `:843 / :1284 / :818` |
| `sse.py:730, 1038` (`StateMonitor`) | `:735-746`, `:1054` (+ `:1125`, `:1166`) |
| `sse.py:1224-1237` (entrega) | `:1240-1254` |
| `sse.py:104-183` (pergunta) | `:105-184` |
| `api.py:1500-1559` (`_on_hook_transition`) | `:1563` |
| contrato 22 → 23/24 | 27 → próximo livre |
