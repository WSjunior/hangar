# Parte 5B: Codex sem terminal no Rust — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** com o Rust de pé, a sessão Codex sem terminal é do Rust do nascimento ao fim: o Rust sobe e mata o processo (`hangar-cano` + `codex app-server --stdio`), religa, reinicia, troca permissão, atende as rotas de escrita e as só do Codex, responde todos os pedidos do servidor e publica estado; o Python só calcula o ambiente do processo e continua dono da criação e da exclusão da sessão (parte 6).

**Architecture:** um módulo comum de processo do cano em Rust (`runtime/process.rs`), desenhado para os dois provedores e usado já pelo Codex; o lado Claude pluga depois (metade Claude). O Python entrega o ambiente por uma política nova (`launch_env`); o Rust monta o argv do Codex a partir do arquivo da sessão. O motor (`runtime/codex.rs`, tipado na 5A) ganha a subida com as reservas do Python, os pedidos do servidor que faltavam e o ciclo de vida. A tabela de despacho da 5-0 passa o Codex sem terminal para o Rust.

**Tech Stack:** Rust 2024 (tokio, `hangar-codex`), Python 3.14 (coordenador, política), Svelte/Expo/GPUI só para link clicável e o Cancelar do nativo.

**Spec:** [`spec.md`](spec.md), seção 5B. **Análise:** [`analise.md`](analise.md). **Herança:** [`pendencias-5b.md`](pendencias-5b.md). **Contrato com a metade Claude:** [`../parte5-claude/contrato-par.md`](../parte5-claude/contrato-par.md). **Decisão do dono (07/10, pela metade Claude):** a subida do processo vai para o Rust na 5B, num módulo comum.

## Global Constraints

- **Base:** `main` com o PR #107 (5A) e o PR da 5-0 juntados. Branch nova a partir dela; o nome e a criação são combinados com o dono antes da Task 1. Linhas citadas aqui foram lidas em `ecf34430c` (5-0) e `e8193f1c0` (5A): cada Task reconfere pelo símbolo antes de editar.
- **Dono único:** com o Rust de pé, falha numa operação migrada vira erro com código e motivo (diário + `hangar-server.log`), nunca passagem da sessão ao Python. O código Python do Codex fica para o modo `python` (Rust ausente) e sai na parte 7.
- **Contrato interno:** sobe uma vez nesta branch (política `launch_env`, campos novos do `open`, evento novo se houver), no próximo número livre na junção (`RUST_SERVER_PROTOCOL` em `backend/app/rust_server.py` e `INTERNAL_PROTOCOL` em `crates/hangar-server/src/lib.rs`, juntos, mais o teste fixo `crates/hangar-server/tests/proxy.rs`).
- **Ambiente do processo é segredo:** o `env` de `launch_env` (tokens do Jev, `CODEX_HOME` de conta) nunca é gravado em disco nem em log; o Rust o pede ao Python a cada subida.
- **Paridade de formato:** toda rota que o Rust passa a atender devolve o mesmo corpo e código do Python, provado por fixture gerada pela rota Python (`backend/tests/fixtures/contract/`).
- **Desempenho (README, "erros que já custaram"):** nada de laço; religar por evento (saída do cano) com teto de 3 subidas seguidas e espera 5 s dobrando; canais com limite; processo filho só quando a entrada muda; `fsync` fora de trava.
- **Windows:** sem varredura de órfãos (não há `/proc`); `CREATE_NEW_PROCESS_GROUP | CREATE_NO_WINDOW`; kill por `taskkill /T /F /PID` aceitando código 0 e 128; caminho do `codex` resolvido (PATHEXT). Ler "Regras vigentes" de `docs/decisoes/windows.md` antes das Tasks 3 e 5.
- **Harnesses:** ler "Regras vigentes" de `docs/decisoes/harnesses.md` antes de cada Task que mexe no motor, no ator ou no coordenador.
- Testes de cada Task: escritos primeiro, vistos falhar, só os dos arquivos tocados. No máximo 2 `cargo` na máquina, `CARGO_BUILD_JOBS=4`, `target/` da worktree.
- Texto de tela por `m.<chave>()`, `pt.json` e `en.json` juntos. Comentário curto, sobre o porquê, em português com acento.
- Commit por caminho, sem push; o push é do controlador.

## Review Focus

1. **Rust reinicia com sessão Codex sem terminal trabalhando:** a sessão volta pelo cano vivo (sem subir outro processo), o turno em curso continua e a fila não duplica. Teste na Task 4.
2. **O app-server morre no meio do turno:** a sessão mostra `codex_turno_cortado`/`headless_caiu`, religa sozinha (teto 3, espera crescente) e nunca sobe dois processos para a mesma sessão. Teste na Task 5.
3. **Troca de permissão com turno rodando ou pedido pendente:** recusa 409 `erro_permissao_ocupada` sem matar nada; com a sessão parada, troca o sandbox e a conversa continua na mesma thread. Teste na Task 5.
4. **Pedido de permissão/formulário MCP de um subagente (outra thread):** aparece e é respondido, não é descartado. Teste na Task 2.
5. **Backend Python cai e volta (Rust de pé):** a sessão Codex não é tocada pelo Python (sem `watch_sessions` abrindo cliente Python); nada é morto como órfão. Teste na Task 4.

---

## Desenho do módulo comum de processo (`crates/hangar-server/src/runtime/process.rs`)

Conferido pela metade Claude (07/10, 11 pontos incorporados abaixo).

```rust
/// O que o Python entrega pela política `launch_env` (os dois provedores).
pub struct LaunchSpec {
    pub provider: Provider,          // Claude | Codex (o da tabela da 5-0)
    pub key: String,                 // chave da sessão (16 primeiros chars no nome do socket/log)
    pub cwd: PathBuf,
    pub program: Vec<String>,        // comando PRONTO dentro do cano (Codex: app-server com os -c do modo;
                                     // Claude: hangar-engine --exec … claude -p …, .CMD no Windows)
    pub env: Vec<(String, String)>,  // ambiente COMPLETO (TMUX removido, CP_*/HANGAR_CANO_* postos)
    pub cano_extra: Map<String, Value>, // campos que vão dentro de `cano` além dos do Rust (Claude: config_marca)
    pub sidecar_dir: PathBuf,        // pasta do arquivo da sessão (socket e log moram nela)
}

pub struct Cano { pub pid: u32, pub escuta: String, pub token: String, pub ts: f64, pub versao: u32,
                  pub extra: Map<String, Value> }

pub enum Liveness { Dead, Ours, Foreign }   // pelo pid E pela identidade (`--log …cano-<key16>.log`)
pub fn liveness(pid: u32, key: &str) -> Liveness;

pub async fn spawn(spec: &LaunchSpec) -> Result<Cano, ProcessError>;   // não grava nada no arquivo da sessão
pub async fn kill(cano: &Cano, key: &str) -> Result<(), ProcessError>;  // só se `liveness == Ours`
pub fn kill_orphans(live: &HashSet<String>) -> usize;                    // uma vez, ao subir o Rust
pub fn cano_binary() -> Result<PathBuf, ProcessError>;                  // sonda: sem args sai com 2
```

