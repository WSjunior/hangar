# Parte 5, Task 5-0: caminho de escrita no Rust — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** com o Rust de pé, `/input`, `/steer`, `/interrupt`, `/select`, `/select/submit`,
`/answer`, `/keys`, `/term-input` e `DELETE …/queue/{id}` de sessão Claude entram e são
executados no Rust, sem a ida e volta Rust → Python → Rust; `prepare_prompt`, `format_status` e
`skill_catalog` deixam de ser pedidos ao Python.

**Architecture:** um módulo novo `session_write` no `hangar-server` reivindica as rotas, acha a
entrada do ator pelo nome no `RuntimeRegistry` e decide por uma tabela (rota × provedor × modo ×
saúde da entrada) se atende ou repassa ao Python com o corpo intacto. Uma porta de entrada por
sessão no Rust (`IngressGates`) substitui o `_transfer_guard`/`freeze` do Python para as escritas
que o Rust atende: o Python a fecha e abre por um comando novo do gateway. As políticas puras
passam a rodar dentro do ator (`runtime/local_policy.rs`), no mesmo lugar onde o `local_output`
já roda.

**Tech Stack:** Rust (axum, tokio, serde_json) no `crates/hangar-server`; Python 3.14 + FastAPI
no `backend/app`; golden gerado pelo Python em `backend/tests/fixtures/contract/`.

**Spec:** `docs/migracao-rust/parte5-claude/spec.md` (seção "5-0"), com `analise.md` e
`contrato-par.md` ao lado. Linhas conferidas em `0b779d4bf`; cada Task reconfere as dela antes de
codar.

## Global Constraints

- **Testes (regra do projeto):** cada Task escreve os testes dela; rodar só quando o dono
  autorizar na abertura da execução, e então só os focados: `cd backend && uv run pytest
  tests/<x>.py` e `cd crates && nice -n 10 env CARGO_BUILD_JOBS=4 cargo test -p hangar-server
  --test <x>` (ou `--lib <mod>::`). No máximo 2 `cargo` na máquina (`pgrep -c -x cargo`); `target/`
  desta worktree apagado no fim; `rust-analyzer` desligado na worktree.
- **Contrato interno:** a 5-0 é uma junção só e sobe **um** número nos dois lados
  (`backend/app/rust_server.py:35`, `crates/hangar-server/src/lib.rs:30`, teste fixo
  `crates/hangar-server/tests/proxy.rs:30`), o próximo livre na hora de juntar (hoje 36 → 37). Se
  a metade Codex juntar antes, pega o seguinte.
- **Paridade:** toda resposta que o Rust passa a dar (corpo e código HTTP, inclusive erro) sai
  igual à do Python, provada por golden gerado rodando a rota/função Python
  (`backend/tests/fixtures/contract/gen_golden.py`). Mudou a regra no Python durante a execução →
  regenerar no mesmo commit.
- **Dono único:** com o Rust de pé, erro numa escrita que o Rust atende é erro com código
  (`route_failed`, `routes.rs:178`), nunca repasse ao Python depois de começar a executar. O
  repasse só existe na decisão inicial (linha `Python` da tabela, entrada ausente ou não saudável).
- **Nada novo no backend do notebook do dono.** Nenhum backend real, nenhuma sessão real. O uso
  real é do dono com a sessão `hangar`, pelo canal de testes, depois da junção.
- **Desempenho:** conferir cada Task contra "Desempenho: erros que já custaram"
  (`docs/migracao-rust/README.md`): nada de laço novo, leitura de arquivo em `spawn_blocking`,
  canais com limite, cache com teto.
- Log e diário só com código, nome da sessão e nome de campo; nunca texto de prompt, resposta ou
  tecla.
- Identificador novo em inglês; comentário curto em português, sobre o porquê. `git add` por
  caminho; commits descritivos em inglês; sem push.
- Harnesses: ler "Regras vigentes" de `docs/decisoes/harnesses.md` antes das Tasks 4–6. Windows:
  ler "Regras vigentes" de `docs/decisoes/windows.md` antes de mexer em tecla do pane (Tasks 5–6).

## Review Focus

- **Escrita durante troca de conta ou relançamento** (`/rename`, troca de modelo, transferência
  de conversa): a escrita espera a porta reabrir (até 30 s) e cai na geração nova; nunca escreve
  na entrada que está fechando nem responde 503 calado. Teste na Task 2.
- **Corpo inválido ou campo a mais** (`{"text": "oi", "x": 1}`): mesma resposta 422 do Python,
  porque o corpo vai intacto ao Python. Teste na Task 3.
