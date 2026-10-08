# Parte 5 (Codex): análise (07/10/2026)

Medido nesta máquina (codex-cli 0.159.3, rustc 1.98.1) sobre a `main` em `dcae49227`. As
medições do notebook (codex-cli 0.160.1) vêm da branch `fix/codex-hardening` e estão marcadas.
Escopo pedido pelo dono: **tirar todo o Codex do Python**, não só sessão, envio e estado. Pi, omp
e Kimi ficam fora.

## Conclusão

1. **O núcleo já existe no Rust e nunca rodou.** `runtime/codex.rs` (1.123 linhas, 28 testes) é
   um motor completo do Codex sem terminal: `initialize`, `thread/start|resume`, `turn/start|
   steer|interrupt`, aprovações, perguntas, Fast, voz. Está ligado no ator e no gateway; quem o
   deixa parado é uma linha do Python (`runtime_coordinator.py:515-517`, `_born_in_rust` só aceita
   `provider == "claude"`) e a decisão 2 do dono único ("fica no Python até prova real").
2. **O resto é grande e espalhado.** ~13,3 mil linhas de Python só do Codex (12,3 mil em
   `adapters/codex/` e `codex_*.py`, mais `oauth_codex`, `uso_codex`, `claude_to_codex`), ~1,1 mil
   de scripts de lançamento e ganchos, 18,4 mil linhas de teste em 52 arquivos. Além disso, o
   Codex tem ramos dentro de módulos compartilhados: ~486 ocorrências em `api.py`, 168 em
   `registry.py`, e trechos em `sse.py`, `state.py`, `terminal_input.py`, `cotas.py`,
   `costs_sources.py`, `config_sync.py`, `archive*.py`, `bastao.py` e outros.
3. **Tipos do protocolo: schema do próprio binário, com tipos escritos à mão só do que o Hangar
   usa.** O crate oficial custa 664 crates e não compila como dependência sem copiar patches e
   lockfile do Codex a cada versão. Detalhe em "Tipos do protocolo".
4. **O que trava a saída completa não é o protocolo, é o acoplamento.** `codex_contas.py` é
   importado por 22 módulos; os lançadores `hangar-codex-tui` e `hangar-codex` importam módulos do
   backend direto pelo Python do sistema; `codex_voice_broker` importa `app.api`. A ordem das
   subpartes da spec sai disso.

## Onde está cada coisa hoje

| Bloco | Python (linhas) | Rust hoje |
|---|---|---|
| Núcleo de sessão: `adapter.py` 2626, `appserver.py` 431, `sem_terminal.py` 203, `sessions.py` 395, `lancador.py` 100, `questions.py` 45, `async_questions.py` 135, `chat_controls.py` 19 | 3.954 | Motor sem terminal pronto e parado; cano Rust já hospeda o app-server (`hangar-cano`, guarda pedidos pendentes e texto em voo por thread) |
| Conversa: `rollout.py` 373 | 373 | Portado (`transcript/codex.rs`), atende `/history` e `/events` |
| Custos/uso: `uso_codex.py` 218, trechos de `costs_sources.py` | 218 | `costs/codex.rs` portado; escopos de conta chegam do Python |
| Contas, cota, catálogo: `codex_contas*.py` (5 arquivos) 3.147, `codex_appserver.py` 227, `codex_models.py` 296, `oauth_codex.py` 376, trechos de `cotas.py` | 4.046 | Nada |
| Integração nativa: `codex_integracao` 1105, `importador` 409, `instrucoes` 220, `hooks_arquivos` 158, `hook_installer` 136, `fragmentos` 134, `skills` 393, `compat` 291, `arquivos` 268, `opcoes` 66, `msgs` 104 | 3.284 | Nada |
| Transferência Claude→Codex: `transfer.py` 360, `claude_to_codex.py` 409, trechos de `conversation_transfer.py` e `conversation_history.py` | 769 | Nada |
| Voz (beta): `codex_voice.py` 149, `codex_voice_broker.py` 365 | 514 | O motor já repassa `thread/realtime/*` |
| Terminal: `codex_permissions.py` 133 (seletor `/permissions`), `menu_codex` em `state.py` | 133 | Monitor da parte 4 já captura o pane do Codex (`terminal_state.rs:297`), sem envio |
| Lançadores: `hangar-codex-tui` 662, `hangar-codex` 290, `codex-hook-allow.py` 49, `codex-hook-json.py` 52, `shell/codex.*` 63 | 1.116 | Nada |