Regras do módulo (cada uma de um ponto da conferência):

1. **Nunca dois processos para a mesma sessão.** Antes de subir: `liveness(cano.pid)` — `Ours` → conecta e nunca sobe outro (mesmo com `versao` ausente: o `cano.py`/`hangar-cano` vivos já falam a versão 2); `Foreign` ou `Dead` → pode subir. Vale para a subida da Task 4 e a religação da Task 5.
2. **Quem grava o arquivo da sessão é o Python** (`session.patch_meta`, já existente; o catálogo `_PATCH` ganha `cano` para os dois provedores). O sidecar do Claude não tem trava de arquivo e o Python grava nele em vida; com o Rust gravando direto, gravações cruzadas se perdiam. Até a parte 6, o Rust só pede.
3. **Apagar o `cano`** é a política `session.clear_cano {pid}`: só zera se o `cano.pid` gravado ainda for o mesmo, e nunca recria arquivo de sessão apagado (`sessions.update` devolve `None`).
4. **`cano_extra`** vem do `launch_env` e é gravado dentro de `cano` junto com `pid/escuta/token/ts/versao` (Claude: `config_marca`, que o `reload_stamp` lê).
5. **Subiu e não escutou em 10 s** → `kill` + `session.clear_cano` + `NotListening` (como o `_subir_cano` do Claude). `open` que falha por erro de conexão descarta só o cano subido nesta chamada (`_CONNECT_CODES` do coordenador).
6. **Teto esgotado não sobrescreve o problema** que a última subida deixou.
7. **O comando é sempre do Python** (`launch_env.program`): o Rust não monta argv de nenhum provedor.
8. `launch_env` pode gravar a `key` se faltar (comportamento atual do Claude); para o Codex a `key` já existe desde a criação.
9. **Binário do cano:** `CP_RUST_CANO_BIN`, senão a pasta do executável do `hangar-server`, senão `~/.hangar/bin/`; sondado uma vez (sem args sai com 2). Ausente → `cano_ausente` (com o Rust de pé não há `cano.py` de reserva: a reserva é o Python inteiro).
10. **Órfãos têm um dono só:** com o Rust de pé (modo `rust`/`pending`), a varredura é do Rust, uma vez ao subir, nunca a cada religada; o `matar_orfaos` do Python só roda no modo `python`. Mantida a reserva do `HOME` para cano antigo sem `HANGAR_CANO_OWNER`. Conjunto vivo: chaves de `~/.hangar/claude-headless/` e de `~/.hangar/codex-sessions/` (headless).
11. **`Cano.pid` é o pid do próprio cano:** `systemd-run --user --scope --collect -q --` faz `exec` (nunca `--unit`), e o Python lê esse pid em vários lugares (`runtime_coordinator.py:149`, `registry.py:271-278`, `api.py:3206`, …).

Fica no Python até a parte 6: o argv do Claude (`--resume`/`--session-id`, plugin-dir, `model_args`), `engine_env`/cliproxy, o ambiente e `_marca_config`.

---

### Task 1: Motor — subida igual à do Python e códigos que o app espera

**Files:**
- Modify: `crates/hangar-server/src/runtime/codex.rs` (bootstrap: `bootstrap`, continuações `bootstrap`/`bootstrap_thread`/`bootstrap_ready`; `Select`)
- Modify: `crates/hangar-codex/src/proto.rs` (`ThreadResumeParams`, `ThreadStartParams` com campos usados)
- Modify: `crates/hangar-codex/src/version.rs`, `crates/hangar-codex/schema/` (versão conferida)
- Test: `crates/hangar-server/tests/runtime_codex.rs`

**Interfaces:**
- Produces: problema `codex_esforco_nao_aplicado`; código `no_pending_permission` no `Select` sem aprovação; versão conferida 0.161.

- [x] **Step 1: Conferir a 0.161**

Rodar `scripts/conferir-codex-schema` com o Codex desta máquina (0.161.0). O script recusa por versão: trocar `checked_version!()` para `"0.161.0"`, rodar de novo, olhar o `git status --short` do schema. Campo usado que sumiu → corrigir o tipo em `proto.rs` (nunca o recorte). Apagar o recorte 0.159.3. Commit: `chore(codex): check protocol against codex 0.161.0`.

- [x] **Step 2: Testes da subida (falham)**

Em `tests/runtime_codex.rs`:

```rust
fn bootstrapped(meta:serde_json::Value) -> (Engine,Vec<Value>) {
    let mut engine = Engine::new(meta,1,clock(10.0));
    let effects = engine.bootstrap(true,"boot".into()).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    let effects = line(&mut engine,json!({"id":id,"result":{"userAgent":format!("hangar/{} (x)",hangar_codex::version::CHECKED)}}),11.0);
    (engine,frames(&effects))
}

#[test]
fn resume_carries_cwd_policy_sandbox_and_tier_like_python() {
    let (_,frames) = bootstrapped(json!({"name":"s","thread_id":"t1","headless":true,"cwd":"/p",
        "permission_mode":"Ask for approval","service_tier":"priority"}));
    let resume = frames.iter().find(|f|f["method"] == "thread/resume").unwrap();
    assert_eq!(resume["params"],json!({"threadId":"t1","cwd":"/p","approvalPolicy":"on-request","sandbox":"read-only","serviceTier":"priority"}));
}

#[test]
fn missing_model_provider_retries_resume_with_openai() {
    let (mut engine,frames) = bootstrapped(json!({"name":"s","thread_id":"t1","headless":true,"cwd":"/p"}));
    let resume = frames.iter().find(|f|f["method"] == "thread/resume").unwrap().clone();
    let effects = line(&mut engine,json!({"id":resume["id"],"error":{"code":-32600,"message":"Model provider `x` not found"}}),12.0);
    let retry = frames_of(&effects).into_iter().find(|f|f["method"] == "thread/resume").unwrap();
    assert_eq!(retry["params"]["modelProvider"],"openai");
}

#[test]
fn no_rollout_falls_back_to_thread_start() {
    let (mut engine,frames) = bootstrapped(json!({"name":"s","thread_id":"t1","headless":true,"cwd":"/p","model":"gpt-6"}));
    let resume = frames.iter().find(|f|f["method"] == "thread/resume").unwrap().clone();
    let effects = line(&mut engine,json!({"id":resume["id"],"error":{"code":-32600,"message":"no rollout found for thread id t1"}}),12.0);
    assert!(frames_of(&effects).iter().any(|f|f["method"] == "thread/start" && f["params"]["model"] == "gpt-6"));
}

#[test]
fn refused_effort_is_a_problem_not_a_failure() {
    let (mut engine,frames) = bootstrapped(json!({"name":"s","thread_id":"t1","headless":true,"cwd":"/p","effort":"max"}));
    let resume = frames.iter().find(|f|f["method"] == "thread/resume").unwrap().clone();
    let effects = line(&mut engine,json!({"id":resume["id"],"result":{"thread":{"id":"t1","status":{"type":"idle"},"turns":[]},"model":"gpt-6"}}),12.0);
    let update = frames_of(&effects).into_iter().find(|f|f["method"] == "thread/settings/update").unwrap();
    line(&mut engine,json!({"id":update["id"],"error":{"code":-32600,"message":"effort max not supported"}}),13.0);
    assert_eq!(engine.view()["problema"],"codex_esforco_nao_aplicado");
    assert_eq!(engine.control_view()["ready"],true);
}

#[test]
fn select_without_pending_approval_says_no_pending_permission() {
    let mut engine = engine();
    let error = engine.command(command(OperationKind::Select,json!({"option":1})),clock(10.0)).unwrap_err();
    assert_eq!(error.code,"no_pending_permission");
}
```

(`frames_of` = o `frames` já existente aplicado a `&[Effect]`; renomear se colidir.)

- [x] **Step 3: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex`
Expected: FAIL nos 5 testes novos.

- [x] **Step 4: Implementar**

- `thread/resume` da subida leva `cwd`, `approvalPolicy`, `sandbox` e `serviceTier` (quando houver), como `adapter.py:_subir_sem_terminal`; a verificação do Fast (`thread/resume` só com `threadId`) não muda.
- Erro do `thread/resume` da subida: mensagem com "Model provider" e "not found" → repete com `modelProvider: "openai"` (uma vez); com "no rollout found" → `thread/start` com `cwd`, `approvalPolicy`, `sandbox`, `serviceTier` e `model` (só se o arquivo da sessão tiver modelo). Ambos só sem `transfer_id` no metadata. Os dois novos campos entram em `ThreadResumeParams` (`cwd`, `approval_policy`, `sandbox`, `service_tier`, `model_provider`) com `skip_serializing_if`, e o teste do recorte continua verde.
- Esforço recusado no `thread/settings/update` da subida: `state.problema = "codex_esforco_nao_aplicado"` com o detalhe (300 caracteres), e a subida termina pronta (`ready = true`, `WakeQueue`).
- `Select` sem aprovação pendente: `RuntimeError::new("no_pending_permission", "nenhuma aprovação pendente")`.
- Texto do problema: `messages/pt.json` `problema_codex_esforco_nao_aplicado`: "O Codex não aceitou o nível de esforço escolhido; a sessão segue no padrão do modelo", `en.json`: "Codex did not accept the chosen effort level; the session keeps the model default"; mapear em `frontend/src/lib/problema.ts` e `mobile/src/chat/SessionProblem.tsx` (se a chave ainda não existir).

- [x] **Step 5: Rodar e ver passar; commit**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex --test runtime_contract && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --lib`
Commit: `feat(codex): Rust bootstrap matches the Python fallbacks and error codes`.

---

### Task 2: Motor — todos os pedidos do servidor

**Files:**
- Modify: `crates/hangar-codex/src/proto.rs` (`PermissionsRequestApprovalParams`, `McpServerElicitationRequestParams`, respostas)
- Modify: `crates/hangar-server/src/runtime/codex.rs` (`notification` ramo de pedidos, `view`, `blocking_question`, `Select`, `AnswerQuestions`, `unsupported_notice`)
- Test: `crates/hangar-server/tests/runtime_codex.rs`

**Interfaces:**
- Consumes: tabela de pedidos da spec 5B.
- Produces: cartão de permissão com opções `["Permitir neste turno","Permitir na sessão","Negar"]`; pergunta nativa para formulário MCP (`provider:"codex"`, `request_id`, `questions`); cartão de link `["Concluí","Cancelar"]` com a URL no texto.

- [x] **Step 1: Tipos**

Em `proto.rs`, trocar `ThreadOnlyParams` por:

```rust
wire!(pub struct AdditionalFileSystemPermissions { pub read:Option<Vec<String>>, pub write:Option<Vec<String>> });
wire!(pub struct AdditionalNetworkPermissions { pub enabled:Option<bool> });
wire!(pub struct RequestPermissionProfile { pub file_system:Option<AdditionalFileSystemPermissions>, pub network:Option<AdditionalNetworkPermissions> });
wire!(pub struct PermissionsRequestApprovalParams { pub thread_id:String, pub cwd:Option<String>, pub reason:Option<String>, pub permissions:RequestPermissionProfile });
wire!(pub struct McpServerElicitationRequestParams { pub thread_id:String, pub server_name:String, pub mode:Option<String>,
    pub message:Option<String>, pub url:Option<String>, pub requested_schema:Option<serde_json::Value> });
```

Conferir com o schema 0.161 (`scripts/conferir-codex-schema`) os nomes `fileSystem`, `read`, `write`, `network.enabled`, `serverName`, `mode`, `message`, `url`, `requestedSchema`; o campo que divergir segue o schema. `LOCAL_NAMES` perde `ThreadOnlyParams`. `ServerRequest` ganha `CurrentTimeRead` (`"currentTime/read"`, params livres) e as variantes passam a carregar os tipos acima.