- **Entrada do ator existe mas está doente** (cano morto, observação do terminal falhando,
  sessão nascendo): repasse ao Python, que reabre como hoje; nunca 503 na primeira tentativa.
  Teste na Task 3.
- **Permissão segurada pelo plugin e opção 3** no `/select`: 409 `erro_opcao_nao_convergiu`,
  nenhuma tecla no pane. Teste na Task 5.
- **`/steer` de sessão sem terminal recusado pelo ator**: 409 `erro_sem_turno` (o Python hoje
  deixa o `ValueError` virar 500; o golden registra a correção). Teste na Task 4.

---

## Arquivos

| Arquivo | Responsabilidade |
|---|---|
| `crates/hangar-server/src/session_write/mod.rs` (novo) | rotas, extração do corpo, repasse com corpo intacto, envelope de erro |
| `crates/hangar-server/src/session_write/table.rs` (novo) | `Provider`, `WriteRoute`, `decide()` |
| `crates/hangar-server/src/session_write/input.rs` (novo) | `/input`, `/steer` |
| `crates/hangar-server/src/session_write/control.rs` (novo) | `/interrupt`, `/keys`, `/term-input`, `/select`, `/select/submit`, fila |
| `crates/hangar-server/src/session_write/answer.rs` (novo) | `/answer`, texto do "conversar sobre isso" |
| `crates/hangar-server/src/runtime/ingress.rs` (novo) | `IngressGates` |
| `crates/hangar-server/src/runtime/local_policy.rs` (novo) | `prepare_prompt`, `format_status`, `skill_catalog` |
| `crates/hangar-server/src/runtime/gateway.rs` | `writable()`, comando `ingress` |
| `crates/hangar-server/src/runtime/actor.rs` | políticas locais antes do `PolicyClient` |
| `crates/hangar-server/src/routes.rs`, `migration_status.rs`, `lib.rs` | registro das rotas, tabela de migração, contrato |
| `backend/app/runtime_coordinator.py` | porta do Rust no `freeze`, `rust_owns()` |
| `backend/app/conversation_transfer.py` | porta do Rust na troca |
| `backend/app/internal_api.py` | `/internal/sessions/{n}/plugin` (temporária, sai na C2), `/internal/quota` |
| `backend/app/runtime_policy.py` | saem `prepare_prompt`, `format_status`, `skill_catalog`, `answer_body`, `session.marker`, `diag.error`, `quota` |
| `backend/tests/fixtures/contract/gen_golden.py` | golden das rotas e políticas |

---

### Task 1: Entrada por nome e porta de entrada no Rust

**Files:**
- Create: `crates/hangar-server/src/runtime/ingress.rs`
- Modify: `crates/hangar-server/src/runtime/gateway.rs:26-30,102-117,362-415`, `crates/hangar-server/src/runtime/mod.rs`, `crates/hangar-server/src/lib.rs:30`, `backend/app/rust_server.py:35`, `crates/hangar-server/tests/proxy.rs:30`
- Test: `crates/hangar-server/src/runtime/ingress.rs` (`#[cfg(test)]`), `crates/hangar-server/tests/runtime_ingress.rs` (novo)

**Interfaces:**
- Produces:
  - `pub struct IngressGates` com `pub async fn enter(&self, key:&str, wait:Duration) -> Result<IngressPass, GateClosed>` (espera a porta abrir até `wait`; `IngressPass` solta no `Drop`), `pub async fn close(&self, key:&str) ` (fecha e espera os passes em curso acabarem), `pub fn open(&self, key:&str)`.
  - `pub struct GateClosed;` — vira 409 `session_transfer_busy` na rota.
  - `RuntimeRegistry::writable(&self, name:&str) -> Option<WriteTarget>` com `pub struct WriteTarget { pub key:String, pub generation:u64, pub provider:String, pub terminal:bool, pub healthy:bool, pub(crate) handle:EntryHandle }`.
  - `RuntimeRegistry::ingress(&self) -> &IngressGates`.
  - Comando do gateway `{"kind":"ingress","closed":bool}` (envelope com `key`; geração ignorada, a porta é por chave).

- [ ] **Step 1: Testes da porta (falham sem o módulo)**

```rust
#[tokio::test]
async fn close_waits_for_passes_and_blocks_new_ones() {
    let gates = IngressGates::default();
    let pass = gates.enter("k", Duration::from_millis(10)).await.unwrap();
    let g = gates.clone();
    let closing = tokio::spawn(async move { g.close("k").await });
    tokio::time::sleep(Duration::from_millis(20)).await;
    assert!(!closing.is_finished());            // espera a escrita em curso
    drop(pass);
    closing.await.unwrap();
    assert!(gates.enter("k", Duration::from_millis(20)).await.is_err());
    gates.open("k");
    assert!(gates.enter("k", Duration::from_millis(20)).await.is_ok());
}

#[tokio::test]
async fn closed_gate_lets_writer_through_when_reopened_in_time() {
    let gates = IngressGates::default();
    gates.close("k").await;
    let g = gates.clone();
    tokio::spawn(async move { tokio::time::sleep(Duration::from_millis(30)).await; g.open("k") });
    assert!(gates.enter("k", Duration::from_secs(1)).await.is_ok());
}
```