Rotas: todas as `POST` de controle do Codex (`/input`, `/steer`, `/interrupt`, `/select`,
`/answer`, `/question/skip`, `/models`, `/model`, `/service-tier`, `/codex/mode`,
`/codex/plan/implement`, `/codex-permissions`, `/limits`, `/commands`, `/recarregar`,
`/modo-execucao`, `/codex/voice`) e `/api/codex-contas`, `/api/harness/codex/*`,
`/api/credenciais/codex*`, `/api/model-options?provider=codex` são repassadas ao Python
(`migration_status.rs:57-62, 81, 135`). Estado, prévia e `ask_question` do Codex saem do
`side-events` do Python (`side.rs:1048`: "Sessão sem Monitor (Codex): o Python segue dono").

## Como o Codex funciona hoje (o que o Rust precisa reproduzir)

**Sem terminal.** O backend sobe `hangar-cano` (ou `cano.py`) num escopo do systemd com
`codex app-server --stdio -c sandbox_mode=… -c approval_policy=…`, conecta por socket com token,
manda `initialize` (`experimentalApi: true`) e abre/retoma a thread. Um cliente por cano;
religar lê o snapshot (pedidos pendentes voltam, texto em voo vira `turn/started` + delta
sintéticos). Três modos de permissão; trocar de sandbox reabre o servidor ocioso. `watch_sessions`
religa todo sidecar a cada 2 s. Órfão = cano sem sidecar, achado por `HANGAR_CANO_KEY` no
`/proc/*/environ`.

**Com terminal.** O lançador `hangar-codex-tui`, dentro do pane, sobe
`codex app-server --listen ws://127.0.0.1:<porta>` e a TUI `codex --remote`; captura o
`thread/started` e grava o sidecar (`endpoint`, pids, `thread_id`). O backend conecta no mesmo
WebSocket como segundo cliente e assina a thread com `thread/resume` (sem isso não recebe
`turn/*`). Envio é `turn/start` pelo WebSocket, não teclado. O teclado só entra no seletor
`/permissions`, no "implementar plano" e no menu antes de existir thread.

**Estado.** Um consumidor por sessão traduz notificações em `StateEvent`: `turn/started|completed`,
`thread/status/changed`, `item/agentMessage/delta` (prévia), `thread/tokenUsage/updated` (💬),
`account/rateLimits/updated` (⚡/📅), `thread/settings/updated` (pílulas), `error` com
`willRetry` (`codex_sem_conexao`), `hook/completed` (`codex_prompt_bloqueado`),
`model/safetyBuffering/updated`. A linha de status é montada no Python
(`format_status_line`); o motor Rust já pede isso por `/internal/runtime/policy` (`format_status`).

## Lacunas (comparação com o T3 Code, 07/10)

- **Pedidos do servidor recusados.** O schema 0.159.3 tem 11 pedidos do servidor. O Hangar atende
  3 (`item/commandExecution/requestApproval`, `item/fileChange/requestApproval`,
  `item/tool/requestUserInput`) e responde `-32601` aos outros, inclusive
  `item/permissions/requestApproval` (pede leitura/escrita em caminhos e rede, resposta com
  `scope: turn|session`) e `mcpServer/elicitation/request` (formulário MCP com `requestedSchema`
  de campos simples, ou URL, resposta `accept|decline|cancel`). Os legados `applyPatchApproval` e
  `execCommandApproval` são do protocolo v1; `currentTime/read` só sai com `--experimental`.
- **Pedido de outra thread some.** O filtro de thread (`adapter.py:1916`) roda antes do ramo de
  pedidos: pedido de subagente não é recusado nem mostrado, e o turno pode pendurar.
- **Sem `thread/fork`, `thread/revert`, `thread/goal/*` nem lista de terminais em segundo plano
  (`thread/backgroundTerminals/list|terminate|clean`).** Só `backgroundTerminals/*` é
  experimental; fork, revert e goal estão no protocolo estável.
- **Protocolo lido como dicionário solto**, no Python e no Rust. O rename `reasoningEffort` →
  `effort` deixou a pílula vazia sem erro. Hoje o nome muda conforme o lado: `effort` nos pedidos
  (`TurnStartParams`, `ThreadSettingsUpdateParams`) e em `ThreadSettings`; `reasoningEffort` nas
  respostas (`ThreadStartResponse`, `Thread`); `reasoning_effort` no `collaborationMode`.

## Tipos do protocolo

Medido em `/tmp/parte5-codex-medicao` (fora do repositório), codex-cli 0.159.3.