- [x] **Step 2: Testes (falham)**

```rust
fn request(engine:&mut Engine,id:i64,method:&str,params:Value) -> Vec<Effect> {
    line(engine,json!({"id":id,"method":method,"params":params}),10.0)
}
fn reply_to(effects:&[Effect],id:i64) -> Value { frames_of(effects).into_iter().find(|f|f["id"] == id && f.get("method").is_none()).unwrap() }

#[test]
fn permissions_request_is_a_card_and_answers_with_scope() {
    let mut engine = engine();
    request(&mut engine,5,"item/permissions/requestApproval",json!({"threadId":"thread-1","cwd":"/p","reason":"ler config",
        "permissions":{"fileSystem":{"read":["/etc/x"],"write":["/p/out"]},"network":{"enabled":true}}}));
    let view = engine.view();
    assert_eq!(view["state"],"awaiting_input");
    assert_eq!(view["options"],json!(["Permitir neste turno","Permitir na sessão","Negar"]));
    let text = view["question"].as_str().unwrap();
    assert!(text.contains("/etc/x") && text.contains("/p/out") && text.contains("rede"));
    let effects = engine.command(command(OperationKind::Select,json!({"option":2})),clock(11.0)).unwrap();
    let answer = reply_to(&effects,5);
    assert_eq!(answer["result"]["scope"],"session");
    assert_eq!(answer["result"]["permissions"]["fileSystem"]["read"],json!(["/etc/x"]));
}

#[test]
fn denied_permissions_grant_nothing() {
    let mut engine = engine();
    request(&mut engine,5,"item/permissions/requestApproval",json!({"threadId":"thread-1","permissions":{"network":{"enabled":true}}}));
    let effects = engine.command(command(OperationKind::Select,json!({"option":3})),clock(11.0)).unwrap();
    assert_eq!(reply_to(&effects,5)["result"],json!({"permissions":{},"scope":"turn"}));
}

#[test]
fn elicitation_form_becomes_a_native_question() {
    let mut engine = engine();
    request(&mut engine,6,"mcpServer/elicitation/request",json!({"threadId":"thread-1","serverName":"db","mode":"form","message":"Qual ambiente?",
        "requestedSchema":{"type":"object","properties":{"env":{"type":"string","enum":["dev","prod"],"title":"Ambiente"},"note":{"type":"string","title":"Nota"}},"required":["env"]}}));
    let question = &engine.view()["codex_question"];
    assert_eq!(question["request_id"],6);
    assert_eq!(question["questions"][0]["options"],json!([{"label":"dev","description":""},{"label":"prod","description":""}]));
    assert_eq!(question["questions"][1]["options"],json!([]));
    let effects = engine.command(command(OperationKind::AnswerQuestions,json!({"request_id":6,"answers":[
        {"question_id":"env","kind":"option","indices":[1]},{"question_id":"note","kind":"text","value":"urgente"}]})),clock(11.0)).unwrap();
    assert_eq!(reply_to(&effects,6)["result"],json!({"action":"accept","content":{"env":"prod","note":"urgente"}}));
}

#[test]
fn elicitation_schema_that_does_not_fit_is_declined_with_a_note() {
    let mut engine = engine();
    let effects = request(&mut engine,7,"mcpServer/elicitation/request",json!({"threadId":"thread-1","serverName":"db","mode":"form","message":"x",
        "requestedSchema":{"type":"object","properties":{"deep":{"type":"object"}}}}));
    assert_eq!(reply_to(&effects,7)["result"]["action"],"decline");
    assert!(effects.iter().any(|e|matches!(e,Effect::Policy { kind,.. } if kind == "local_output")));
}

#[test]
fn elicitation_url_is_a_link_card() {
    let mut engine = engine();
    request(&mut engine,8,"mcpServer/elicitation/request",json!({"threadId":"thread-1","serverName":"gh","mode":"url","message":"Autorize","url":"https://example.com/auth","elicitationId":"e1"}));
    let view = engine.view();
    assert!(view["question"].as_str().unwrap().contains("https://example.com/auth"));
    assert_eq!(view["options"],json!(["Concluí","Cancelar"]));
    let effects = engine.command(command(OperationKind::Select,json!({"option":2})),clock(11.0)).unwrap();
    assert_eq!(reply_to(&effects,8)["result"],json!({"action":"cancel"}));
}

#[test]
fn current_time_is_answered() {
    let mut engine = engine();
    let effects = request(&mut engine,9,"currentTime/read",json!({}));
    assert!(reply_to(&effects,9)["result"].is_object());
}

#[test]
fn request_from_a_subagent_thread_is_not_dropped() {
    let mut engine = engine();
    request(&mut engine,10,"item/commandExecution/requestApproval",json!({"threadId":"subagent-thread","command":"ls"}));
    assert_eq!(engine.view()["state"],"awaiting_input");
}
```