- [ ] **Step 2: Implementar `ingress.rs`**

```rust
//! Porta de entrada das escritas por chave de sessão. Quem troca de dono ou relança (Python) fecha;
//! a rota do Rust espera reabrir em vez de escrever numa entrada que está fechando.
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::watch;

#[derive(Clone, Default)]
pub struct IngressGates { inner: Arc<Mutex<HashMap<String, Gate>>> }

struct Gate { closed: watch::Sender<bool>, passes: watch::Sender<usize> }

pub struct GateClosed;
pub struct IngressPass { gates: IngressGates, key: String }

impl IngressGates {
    fn with_gate<T>(&self, key: &str, f: impl FnOnce(&Gate) -> T) -> T {
        let mut map = self.inner.lock().unwrap();
        f(map.entry(key.into()).or_insert_with(|| Gate { closed: watch::channel(false).0, passes: watch::channel(0).0 }))
    }
    pub async fn enter(&self, key: &str, wait: Duration) -> Result<IngressPass, GateClosed> {
        let deadline = tokio::time::Instant::now() + wait;
        loop {
            // Conferir e contar sob a mesma trava do `close`: senão um fechamento passa entre os dois.
            let mut closed = match self.with_gate(key, |g| {
                if *g.closed.borrow() { Err(g.closed.subscribe()) } else { g.passes.send_modify(|n| *n += 1); Ok(()) }
            }) {
                Ok(()) => return Ok(IngressPass { gates: self.clone(), key: key.into() }),
                Err(rx) => rx,
            };
            tokio::time::timeout_at(deadline, closed.wait_for(|c| !*c)).await.map_err(|_| GateClosed)?.map_err(|_| GateClosed)?;
        }
    }
    pub async fn close(&self, key: &str) {
        let mut passes = self.with_gate(key, |g| { g.closed.send_replace(true); g.passes.subscribe() });
        let _ = passes.wait_for(|n| *n == 0).await;
    }
    pub fn open(&self, key: &str) {
        if let Some(g) = self.inner.lock().unwrap().get(key) { g.closed.send_replace(false); }
    }
}

impl Drop for IngressPass {
    fn drop(&mut self) {
        if let Some(g) = self.gates.inner.lock().unwrap().get(&self.key) { g.passes.send_modify(|n| *n -= 1); }
    }
}
```

Teto: um `Gate` por chave que já escreveu; some no `close` do registro (`close_locked`,
`gateway.rs:223`) quando estiver aberto e sem passes. Acrescentar ao Step 1 um teste de corrida:
100 `enter` concorrentes com um `close` no meio; depois que o `close` volta, nenhum `enter` novo
devolve passe até o `open`.

- [ ] **Step 3: `writable()` e comando `ingress` no gateway**

`writable` percorre `entries` como `terminal_view` (`gateway.rs:249`) e devolve a entrada cujo
`name` bate. `healthy` vem do retrato (`handle.snapshot()`, prazo de 1 s como em `:281`):
sem terminal, `view.alive == true && view.initialized == true && error` ausente; com terminal,
`error` ausente e `view.error` fora de `_TERMINAL_PRE_EFFECT_ERRORS` do Python
(`runtime_coordinator.py`, procurar o nome). Conferir os nomes exatos dos campos no
`snapshot()` de `runtime/actor.rs` e `runtime/terminal.rs` antes de escrever.

No `dispatch` (`gateway.rs:362`): `"ingress"=>&["kind","closed"]`; tratado antes do
`registry.entry` (a porta vale sem entrada aberta):

```rust
if kind == "ingress" {
    let closed = command["closed"].as_bool().ok_or_else(||failure("ingress_payload"))?;
    if closed { registry.ingress().close(&envelope.key).await } else { registry.ingress().open(&envelope.key) }
    return Ok(json!({"closed":closed}));
}
```

- [ ] **Step 4: Teste de integração do comando** (`tests/runtime_ingress.rs`): envelope `ingress`
  com `closed:true` responde `{"ok":true,"result":{"closed":true}}`; campo a mais responde 503
  `command_fields`.

- [ ] **Step 5: Subir o contrato** para 37 nos três lugares do Global Constraints.

