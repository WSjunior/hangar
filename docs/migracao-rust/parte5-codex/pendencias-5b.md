# Parte 5: o que a 5B (e seguintes) herda

Entrada do plano da 5B. Origem: revisão final da 5A (PR #107) e o recado da metade Claude ao
concluir a 5-0 (07/10/2026, `7cb358073` na `hangar-server-parte5-claude`, contrato 37; detalhe em
`docs/migracao-rust/parte5-claude/registro-5-0.md` daquela branch).

## Da 5-0 (caminho de escrita comum)

- ~~Ligar o Codex é mudar as linhas Codex de `session_write/table.rs`~~ (feito na 5B, Task 4, para o Codex sem terminal; o com terminal é a 5C).
- `_born_in_rust = headless && rust_owns(provider, true)`: Codex com terminal precisa de gancho
  próprio na 5C (segue aberto).
- ~~`start_sessions` pode rodar antes do primeiro `configure_transport`~~ (fechado na 5B, Task 4).
- `format_status` do Codex já roda no Rust, mas `default_model`/`default_effort` não vêm no
  payload do motor Codex (decidir na 5E).
- A porta de entrada é por nome, e a troca de agente a segura (recusa na hora).

## Da 5A

- ~~**Aviso de versão:**~~ fechado na 5B, Task 1 (versão conferida agora 0.161.0, no lugar da 0.159.3; a pendência da 0.160 está coberta). Era: conferida 0.159.3; o notebook roda 0.160.1, então toda sessão Codex mostra
  `codex_versao_nao_conferida` (no nativo marca a sessão como problema na lista). Antes do uso
  real: `scripts/conferir-codex-schema` com a 0.160, ou mover o aviso para fora de `problema`.
- ~~`turn/completed` ilegível~~ (fechado na 5B, Task 2): `turn/completed` ilegível com `status: "failed"` não marca `headless_turno_erro` no caminho cru.
- ~~`command` vazio na aprovação~~ (fechado na 5B, Task 2): `command` vazio na aprovação ainda oferece "Sempre permitir" (tratar vazio como ilegível).
- O cliente aborta a leitura sem drenar as últimas linhas quando o escritor morre (segue aberto).
- ~~Pedido do servidor de outra thread (subagente)~~ (fechado na 5B, Task 2): pedido do servidor de outra thread (subagente) ainda é descartado pelo filtro de thread.
- ~~`spawn_stdio` no Windows~~ (fechado na 5B, Task 3, módulo `process.rs`; Windows só provado pelo verificar-local/CI): resolver `codex.cmd` (PATHEXT) e matar a árvore (`taskkill /T`).

## Aberto depois da 5B

Fechado na 5B: Codex sem terminal do nascimento ao fim no Rust (cano, religar, controles, tabela
de pedidos, rotas só do Codex), contrato 38. Falta e fica para depois:

- **Medição e uso real (Task 8, passos 2 e 3):** CPU e RSS de 5 sessões Codex sem terminal em
  backend isolado, e o roteiro com o dono (criar, aprovar, formulário MCP, Stop, trocar
  modelo/Fast/modo/permissão, matar o `hangar-cano`, reiniciar com turno rodando, apagar a
  sessão, `CP_RUST_SERVER=0`). Não subir backend nesta máquina sem lançador com no-op: o Rust
  varre órfãos ao subir. Windows só provado pelo CI.
- **`POST /recarregar` do Codex fica no Python** (precisa do registro de transferência); sai na
  parte 6/7. O reinício já vai ao Rust.
- **`!t.terminal` em `admit_codex` sem caso real até a 5C** (o Rust registra terminal sempre como
  Claude); conferir que as rotas restart/recarregar do runtime só olham `provider == "codex"`
  sem desviar Codex com terminal.
- **`default_model`/`default_effort`** continuam fora do payload do motor Codex: 5E.
- **Esforço na subida:** o `settings/update` manda só `effort`; o Python manda também `model`.
- **Formulário MCP:** não respeita `required`; rótulos duplicados de Choice resolvem para o
  primeiro; cartão de link sem `message`/`url` mostra ": " solto; faltam testes de decline sem
  `requestedSchema`, multi-select e `-32601` com nota.
- **Pedidos de subagente guardados na abertura da voz** contam no `MAX_FRAME`, e o
  `serverRequest/resolved` deles não é guardado (cartão pode ficar pendente).
- **Subida/religar:** cano `Ours` sem escuta/token vira `""` e a conexão falha sempre; faltam
  testes dos caminhos de falha (`patch_meta` falha, `discard`+`clear_cano`, `NotListening` com pid
  antigo, dois `open` simultâneos); `respawn.carried` só zera em subida boa; `sweep_orphans`
  cancela tudo com um `.json` ilegível; `restart` em sessão morta sobe duas vezes; comentário em
  `actor.rs` ~990 desatualizado; `cano_binary` guarda em cache a ausência (binário instalado
  depois só com restart).
- **Rotas:** lista de modelos esvazia com campo fora do schema; `/interrupt` pendura se o
  `thread/read` intermediário falha; `/codex/mode` não grava `previous_non_plan`;
  `read_rate_limits` do convidado só engole `RuntimeError`/`ValueError`; `/answer` recusado pelo
  ator vira 503 no Python, não 409; `prepare_session` lê o sidecar a cada envio.
- **Front:** chave `native_ask_skip` sem uso; URL longa em botão ghost no nativo pode transbordar
  (conferir na tela); pergunta assíncrona do Codex no nativo diz "Cancelar" (antes "Pular"); sem
  teste do roteamento Skip×Cancel.
- **Processo:** `resolve_program` não confere o bit de execução; `set_var` de `CP_RUST_CANO_BIN`
  em teste com outras threads.
- **Claude sem terminal no mesmo caminho caro (da Task 9):** a prévia dele ainda vai ao Python por
  `/runtime/events` e volta como `StateEvent` inteiro pelo `RuntimeAdapter.state_stream`, como o
  Codex antes da Task 9 ([medicao-5b.md](medicao-5b.md), seção "Depois da Task 9"). O feed
  (`state/runtime_feed.rs`) não depende de provedor: ligar é o ator Claude escrever no `watch` e o
  predicado do hub aceitar `ClaudeHeadless`. Fica com a metade Claude, que ainda tem `suggest` e a
  faixa a conferir.
- **Contrato 39** nesta branch: a junção reconfere o próximo número livre na `main`.
- **Renomear sessão Codex sem terminal** com chat aberto: o canal do feed é por nome e o hub
  também; conferir no uso real que o rename fecha e reabre os dois.