- [x] **Step 3: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex`
Expected: FAIL nos 7 testes novos.

- [x] **Step 4: Implementar**

- No `notification`, o ramo de pedidos (`line.get("id")`) passa para **antes** do desvio por outra thread: pedido de qualquer thread entra em `server_requests` (voz continua desviada antes, pela thread organizadora).
- `APPROVALS` (lista dos métodos que viram cartão) = comando, arquivo, permissões, formulário MCP em modo URL. `view()` monta o texto:
  - permissões: `"Permitir {leitura de A, B}{; escrita em C}{; rede}?{ motivo}"` (listas separadas por vírgula; "rede" só com `network.enabled == true`);
  - URL: `"{serverName} pede para abrir {url}: {message}"`, opções `["Concluí","Cancelar"]`.
- `Select` decide pelo método do pedido guardado:
  - comando/arquivo: como hoje (1/2/3 → `accept`/`decline`/`acceptForSession`);
  - permissões: 1 → `{"permissions": <o perfil pedido>, "scope":"turn"}`, 2 → idem com `"scope":"session"`, 3 → `{"permissions":{}, "scope":"turn"}`;
  - URL: 1 → `{"action":"accept"}`, 2 → `{"action":"cancel"}`.
- Formulário MCP: `blocking_question` também cobre `mcpServer/elicitation/request` com `mode != "url"`. Cada propriedade do `requestedSchema` vira uma pergunta (`id` = nome da propriedade, `header` = `title` ou o nome, `question` = `description` ou `message`): `enum` ou `oneOf` com `const` → opções; `boolean` → `["sim","não"]`; `string`/`number`/`integer` → sem opções (texto livre, `isOther: true`). Qualquer outra forma (objeto aninhado, `array`, schema ausente) → resposta imediata `{"action":"decline"}` + `local_output` "O servidor MCP `{serverName}` pediu um formulário que o Hangar não sabe mostrar; o pedido foi recusado." `AnswerQuestions` para esse pedido responde `{"action":"accept","content":{id: valor}}` (número convertido de volta para número; booleano de "sim"/"não").
- `currentTime/read` → `{"currentTime": <ISO-8601 UTC do relógio do motor>}` (conferir o nome do campo no schema 0.161; seguir o schema).
- Restam com `-32601` + nota: `item/tool/call`, `account/chatgptAuthTokens/refresh`, `attestation/generate`, legados v1, desconhecido.
- Cancelar o formulário MCP pelo app (`SkipQuestion` com o `request_id` de um formulário) → `{"action":"cancel"}`.
- Herdado da 5A (`pendencias-5b.md`): aprovação de comando com `command` vazio ou só espaços é tratada como ilegível (texto fixo, sem "Sempre permitir"); `turn/completed` lido cru com `status: "failed"` marca `headless_turno_erro` sem detalhe. Um teste para cada, no estilo dos de cima.

- [x] **Step 5: Rodar e ver passar; commit**

Run: o mesmo do Step 3 + `cargo test -p hangar-codex --lib`.
Commit: `feat(codex): answer permission and MCP form/link requests, keep subagent requests`.

---

### Task 3: Módulo comum de processo do cano

**Files:**
- Create: `crates/hangar-server/src/runtime/process.rs`
- Modify: `crates/hangar-server/src/runtime/mod.rs`
- Test: `crates/hangar-server/tests/runtime_process.rs`

**Interfaces:**
- Produces: `LaunchSpec`, `Cano`, `Liveness`, `ProcessError { NoCano, Spawn(String), NotListening, StillAlive }` com `code()` (`cano_ausente`, `cano_nao_subiu`, `cano_nao_escutou`, `cano_continua_vivo`), `liveness`, `spawn`, `kill`, `kill_orphans`, `cano_binary` — assinaturas e regras no desenho acima. O módulo não grava o arquivo da sessão.

- [x] **Step 1: Testes (falham)**

`tests/runtime_process.rs` (Linux; Windows compila e roda só o que não depende de `/proc`):

```rust
use hangar_server::runtime::process::*;
use std::collections::HashSet;

fn fake_cano(dir:&std::path::Path) -> std::path::PathBuf {
    // Cano falso: escuta o socket pedido e responde o snapshot, como o hangar-cano.
    let path = dir.join("fake-cano.sh");
    std::fs::write(&path,"#!/bin/sh\nexec \"$HANGAR_TEST_CANO\" \"$@\"\n").unwrap();
    std::os::unix::fs::PermissionsExt::set_mode(&mut std::fs::metadata(&path).unwrap().permissions(),0o755);
    path
}

#[tokio::test]
async fn spawn_listens_and_kill_ends_the_group() {
    let dir = tempfile::tempdir().unwrap();
    unsafe { std::env::set_var("CP_RUST_CANO_BIN",env!("CARGO_BIN_EXE_hangar-cano")); }
    let spec = LaunchSpec { provider:Provider::Codex, key:"0123456789abcdef0123".into(), cwd:dir.path().into(),
        program:vec!["/bin/sleep".into(),"300".into()], env:std::env::vars().collect(),
        cano_extra:serde_json::Map::from_iter([("config_marca".into(),serde_json::json!("m1"))]), sidecar_dir:dir.path().into() };
    let cano = spawn(&spec).await.unwrap();
    assert_eq!(cano.versao,2);
    assert_eq!(cano.extra["config_marca"],"m1");
    assert!(cano.escuta.starts_with("unix:"));
    assert!(matches!(liveness(cano.pid,&spec.key),Liveness::Ours));
    assert!(dir.path().join("cano-0123456789abcdef.log").exists());
    kill(&cano,&spec.key).await.unwrap();
    assert!(matches!(liveness(cano.pid,&spec.key),Liveness::Dead));
    assert!(!dir.path().join("cano-0123456789abcdef.log").exists());
}

#[test]
fn a_live_pid_of_another_program_is_foreign() {
    let mut other = std::process::Command::new("/bin/sleep").arg("300").spawn().unwrap();
    assert!(matches!(liveness(other.id(),"0123456789abcdef0123"),Liveness::Foreign));
    other.kill().unwrap();
}

#[tokio::test]
async fn kill_refuses_a_reused_pid() {
    let other = std::process::Command::new("/bin/sleep").arg("300").spawn().unwrap();
    let cano = Cano { pid:other.id(), escuta:"unix:/x".into(), token:"t".into(), ts:0.0, versao:2 };
    assert!(kill(&cano,"0123456789abcdef0123").await.is_ok());   // `Foreign`: não mata
    assert!(std::path::Path::new(&format!("/proc/{}",other.id())).exists());
    unsafe { libc::kill(other.id() as i32,libc::SIGKILL); }
}

#[test]
fn orphans_are_processes_with_a_dead_key_and_our_owner() {
    let mut child = std::process::Command::new("/bin/sleep").arg("300")
        .env("HANGAR_CANO_KEY","deadbeefdeadbeef").env("HANGAR_CANO_OWNER",std::env::var("HOME").unwrap()).spawn().unwrap();
    assert!(kill_orphans(&HashSet::from(["outra".to_string()])) >= 1);
    assert!(child.wait().is_ok());
}
```

(O teste de `spawn` usa o `hangar-cano` real do workspace como cano e `/bin/sleep` como programa; ajustar `CARGO_BIN_EXE_*` se o binário morar em outro crate — nesse caso apontar `CP_RUST_CANO_BIN` para `target/<perfil>/hangar-cano` montado pelo `cargo build -p hangar-cano` antes do teste.)

- [x] **Step 2: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo build -p hangar-cano && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_process`
Expected: FAIL de compilação (`process` não existe).

- [x] **Step 3: Implementar `process.rs`**