**(a) Schema emitido pelo binário** (`codex app-server generate-json-schema --experimental`):
440 arquivos, 4,29 MB; arquivo único v2 com 736 KB e 789 definições. 167 pedidos do cliente
(104 sem `--experimental`), 11 do servidor, 83 notificações (iguais com e sem a opção).
- Geração completa por `typify` 0.8: o arquivo único v2 gera 72 mil linhas e 899 tipos que
  compilam em ~20 s só com serde. Mas: `ServerRequest` derruba o gerador até trocar os esquemas
  booleanos `true` por `{}`; `ServerNotification` vira `untagged` com `Variant0..Variant82`
  (tenta uma por uma, erro ilegível); saem 12-13 `deny_unknown_fields` que quebram a cada campo
  novo do Codex.
- A versão vem de graça no `initialize`: `userAgent` = `"<cliente>/0.159.3 (…)"`.

**(b) Crate oficial `codex-app-server-protocol`** (tag `rust-v0.159.3`): a OpenAI não publica no
crates.io (o nome lá é uma republicação de terceiros, 0.63.0). Do git: 22 dependências diretas, 12
internas do Codex, **664 crates** no total (aws-lc-sys, openssl-sys, sqlx, tonic, keyring, image);
build do zero 4 min 17 s, target 3,4 GB. Como dependência fora do workspace do Codex **não
resolve**: o `[patch.crates-io]` dele (forks de `tungstenite`) não vale para quem depende; com o
patch copiado o lock novo puxa um `rama` alfa que não compila; só passa copiando também o
`Cargo.lock` do Codex. Cada atualização do Codex prenderia o lockfile do Hangar ao dele.

**T3 Code** gera TypeScript de um commit fixo do Codex (schema versionado no repositório do Codex,
igual à saída sem `--experimental`), instala um binário Codex próprio com sha256 e regenera em PR a
cada versão (7 vezes entre abril e setembro; afrouxa campos pontuais como `PlanType`). O Hangar não
fixa o Codex: usa o que a pessoa tem instalado. Por isso a diferença de versão tem de ser vista em
tempo de execução, não só no build.

**Recomendação:** (a) enxuta. Tipos escritos à mão para o que o Hangar usa (~25 métodos, as três
uniões com `#[serde(tag = "method")]` e variante de reserva para método desconhecido, campos
opcionais com `default`, nunca `deny_unknown_fields`); um recorte do schema da versão conferida
guardado no repositório só para teste; aviso quando a versão do `initialize` difere da conferida.
Desenho na spec.

## Medidas do notebook que a spec assume (`fix/codex-hardening`, codex-cli 0.160.1)

- `turn/interrupt` deixa o comando do turno vivo; o Stop passa a mandar
  `thread/backgroundTerminals/terminate` depois.
- App-server morto no meio do turno: depois do `resume`, a thread fica `idle` e o último turno
  `interrupted` (vira `codex_turno_cortado`).
- Sem `summary` no `turn/start`, o rollout guarda o pensamento só cifrado; com
  `summary: "detailed"` chegam `item/reasoning/*` ao vivo.
- `codexErrorInfo` distingue limite de uso e falta de login (`codex_limite_uso`,
  `codex_sem_login`); kill por pid confere a identidade do processo.

A branch tem 2 commits sobre `fa6617111`, sem revisão nem testes rodados. O plano parte dela.

## Riscos

- **Protocolo experimental muda a cada versão** (32% dos commits do backend corrigem CLI externa).
  O tipo tolerante + aviso de versão + recorte do schema transformam rename calado em teste
  vermelho na atualização e em aviso visível na máquina.
- **Codex sem terminal no Rust nunca rodou no uso real** (motivo da decisão 2 do dono único). Cada
  subparte de sessão termina com prova real antes de virar o dono.
- **Lançadores no Python do sistema.** Enquanto importarem módulos do backend, o Python não sai.
- **`codex_contas` com 22 consumidores Python.** Mover a escrita não pode quebrar quem só lê.
- **Windows:** travas `msvcrt`, caminhos `ntpath`/UNC em `codex_skills`, `PureWindowsPath` em
  `codex_compat`, `codex.CMD`/PATHEXT no lançador, `taskkill /T` no kill do cano, psmux no seletor
  `/permissions`. A parte 4 deixou a observação tmux `-C` desligada no Windows.
- **Teste:** 18,4 mil linhas de teste Python do Codex. O que vale como contrato vira golden ou
  teste Rust; o resto sai com o código na parte 7.