- [ ] **Step 6: Commit**

```bash
git add crates/hangar-server/src/runtime/ingress.rs crates/hangar-server/src/runtime/mod.rs crates/hangar-server/src/runtime/gateway.rs crates/hangar-server/tests/runtime_ingress.rs crates/hangar-server/src/lib.rs backend/app/rust_server.py crates/hangar-server/tests/proxy.rs
git commit -m "feat(server): per-session ingress gate and name lookup for Rust-served writes"
```

### Task 2: O Python fecha a porta do Rust no relançamento e na troca

**Files:**
- Modify: `backend/app/runtime_coordinator.py` (`freeze`, `_enter_rust`), `backend/app/conversation_transfer.py:446-479` (`session_operation`), `backend/app/rust_server.py` (`RustTransport`)
- Test: `backend/tests/test_runtime_ingress.py` (novo)

**Interfaces:**
- Consumes: comando `{"kind":"ingress","closed":bool}` (Task 1).
- Produces: `RuntimeCoordinator.ingress(name:str, closed:bool) -> Awaitable[None]` (sem slot Rust: não faz nada) e `ingress_sync(name, closed)` para o código em thread.

- [ ] **Step 1: Testes** com um `transport` falso que grava os comandos:
  - `freeze(name)` manda `ingress closed=True` antes de qualquer `close/open` e `closed=False` na saída, inclusive com exceção dentro do bloco.
  - `session_operation(name)` faz o mesmo (pela versão síncrona) e, se o registro da troca ficar em fase não terminal ao sair, **não** reabre.
  - `_enter_rust` manda `closed=True` para cada sessão de `list_incomplete()`.
  - Sem Rust (`transport is None`) nenhum comando sai.
- [ ] **Step 2: Implementar.** `freeze` já serializa por nome; a porta fecha dentro dele, antes
  do primeiro efeito. A transferência termina por `save_transfer` com fase terminal: ali chamar
  `ingress_sync(name, False)`. Falha ao mandar `ingress` com o Rust de pé sobe como erro da
  operação (não é ignorada: escrever com a porta aberta durante a troca é o defeito que ela evita).
- [ ] **Step 3: Sessão Claude sem vínculo no Rust é erro** (decisão 8 da spec). No
  `_send_one_available` (`api.py:4364-4385`), com o modo `rust` e provedor `claude`, quando
  `_send_managed` devolve `None` (o `prepare_session` não achou vínculo), responder
  `{"ok":False,"error":erro("erro_envio_falhou","sessão Claude sem vínculo no Rust"),"delivered":False}`
  em vez de seguir ao socket nativo/plugin/tmux do Python. No modo `python` nada muda. Teste:
  coordenador falso no modo `rust` com `prepare_session → False` responde o erro e não chama
  `terminal_input`.
- [ ] **Step 4: Commit** — `fix(runtime): close the Rust ingress gate during relaunch and conversation transfer`.

### Task 3: Tabela de despacho e esqueleto das rotas

**Files:**
- Create: `crates/hangar-server/src/session_write/mod.rs`, `crates/hangar-server/src/session_write/table.rs`
- Modify: `crates/hangar-server/src/lib.rs` (módulo), `crates/hangar-server/src/routes.rs:211-245`, `crates/hangar-server/src/migration_status.rs:98-118`
- Test: `table.rs` (`#[cfg(test)]`), `crates/hangar-server/tests/session_write_routes.rs` (novo, com `tests/fake/mod.rs`)

**Interfaces:**
- Consumes: `RuntimeRegistry::writable`, `IngressGates::enter` (Task 1); `routes::gate`, `routes::pass`, `routes::route_failed`, `routes::cors`.
- Produces:
  - `pub enum Provider { Claude, Codex }` (`from_str("claude"|"codex")`).
  - `pub enum WriteRoute { Input, Steer, Interrupt, Select, SelectSubmit, Answer, Keys, TermInput, QueueRemove }`.
  - `pub fn decide(route:WriteRoute, provider:Provider, terminal:bool, healthy:bool) -> Owner` com `pub enum Owner { Rust, Python }`. Linhas `Codex` todas `Python` nesta Task (a metade Codex vira as dela).
  - `pub(crate) struct Ctx { st:Arc<AppState>, name:String, target:WriteTarget, pass:IngressPass, headers:HeaderMap }` e `pub(crate) async fn admit(st, peer, req, route) -> Result<(Ctx, Bytes), Response>`: devolve `Err(resposta do Python)` quando a decisão é `Python`, quando quem pede não é o dono (convidado segue ao Python), quando não há entrada (`writable` vazio) ou quando o corpo não desserializa — em todos esses casos o pedido sai ao Python **com o corpo já lido**, remontado com `Request::from_parts(parts, Body::from(bytes))`.
  - `pub(crate) fn detail(status:StatusCode, code:&str, msg:&str, params:Value) -> Response` → corpo `{"detail":{"code","params","msg"}}`, o envelope do `HTTPException(detail=erro(...))` (`mensagens.py:16`).
  - `admit` espera a porta até 30 s; porta fechada além disso → 409 `{"detail":{"code":"session_transfer_busy",...}}` com a mensagem de `public_error` (`conversation_transfer.py:486-490`).