Seguir o desenho acima. Pontos que valem cada um uma regra vigente:
- `spawn` não devolve antes de o cano escutar: tenta conectar (`runtime::cano::connect`, já existente) por até 10 s; sem escuta → mata o que subiu e devolve `NotListening` (o ator zera o `cano` pela política).
- Identidade antes de matar: Linux lê `/proc/<pid>/cmdline` e exige `cano-<key16>.log`; Windows compara o caminho do executável com o binário do cano (via `sysinfo`, já dependência fora do Linux). Pid de outro dono → `Ok(())` sem matar e `warn` (o arquivo da sessão já não aponta para processo vivo).
- `kill` apaga `cano-<key16>*` (socket e log) depois de confirmar a saída; processo que não sai em 5 s → `StillAlive`, arquivos conservados.
- `kill_orphans` lê `/proc/*/environ` só dos processos do mesmo `uid`; manda `SIGTERM` por pid (não por grupo), como o Python; conta e loga os "alheios" (marca sem prova de dono).
- O módulo não grava o arquivo da sessão (regra 2 do desenho): quem grava é o ator, pela política `session.patch_meta {cano}` / `session.clear_cano {pid}` (Task 4).

- [x] **Step 4: Rodar e ver passar; commit**

Run: o do Step 2.
Commit: `feat(runtime): shared cano process module (spawn, kill with identity, orphans)`.

---

### Task 4: Nascer e religar no Rust (Python só calcula o ambiente)

**Files:**
- Modify: `backend/app/runtime_policy.py` (política `launch_env`), `backend/app/internal_api.py` (se a rota de política filtrar por tipo)
- Modify: `backend/app/runtime_coordinator.py` (`ensure_open`, `prepare_session`, `_launch_and_open`, `_reopen`, `start_sessions`: genéricos por provedor; Codex sem terminal nasce no Rust)
- Modify: `backend/app/runtime_adapter.py` (embrulhos `ensure_running`/`acordar` também para o Codex sem terminal), `backend/app/adapters/codex/adapter.py` (`watch_sessions`/`warm_sessions` pulam sessão do Rust), `backend/app/api.py` (`_aquecer_codex_sem_terminal` → `coordinator.ensure_open`)
- Modify: `crates/hangar-server/src/runtime/gateway.rs` (`open` com `launch`), `actor.rs` (subida/religação), `codex.rs` (argv do Codex)
- Modify: `crates/hangar-server/src/session_write/table.rs` (Codex sem terminal → Rust)
- Modify: `backend/app/rust_server.py`, `crates/hangar-server/src/lib.rs`, `crates/hangar-server/tests/proxy.rs` (contrato)
- Test: `backend/tests/test_runtime_routing.py`, `backend/tests/test_runtime_policy.py` (ou o arquivo dos testes de política), `crates/hangar-server/tests/runtime_actor.rs`, `crates/hangar-server/src/session_write/table.rs` (testes da tabela)

**Interfaces:**
- Consumes: `process::{spawn,LaunchSpec}` (Task 3); motor da Task 1.
- Produces: política `launch_env` → `{"program": [...], "env": {k: v}, "cano_extra": {...}}` (os dois provedores; Codex: `sem_terminal.argv(meta)` com o caminho resolvido do `codex`); políticas `session.patch_meta {cano}` e `session.clear_cano {pid}`; descriptor do `open` aceita `"launch": true` (sobe só se o processo gravado não for `Ours`); `table::decide(_, Provider::Codex, false, true) == Owner::Rust` para as rotas de escrita.

- [x] **Step 1: Testes (falham)**

Python, `test_runtime_policy`: `launch_env` para uma sessão Codex devolve `program` (app-server com os `-c` do modo, `codex` com caminho resolvido) e `env` com `CODEX_HOME` da conta, `CP_SESSION_KEY`, `HANGAR_CANO_KEY`, `HANGAR_CANO_OWNER`, sem `TMUX`/`TMUX_PANE`; `session.clear_cano` só zera com o pid igual e não recria arquivo apagado.

```python
def test_launch_env_da_sessao_codex(monkeypatch, tmp_path):
    monkeypatch.setenv("TMUX", "x")
    meta = {"name": "cx", "key": "k" * 32, "codex_account": "default", "jev": False, "cwd": str(tmp_path)}
    out = runtime_policy.run("launch_env", {}, {"provider": "codex", **meta})
    env = out["env"]
    assert env["CP_SESSION_KEY"] == env["HANGAR_CANO_KEY"] == "k" * 32
    assert env["HANGAR_CANO_OWNER"] == str(Path.home())
    assert "TMUX" not in env and "CODEX_HOME" in env
    assert Path(out["program"][0]).name.startswith("codex") and out["program"][1:3] == ["app-server", "--stdio"]
```

Python, `test_runtime_routing`: com transporte de pé e `owns` incluindo `("codex", True)`, `ensure_open("cx")` de uma sessão Codex sem terminal sem `cano` manda `open` com `launch: True` e não chama `sem_terminal.subir`; `warm_sessions` não chama `ensure_running` para ela.

Rust, `table.rs`: `codex_without_terminal_is_rust` (Input/Steer/Interrupt/Select/Answer/QueueRemove → Rust; Keys/TermInput/SelectSubmit → Python) e `codex_with_terminal_is_python` (até a 5C); `owned_modes` passa a incluir `(Codex, true)`.

Rust, `runtime_actor.rs`: `open` Codex com `launch: true` e sem cano → pede `launch_env` (fake de política devolve env + `codex` = script falso que fala JSON-RPC mínimo), sobe pelo `process::spawn`, grava `cano` no arquivo da sessão e o motor chega a `ready`. `open` com cano vivo → conecta, não sobe outro.

- [x] **Step 2: Rodar e ver falhar**

Run: `cd backend && uv run pytest tests/test_runtime_policy.py tests/test_runtime_routing.py` e `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --lib session_write && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_actor`.

- [x] **Step 3: Implementar**

- `launch_env` (Python, `runtime_policy.run`): Codex → `program = sem_terminal.argv(meta)` com `argv[0]` resolvido (`shutil.which`), `env = sem_terminal._ambiente(meta)`, `cano_extra = {}`; sem binário → erro `codex_ausente`. (O lado Claude pluga depois devolvendo o argv do adapter Claude e `cano_extra = {config_marca}`.)
- `session.patch_meta` aceita `cano` nos dois provedores; `session.clear_cano {pid}` zera só se o `cano.pid` gravado for o mesmo e nunca recria arquivo apagado.
- Rust, `gateway.rs`/`actor.rs`: `open` headless Codex com `launch: true` → `process::liveness(meta.cano.pid)`: `Ours` → conecta (mesmo sem `versao`); `Dead`/`Foreign`/sem cano → `launch_env` → `process::spawn(LaunchSpec{ provider:Codex, program, env, cano_extra, cwd, sidecar_dir })` → `session.patch_meta {cano}` → segue o `open` normal com o cano novo. `NotListening` → `session.clear_cano` e erro com código. Descriptor ganha `sidecar_dir`.
- Coordenador: os pontos que hoje só aceitam `"claude"` (`ensure_open`, `prepare_session`, `_release_python_slot`, `_launch_and_open`, `_reopen`, `_reopen_registered`, `_reopen_after_change`, `reopen_in_change`, `start_sessions`, embrulhos `ensure_running`/`acordar`) passam a usar o provedor da sessão; para Codex sem terminal, "lançar" = `open` com `launch: True` (o Python não sobe processo). `_release_python_slot` para Codex fecha o cliente Python se houver (`adapter._sessions`).
- `warm_sessions`/`watch_sessions`: pulam sessão cujo provedor+modo o Rust é dono (`coordinator.rust_owns("codex", True)` e sidecar `headless`).
- `start_sessions` roda antes do primeiro `configure_transport` com o `owns` padrão (só Claude): quando o `owns` chega e inclui o Codex, registrar as sessões Codex sem terminal no Rust (pendência da 5-0).
- Tabela: `Provider::Codex => if terminal { Owner::Python } else { <mesma regra do Claude sem terminal> }`.
- Contrato: subir para o próximo número livre.

- [x] **Step 4: Rodar e ver passar; commit**

Commit: `feat(codex): headless Codex is born and reconnected by Rust; Python only computes the environment`.

---

### Task 5: Ciclo de vida no Rust (religar, reiniciar, permissão, fechar)

**Files:**
- Modify: `crates/hangar-server/src/runtime/actor.rs` (laço de religação), `codex.rs` (`SetPermissionMode`, `Restart`, `Reload` deixam de ser `lifecycle_required`)
- Modify: `backend/app/runtime_adapter.py`/`runtime_coordinator.py` (fechar a sessão Codex pede `close` com `kill: true` ao Rust; `restart` e `set_permission_mode_sem_terminal` viram controles ao Rust)
- Test: `crates/hangar-server/tests/runtime_actor.rs`, `backend/tests/test_runtime_routing.py`

**Interfaces:**
- Consumes: `process::{spawn,kill}`; `launch_env`.
- Produces: `Respawn { failures:u8, next_at:f64 }` no ator (teto 3, espera 5 s dobrando, zera com subida boa ou ação do usuário); `close` com `kill: true`.

- [x] **Step 1: Testes (falham)**

Rust (`runtime_actor.rs`, com o cano falso da Task 4):
- `cano_exit_respawns_with_backoff_up_to_three`: o cano sai (`cano_saiu`) → o ator sobe de novo após 5 s, depois 10 s, depois 20 s; na quarta saída para e deixa `problema = codex_headless_nao_subiu`.
- `restart_kills_and_respawns_on_the_same_thread`: `Restart` com a sessão ociosa → `kill` do cano atual, `spawn` novo, `thread/resume` da mesma thread.
- `permission_same_sandbox_only_patches_meta`: sessão em "Full Access", `SetPermissionMode("full access")` (mesmo modo, caixa diferente, mesmo sandbox) → só `session.patch_meta {permission_mode:"Full Access"}`, nenhum processo novo.
- `permission_other_sandbox_respawns_when_idle_and_refuses_when_busy`: com turno rodando → erro `erro_permissao_ocupada`; ocioso → kill + spawn com o argv do sandbox novo.
- `close_with_kill_ends_the_process`: `close {kill:true}` → processo morto e arquivos `cano-<key16>*` apagados.

Python (`test_runtime_routing.py`): `DELETE` de sessão Codex sem terminal com o Rust dono chama `close` com `kill: True` e não chama `sem_terminal.matar`; `/recarregar` e `/codex-permissions` (POST) vão ao Rust (controle `restart` / `set_permission_mode`).

- [x] **Step 2: Rodar e ver falhar** (comandos da Task 4).

- [x] **Step 3: Implementar**

- Ator: na saída do cano de sessão Codex sem terminal, se não foi pedida (`close`/`kill`), agenda nova subida pelo `Respawn` (sem laço: um timer por sessão). Subida = `launch_env` + `spawn` + reconexão do motor (`bootstrap(true)`).
- Motor: `Restart`/`Reload` ociosos → efeito novo `Effect::Respawn { reason }` (o ator mata e sobe); `SetPermissionMode` → mesmo sandbox: `session.patch_meta {permission_mode}`; outro sandbox: ocioso → `patch_meta` + `Respawn`; ocupado → `RuntimeError("erro_permissao_ocupada", …)`. Nome de modo desconhecido → `erro_modo_desconhecido`.
- `close` com `kill: true`: para o ator, `process::kill`, apaga os arquivos do cano. O Python (`registry.kill`) continua apagando o arquivo da sessão e a fila depois.
- Órfãos: ao subir o Rust (antes de abrir sessões), `kill_orphans` com o conjunto vivo das duas pastas, uma vez; o `matar_orfaos` do `_boot_sessions` só roda quando o modo é `python` (regra 10 do desenho). Teste Python: com o Rust esperado, `_boot_sessions` não chama `matar_orfaos`.
- Teto esgotado (`Respawn` na terceira falha) mantém o problema da última subida (regra 6).

- [x] **Step 4: Rodar e ver passar; commit**

Commit: `feat(codex): Rust owns headless Codex lifecycle — respawn, restart, permission, close`.

---

### Task 6: Rotas só do Codex no Rust

**Files:**
- Create: `crates/hangar-server/src/session_write/codex.rs`
- Modify: `crates/hangar-server/src/routes.rs`, `crates/hangar-server/src/migration_status.rs`, `crates/hangar-server/src/session_write/mod.rs`
- Create: `backend/tests/fixtures/contract/gen_codex_routes.py` + `golden/codex_routes.json`
- Test: `crates/hangar-server/tests/contract_codex_routes.rs`