- [ ] **Step 1: Testes de `decide`**: Claude saudável com e sem terminal → `Rust` em todas as rotas; `healthy=false` → `Python`; `Codex` → `Python`; `Keys`/`TermInput` sem terminal → `Python` (o Python responde como hoje).
- [ ] **Step 2: Testes de rota** (`session_write_routes.rs`): sem runtime aberto, `POST /api/sessions/s/input` chega ao Python falso com o corpo idêntico (`last_hit`); corpo `{"text":"oi","x":1}` com entrada aberta também chega ao Python intacto; pedido de convidado chega ao Python.
- [ ] **Step 3: Implementar `table.rs`, `mod.rs`** e registrar no `router` (`routes.rs:211`), no padrão das rotas de mods:

```rust
.route("/api/sessions/{name}/input", axum::routing::post(crate::session_write::input::input).fallback(pass_any))
```

uma linha por rota (o `DELETE …/queue/{entry_id}` com `axum::routing::delete`). Em
`migration_status::rust_route` (`:98`) as mesmas rotas entram como do Rust; os `PROBES` já as
listam.
- [ ] **Step 4: Commit** — `feat(server): write-route dispatch table with intact-body passthrough`.

### Task 4: `/input` e `/steer`

**Files:**
- Create: `crates/hangar-server/src/session_write/input.rs`
- Test: `crates/hangar-server/tests/session_write_input.rs`; golden em `backend/tests/fixtures/contract/session_write/` gerado por `gen_golden.py`

**Interfaces:**
- Consumes: `admit`, `detail`, `Ctx` (Task 3); `EntryHandle::command`/`control` (gateway).
- Produces: handlers `input`, `steer`.

Comportamento (fonte: `api.py:4849-4908`, `:4910-4956`, `_send_managed` `:4700-4745`,
`RuntimeAdapter.dispatch` `runtime_adapter.py:826-866`):

- **`/input` texto começando por `/clear` em sessão com terminal → repasse ao Python** (o
  Python esvazia a fila sob `freeze`, `runtime_coordinator.py:1255-1279`). É decisão inicial, antes
  de executar.
- Senão: `operation_id` = uuid hex; comando `submit {text}` no `handle`. Mapeamento da resposta
  igual ao `_send_managed`:
  - `unknown` em mensagem que entra na fila (texto sem `/`) com terminal → `{"ok":true,"delivered":false,"steered":false,"native":false}` e diário `runtime.send_uncertain` com o código.
  - `rejected`/`unknown` fora disso → 400 `{"detail":{"code":"erro_envio_falhou",...}}`.
  - `deferred` de comando de barra com terminal → 400 `erro_comando_nao_executado` com `comando` e `motivo` (o texto exato do Python).
  - `accepted`/`deferred` → `delivered = disposition == "accepted"`, `native = payload.native == true`.
- `steer:true` sem terminal e não entregue → `control steer_queue {entry_id: operation_id}`;
  `steered = operation_id ∈ payload.ids`. Com terminal o Claude não promove (só o Kimi): `steered:false`.
- **`/steer`** sem terminal: com corpo → `control steer {text}` → `{"ok":true,"promoted":false}`;
  sem corpo → `control steer_queue {}` → `{"ok":true,"promoted":false,"confirmed":n,"queued_ids":["queued-<id>",…]}`;
  `rejected` → 409 `erro_sem_turno`. Com terminal: `control steer {}` + `confirm` →
  `{"ok":true,"promoted":…,"confirmed":…}` como em `api.py:4950-4954`.

- [ ] **Step 1: Golden.** Em `gen_golden.py`, gerar os pares (resposta do ator → corpo HTTP) chamando `_send_managed` e `steer_session` com um coordenador falso que devolve cada disposição acima. Salvar em `session_write/input.json` e `steer.json`.
- [ ] **Step 2: Testes Rust** lendo o golden: para cada caso, um ator falso (o `FakeLink`/runtime de teste de `tests/mods_routes.rs` serve de modelo) devolve a disposição e a rota responde o corpo e o código do golden. Mais: `/clear` com terminal chega ao Python falso; `/steer` recusado → 409 `erro_sem_turno`.
- [ ] **Step 3: Implementar `input.rs`.**
- [ ] **Step 4: Commit** — `feat(server): serve Claude /input and /steer in Rust`.