**Interfaces:**
- Consumes: controles do motor (`ListModels`, `ReadSettings`, `SetModel`, `SetServiceTier`, `SetMode`, `ReadRateLimits`, `SkipQuestion`, `ListSkills`, `Restart`, `SetPermissionMode`).
- Produces: `GET /models`, `POST /model`, `POST /service-tier`, `POST /codex/mode`, `GET /limits`, `POST /question/skip`, `GET /commands`, `POST /recarregar`, `GET|POST /codex-permissions` atendidos pelo Rust para Codex sem terminal; os mesmos corpos e códigos do Python (`api.py`: `/models` 6327, `/model` 6341, `/service-tier` 6357, `/codex/mode` 6461, `/limits` 6289 com `_normalize_rate_window`, `/question/skip` 9244, `/commands` 10742 com `skills_do_catalogo`, `/recarregar` 2786, `/codex-permissions` 6438 com `modos_para_tela`).

- [x] **Step 1: Gravar o golden**

`gen_codex_routes.py` chama cada rota Python com um adapter Codex falso (respostas fixas do app-server) e grava `{rota, corpo pedido, status, corpo resposta}` para os casos: sucesso, sessão inexistente (404), sessão com terminal (repasse), erro do motor (502/503 com o código do Python), `/question/skip` de id não assíncrono (409), `/codex-permissions` ocupado (409 `erro_permissao_ocupada`).

- [x] **Step 2: Teste de contrato (falha)**

`contract_codex_routes.rs` sobe o roteador com um ator falso que devolve as mesmas respostas do adapter falso e compara cada caso com o golden.

- [x] **Step 3: Implementar** as rotas em `session_write/codex.rs`, cada uma: porta de entrada por nome (5-0) → `decide` (Codex sem terminal → Rust; com terminal → repasse) → controle no ator → corpo no formato do Python. `migration_status.rs` marca as rotas como Rust.

- [x] **Step 4: Rodar e ver passar; commit**

Run: `cd backend && uv run python tests/fixtures/contract/gen_codex_routes.py` (só se o golden mudar) e `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test contract_codex_routes`.
Commit: `feat(codex): Codex-only routes served by Rust for headless sessions`.

---

### Task 7: Link clicável no cartão e Cancelar no nativo

**Files:**
- Modify: `frontend/src/components/OptionButtons.svelte` (PWA), `mobile/src/chat/OptionButtons.tsx`, `desktop-native/src/app.rs` (`render_options`, `render_ask`)
- Modify: `messages/pt.json`, `messages/en.json` (se precisar de rótulo novo)
- Test: `frontend/src/components/OptionButtons.test.ts`, `mobile/src/chat/OptionButtons.test.tsx`, teste de `desktop-native` vizinho de `render_options`

**Interfaces:**
- Consumes: texto do cartão com URL (Task 2).

- [x] **Step 1: Testes (falham)**: URL `https://…` no `question` vira link que abre fora do app (web: `<a target="_blank" rel="noopener">`; Expo: `Linking.openURL`; nativo: `cx.open_url`); texto sem URL fica igual; trecho entre crases continua `code`. Nativo: o cartão de pergunta (`render_ask`) mostra Cancelar sempre — pergunta assíncrona do Codex → `Action::Skip`; demais → `Action::Cancel` (`/interrupt`), igual ao Cancelar do cartão de opções.
- [x] **Step 2: Rodar e ver falhar** (`cd frontend && npx vitest run src/components/OptionButtons.test.ts`; `cd mobile && npx jest src/chat/OptionButtons.test.tsx`; `cd desktop-native && CARGO_BUILD_JOBS=4 cargo test <teste>`).
- [x] **Step 3: Implementar** (só URL `http(s)://` sem espaços; nada de markdown novo no cartão).
- [x] **Step 4: Rodar e ver passar; commit** `feat(ui): clickable links in option cards; native question card gets Cancel`.

---

### Task 8: Regras, README e prova real (verificação manual)

**Files:**
- Modify: `docs/decisoes/harnesses.md` (regra "Codex sem terminal: o app-server é do CANO" passa a "…o cano é do Rust"; regra do `-32601` restrita à última linha da tabela da 5B; regra nova do módulo de processo), `docs/decisoes/superado.md` (decisão 2 do dono único), `docs/migracao-rust/README.md` ("Ainda no Python" sem o Codex sem terminal), `docs/migracao-rust/parte5-codex/pendencias-5b.md` (riscar o que a 5B fechou)

- [ ] **Step 1: Docs** — as quatro mudanças acima, cada regra com o porquê curto.
- [ ] **Step 2: Medição** — release, backend isolado (lançador com no-op, regra da memória "backend isolado mata canos reais"), 5 sessões Codex sem terminal trabalhando: CPU e RSS do Rust × Python (mesmo roteiro da `parte4/medicao.md`). Gravar em `docs/migracao-rust/parte5-codex/medicao-5b.md`.
- [ ] **Step 3: Uso real (verificação manual)** — no PC de casa, Hangar instalado pelo canal de testes na branch: criar Codex sem terminal pelo app nativo e pelo celular; conversar; aprovar comando, arquivo e permissão; responder formulário MCP; Stop com comando longo; trocar modelo, Fast, modo e permissão; `/recarregar`; matar o `hangar-cano` à mão (religa); reiniciar o backend com turno rodando; apagar a sessão (processo some). `CP_RUST_SERVER=0` continua abrindo pelo Python.
- [ ] **Step 4: Commit** `docs(codex): rules and status after Codex headless moved to Rust`.

---

## Self-review

- Spec 5B coberta: nascimento e vida no Rust (Tasks 3–5), religar sem varrer (Task 5, `Respawn`), controles (Tasks 1, 5, 6), linha de status e skills (já no Rust pela 5-0; conferido na Task 6 via `/commands`), estado publicado (vem do slot em fase Rust, Task 4), tabela de pedidos (Task 2), rotas (Tasks 4 e 6), prova real (Task 8). Pendências herdadas: versão conferida (Task 1), `failed` cru e `command` vazio (Task 2), pedido de subagente (Task 2), `start_sessions` antes do `owns` (Task 4), `default_model/default_effort` (fica para a 5E, como combinado).
- Fora: Codex com terminal (5C), contas/catálogo (5E), criação/rename/exclusão como rota (parte 6; aqui só o kill via Rust).