### Task 5: `/interrupt`, `/keys`, `/term-input`, `/select`, `/select/submit` e descarte da fila

**Files:**
- Create: `crates/hangar-server/src/session_write/control.rs`
- Modify: `backend/app/internal_api.py` (rota temporária `/internal/sessions/{name}/plugin`)
- Test: `crates/hangar-server/tests/session_write_control.rs`, `backend/tests/test_internal_plugin_bridge.py`

**Interfaces:**
- Consumes: Task 3.
- Produces: handlers `interrupt`, `keys`, `term_input`, `select`, `select_submit`, `queue_remove`; `GET /internal/sessions/{name}/plugin` → `{"pending": pergunta_pendente(name)}`; `POST /internal/sessions/{name}/plugin` com `{"interrupted": id|null}` → chama `plugin_bridge.interrompeu(name, id)`. As duas saem na C2 (plugin no Rust).

Comportamento (fonte: `api.py:6110-6129`, `:6463-6481`, `:5935-6033`, `:3523-3530`):

- **`/interrupt?clear=`**: sem terminal → `control interrupt`; `payload.interrupted == false` → 409 `erro_sem_turno` ("Não há turno ativo para interromper."). Com terminal → `GET plugin` (id pendente), `control interrupt {clear}`, `POST plugin {interrupted:id}`. `{"ok":true}`.
- **`/keys`**: `control navigation_key {key}`; tecla fora da lista (o ator recusa) → 400 com o texto do `ValueError` do Python (golden).
- **`/term-input`**: `text` → `control terminal_input {text}`, `key` → `control interactive_key {key}`, nessa ordem, como `api.py:6477-6480`.
- **`/select`** com terminal: painel aberto (`st.term.is_active(name)`) e pendente não `perm:` → 409 do `_recusa_se_painel_aberto`; `perm:` e opção ∉ {1,2} → 409 `erro_opcao_nao_convergiu`; `control select {option, request_id?, require_cursor?}`; resultado incerto → 409 `erro_sem_confirmacao_resposta`; recusa antes da entrega → 503 `erro_opcao_nao_convergiu` (textos de `api.py:5955-5965`). Sem terminal → `control select {option}`; recusado → 409 "nenhum pedido de permissão pendente".
- **`/select/submit`**: painel aberto → 409; `control submit_selected {}`; recusa → 409 `erro_opcao_nao_convergiu` "não consegui enviar as opções marcadas — tente de novo".
- **`DELETE …/queue/{id}`**: `queue {action:{kind:"remove",entry_id}}`; nada removido → 404 `erro_fila_entrada_nao_encontrada`. Conferir antes: o `PromptQueue.remove` do Python (`pqueue.py:919`) não é `@_queue_method` e lê o arquivo do Python, não a fila do Rust — se a sessão gerenciada nunca acha a entrada hoje, é defeito atual que esta rota corrige; registrar no commit.

- [ ] **Step 1: Golden** dos corpos de erro de cada ramo acima, gerados chamando as rotas Python com dublês.
- [ ] **Step 2: Testes Rust** por ramo, incluindo o do Review Focus: permissão segurada + opção 3 → 409 e o ator falso não recebe nenhum `control`.
- [ ] **Step 3: Teste Python** da rota temporária (token interno exigido, 404 sem ele).
- [ ] **Step 4: Implementar.**
- [ ] **Step 5: Commit** — `feat(server): serve Claude interrupt, keys, select and queue discard in Rust`.

### Task 6: `/answer`

**Files:**
- Create: `crates/hangar-server/src/session_write/answer.rs`
- Test: `crates/hangar-server/tests/session_write_answer.rs`; golden `session_write/answer.json` e `askq_chat_text.json`

**Interfaces:**
- Consumes: Tasks 3 e 5 (`GET plugin`); leitor do sidecar do AskUserQuestion já portado em `crates/hangar-server/src/state/ask.rs`.
- Produces: handler `answer`; `fn chat_text(answers:&[Value], questions:&[String]) -> String` (porta de `_askq_conversar_text`, `api.py:9100-9135`).

Comportamento (fonte: `api.py:9187-9240`, `runtime_terminal.py:1044-1090`):

- Validação das respostas como `terminal_input._validate` → 409 `erro_sem_resposta`.
- **Com terminal:** pendente do plugin (`GET plugin`); `request_id` que não bate com o pendente → 409 "a pergunta mudou; resposta conservada". Painel aberto sem pendente → 409.
  - Alguma resposta `kind=chat` e nada pendente: `chat_text` sobre o sidecar; vazio → 409 "resposta sem texto para conversar"; senão `control interrupt {}`, esperar o rodapé sumir (regex que o Rust já tem, `terminal_state.rs:126`; até 3 s, captura a cada 0,1 s pelo pool) e `submit {text}`; disposição fora de `accepted`/`deferred` → 409 "a pergunta foi fechada, mas a resposta por texto não foi confirmada". **Sem empréstimo de teclado.**
  - Senão `control answer_questions {answers, request_id}`.
  - Sucesso: apagar o sidecar do AskUserQuestion (`clear_pending_askq`, `askquestion.py:149`, idempotente) e `{"ok":true,"fallback":false}`.
- **Sem terminal:** `control answer_questions {request_id, answers}`; recusa → 409 `erro_codex_resposta_invalida`; incerto → 503 `erro_codex_resposta_envio` (textos de `api.py:9225-9236`).

- [ ] **Step 1: Golden** de `_askq_conversar_text` (respostas variadas, com e sem perguntas no sidecar, uma e várias `chat`) e dos corpos de erro.
- [ ] **Step 2: Testes Rust**: `chat_text` contra o golden; rota com chat manda `interrupt` e depois `submit` (ordem conferida no ator falso) e nunca `keyboard_loan`.
- [ ] **Step 3: Implementar.**
- [ ] **Step 4: Commit** — `feat(server): serve Claude /answer in Rust, chat answer without keyboard loan`.

### Task 7: O Python pergunta ao Rust de quais provedores ele é dono

**Files:**
- Modify: `crates/hangar-server/src/routes.rs:248-256` (`health`), `backend/app/runtime_coordinator.py:515-517,635-641,679,1026,1248-1250`, `backend/app/rust_server.py` (leitura da saúde)
- Test: `backend/tests/test_runtime_coordinator_owns.py` (novo); `crates/hangar-server/tests/proxy.rs` (campo novo na saúde)

**Interfaces:**
- Consumes: `table::decide` (Task 3).
- Produces: `/__hangar_server/health` ganha `"owns":[{"provider":"claude","headless":true},{"provider":"claude","headless":false}]`, montado a partir da tabela; `RuntimeCoordinator.rust_owns(provider:str, headless:bool) -> bool`, lido do retrato da saúde guardado no `_enter_rust`. `_born_in_rust` passa a ser `self.transport is not None and self.rust_owns(binding.provider, binding.headless)`; os quatro "Codex é sempre do Python" usam `rust_owns`.

- [ ] **Step 1: Testes**: com `owns` só Claude, o comportamento de hoje (Codex no Python); com `owns` incluindo Codex sem terminal, `_born_in_rust` é verdadeiro para ele. Saúde sem `owns` (Rust antigo) é recusada pelo protocolo, então não há caso de compatibilidade.
- [ ] **Step 2: Implementar.**
- [ ] **Step 3: Commit** — `refactor(runtime): ask the Rust server which providers it owns`.

### Task 8: Políticas puras no Rust

**Files:**
- Create: `crates/hangar-server/src/runtime/local_policy.rs`
- Modify: `crates/hangar-server/src/runtime/actor.rs:547-575,1114`, `backend/app/runtime_policy.py:159-208,241-253`, `backend/app/internal_api.py` (`GET /internal/quota`)
- Test: `crates/hangar-server/tests/contract_local_policy.rs`; golden `backend/tests/fixtures/contract/local_policy/*.json`

**Interfaces:**
- Produces: `pub fn run(kind:&str, payload:&Value, meta:&Value, quota:Option<&Value>) -> Option<Result<Value,RuntimeError>>` (`None` = não é local). `GET /internal/quota?config_dir=<abs>` → `{"windows":[…]}` (`runtime_policy._quota`, só janelas sem `por_modelo`).

Regras portadas (paridade por golden):

- **`prepare_prompt`** Claude: `_blocos_do_prompt` (`claude_headless/adapter.py:2205-2243`: regex `📎\s*imagem:\s*(.+?)(?=\s*📎|$)` multilinha, candidatos trecho inteiro e primeira palavra sem `.,;:)` no fim, MIME por sufixo, teto 5 MiB, avisos com o texto exato) + `native_candidate` = casa `^\[(de|grupo|painel):\s*([^\]]+)\]\s*` (`uds_messaging.py:35`). Codex: `{"input":[{"type":"text","text":t}],"skill_name":…}` (`runtime_policy.py:168-169`). Leitura de imagem em `spawn_blocking`.
- **`format_status`** Claude: `status_line` (`adapter.py:1908-1933`), `_rotulo_modelo` (`:2391-2405`), `_esforco_padrao` (`:2429-2440`: env `CLAUDE_CODE_EFFORT_LEVEL`, depois `effortLevel` do `settings.json` da conta), `_fmt_tok` e `_format_reset` (`codex/adapter.py:242-283`), `limit_reset` = `HH:MM` local quando `rate_limit_info.status == "rejected"`. Codex: `format_status_line` (`codex/adapter.py:319-345`, janelas 270–330 e 10020–10140 min) com os campos que o motor Codex manda, inclusive `model || default_model` e `effort || default_effort` (contrato do par, v3).
- **Cota do Claude:** o ator pede `GET /internal/quota` só quando vai formatar e o cache dele (por `config_dir`, 300 s, teto 16 contas) venceu. É no máximo uma chamada a cada 5 min por conta, fora do caminho de escrita. A spec e o contrato do par já trazem esta forma (pedido com cache), que substituiu "fato empurrado pelo Python": faz o mesmo com menos código.
- **`skill_catalog`**: `skills_do_catalogo` (`codex/chat_controls.py:5-19`: habilitadas com nome e caminho, homônimas ganham `:<sha256(path)[:8]>`, ordenadas por nome).
- No ator (`actor.rs:547`), antes do `PolicyClient`: `if let Some(result) = local_policy::run(...)` roda num `spawn_blocking` e volta como o mesmo `Job::Policy`/`Job::PreparedInput` de hoje. `prepare_prompt` (`actor.rs:822`) usa o mesmo caminho.
- Saem do `runtime_policy.run`: `prepare_prompt`, `format_status`, `skill_catalog`, `answer_body`, `quota`, `session.marker`, `diag.error`; `COSMETIC_POLICIES` (`actor.rs:1114`) perde `format_status` e `quota`.

- [ ] **Step 1: Golden** gerado chamando as funções Python: textos com 0, 1 e 2 imagens (arquivo real pequeno, inexistente, acima de 5 MiB), prefixos `[de:`/`[grupo:`/`[painel:`; status com cada combinação de modelo (família conhecida, `[1m]`, motor com `/`), esforço da sessão/env/settings, uso, custo, janelas e `rejected`; catálogo com homônimas.
- [ ] **Step 2: Testes Rust** contra o golden, com relógio e fuso fixos (`TZ=UTC` no teste; `_hora_local` usa a hora local).
- [ ] **Step 3: Implementar** `local_policy.rs` e o desvio no ator.
- [ ] **Step 4: Teste Python** de `/internal/quota` e da remoção dos ramos (`run("answer_body", …)` levanta `ValueError`).
- [ ] **Step 5: Commit** — `feat(runtime): run prompt preparation, status line and skill catalog in Rust`.

### Task 9: Documentação e medição

**Files:**
- Modify: `CLAUDE.md` (regra do `hangar-server`: o Rust atende as escritas de Claude), `docs/decisoes/plataforma.md` (entrada nova "Escritas do Claude no hangar-server" com a medição), `docs/migracao-rust/README.md` ("Ainda no Python", tabela das partes)
- Create: `docs/migracao-rust/parte5-claude/medicao-5-0.md`

- [ ] **Step 1: Medição, só se o dono autorizar backend isolado** (`scripts/prova-dono-unico.py`, classe `Prova`: `HOME` temporário, portas próprias, `tmux -L` próprio, `matar_orfaos` desligado; Claude só Haiku em `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`). Sem autorização, a Task registra o roteiro e deixa os números para o uso real. Medir antes e depois, release: chamadas a `/internal/*` por minuto com 5 sessões Claude (3 com terminal, 2 sem) recebendo uma mensagem a cada 10 s; latência do `POST /input` até a resposta; CPU do Python parado.
- [ ] **Step 2: Docs** — cada regra que a 5-0 torna falsa, corrigida no mesmo commit.
- [ ] **Step 3: Commit** — `docs(migracao-rust): Claude write path in Rust (5-0)`.
- [ ] **Step 4: Avisar** a sessão `hangar` que a branch está pronta para o canal de testes, com o roteiro: mandar (com e sem terminal), interromper, responder pergunta por opção e por chat, permissão segurada no celular, renomear durante um envio, `/clear`.

## Ordem e paralelismo

- Task 1 primeiro. Depois **2, 3 e 8 em paralelo** (não dividem arquivo).
- Tasks 4, 5 e 6 depois da 3; podem correr em paralelo (arquivos próprios; só a linha do
  `router` conflita, resolvida na junção).
- Task 7 depois da 3. Task 9 por último.
