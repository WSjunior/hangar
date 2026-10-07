# Parte 5A: tipos do protocolo Codex e cliente — plano de implementação

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:subagent-driven-development (recommended) or superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** o protocolo do app-server do Codex passa a ser lido e escrito por tipos Rust conferidos contra o schema que o próprio `codex` emite, e o motor `runtime/codex.rs` usa esses tipos; um cliente JSON-RPC assíncrono (stdio e WebSocket) fica pronto para 5C/5E.

**Architecture:** crate novo `crates/hangar-codex` (sem axum) com `proto` (tipos escritos à mão só do que o Hangar usa, tolerantes a campo novo), `version` (aviso de versão diferente da conferida) e `client` (JSON-RPC sobre canais, com transportes stdio e WebSocket). Um recorte do schema da versão conferida fica no crate e um teste confere que todo campo que os tipos usam existe nele. O motor continua uma máquina de estados síncrona: só a borda (o que entra e o que sai) passa a ser tipada; a contabilidade interna (`Rpc.params`, continuações) continua `Value`. Nenhuma mudança de dono: o Codex segue no Python até 5B.

**Tech Stack:** Rust 2024, serde/serde_json (já no workspace), tokio, tokio-tungstenite `=0.29.0` (já no lockfile), schemars 1.x só em `dev-dependencies`.

**Spec:** [`spec.md`](spec.md) (seções "Tipos do protocolo" e "5A"). **Análise:** [`analise.md`](analise.md). **Contrato com a metade Claude:** [`../parte5-claude/contrato-par.md`](../parte5-claude/contrato-par.md).

## Global Constraints

- Base: `main` + `origin/fix/codex-hardening` (Task 1). Nada desta branch é refeito.
- Testes de cada Task: escritos primeiro, vistos falhar, rodados só os dos arquivos tocados
  (`cd crates && CARGO_BUILD_JOBS=4 cargo test -p <crate> --test <x>` ou `--lib <mod>::`). No
  máximo 2 `cargo` na máquina (`pgrep -c -x cargo` antes de rodar); `target/` é o da worktree
  (`crates/target`) e é apagado no fim da 5A; `rust-analyzer` desligado na worktree.
- Tipos: nunca `deny_unknown_fields`; todo struct com `#[serde(default)]`; `ReasoningEffort` é
  `String`; os três nomes do esforço ficam com o nome exato do schema (`effort` nos pedidos e em
  `ThreadSettings`, `reasoningEffort` nas respostas e em `Thread`, `reasoning_effort` em
  `Settings`), sem `alias` cruzado.
- Nome de cada struct = nome da definição no schema do Codex (o teste do recorte casa por nome).
- O `hangar-cano` não depende do crate novo e não muda.
- Contrato interno (`RUST_SERVER_PROTOCOL`/`INTERNAL_PROTOCOL`): a 5A não muda rota `/internal`,
  evento do `side-events` nem variável do filho, então **não sobe**. Se uma Task precisar, ela para
  e avisa.
- Golden `backend/tests/fixtures/headless_runtime/codex-golden.json` e os testes
  `runtime_codex.rs`/`runtime_actor.rs`/`runtime_contract.rs` passam sem mudar asserção existente.
- Log e diário nunca levam texto de conversa: o código do diário é `[a-z0-9_]{1,64}`, o motivo é
  frase fixa.
- Desempenho (README, "erros que já custaram"): decodificar uma vez por linha; canais com limite;
  nenhum laço sem entrada; nenhum clone de linha grande além do que o motor já faz.
- Texto de tela por `m.<chave>()`, `pt.json` e `en.json` no mesmo commit.
- Antes de mexer em `runtime/codex.rs` ou `actor.rs`: ler "Regras vigentes" de
  `docs/decisoes/harnesses.md`.
- Commits seletivos por caminho (`git add <arquivos>`), nunca `-A`; sem push.

## Review Focus

1. **Campo com tipo trocado numa notificação conhecida** (ex.: `turnId: null` num
   `item/agentMessage/delta`): o motor não pode cair nem travar a sessão; a notificação é
   ignorada, o diário recebe `rust.codex_decode` com o método, e a seguinte é processada normalmente.
   Teste na Task 4.
2. **Método novo que o Hangar não conhece:** notificação é ignorada sem diário (não é erro);
   pedido do servidor continua recebendo `-32601` com o nome do método. Testes nas Tasks 2 e 4.
3. **`userAgent` estranho ou ausente no `initialize`** (`codex-cli/0.160.1-alpha.2 (…)`, sem
   barra, vazio): nada quebra, o aviso só sai quando major.minor dá para ler e difere. Teste na
   Task 2.
4. **App-server que morre com pedido em voo:** `request` devolve `Closed` na hora, não espera o
   prazo; o canal de notificações fecha. Teste na Task 5.
5. **Linha enorme do app-server (> 16 MiB):** o cliente encerra a conexão com erro em vez de
   crescer memória sem fim. Teste na Task 5.

---

## Arquivos

| Arquivo | Responsabilidade |
|---|---|
| `crates/Cargo.toml` | membro `hangar-codex`; `schemars` e `tokio-tungstenite` no workspace |
| `crates/hangar-codex/Cargo.toml` | crate novo |
| `crates/hangar-codex/src/lib.rs` | `pub mod proto; pub mod version; pub mod client;` |
| `crates/hangar-codex/src/proto.rs` | tipos do protocolo usados pelo Hangar |
| `crates/hangar-codex/src/version.rs` | versão conferida e leitura do `userAgent` |
| `crates/hangar-codex/src/schema_check.rs` | (só `cfg(test)`) teste do recorte e regerador |
| `crates/hangar-codex/schema/0.159.3.json` | recorte do schema da versão conferida |
| `crates/hangar-codex/src/client.rs` | cliente JSON-RPC assíncrono, transportes stdio e WebSocket |
| `crates/hangar-codex/tests/client.rs` | testes do cliente contra servidor falso |
| `scripts/conferir-codex-schema` | gera o schema do Codex instalado e refaz o recorte |
| `crates/hangar-server/Cargo.toml` | depende de `hangar-codex` |
| `crates/hangar-server/src/runtime/protocol.rs` | `RequestId` vem do crate novo; `Effect::Diag` |
| `crates/hangar-server/src/runtime/codex.rs` | borda tipada, aviso de versão, falha de decodificação |
| `crates/hangar-server/src/runtime/actor.rs` | `PolicyClient` com `DiagClient`; trata `Effect::Diag` |
| `crates/hangar-server/tests/runtime_codex.rs` | testes novos do motor |
| `messages/pt.json`, `messages/en.json`, `frontend/src/lib/problema.ts`, `mobile/src/chat/SessionProblem.tsx` | aviso `codex_versao_nao_conferida` |
| `docs/decisoes/harnesses.md`, `docs/migracao-rust/README.md` | regra nova e estado |

---

### Task 1: Base com a `fix/codex-hardening`

**Files:** nenhum arquivo novo; merge.

**Interfaces:**
- Consumes: `origin/fix/codex-hardening` (2 commits sobre `fa6617111`).
- Produces: `runtime/codex.rs` com `running_commands`, `thinking`, `error_class`,
  `cut_check`, `terminate_after` — a base que as Tasks 2–5 leem.

- [ ] **Step 1: Conferir se o PR já entrou na `main`**

```bash
git fetch origin
git merge-base --is-ancestor origin/fix/codex-hardening origin/main && echo "já na main"
```

Se imprimir "já na main": `git merge --no-edit origin/main` e pular para o Step 3. Senão, Step 2.

- [ ] **Step 2: Juntar a branch**

```bash
git merge --no-edit origin/fix/codex-hardening
```

Esperado: sem conflito (esta branch só tem documentos em `docs/migracao-rust/`). Conflito em
qualquer arquivo de código → parar e avisar.

- [ ] **Step 3: Conferir a base**

```bash
cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex --test runtime_contract --test runtime_actor
```

Esperado: tudo verde. Falha aqui é da `fix/codex-hardening` (sem testes rodados na origem):
anotar o teste e a saída, parar e avisar antes de seguir.

---

### Task 2: Crate `hangar-codex` com os tipos e a versão

**Files:**
- Modify: `crates/Cargo.toml`
- Create: `crates/hangar-codex/Cargo.toml`, `src/lib.rs`, `src/proto.rs`, `src/version.rs`
- Modify: `crates/hangar-server/Cargo.toml`, `crates/hangar-server/src/runtime/protocol.rs:40-45`

**Interfaces:**
- Produces (usados nas Tasks 3–5):
  - `hangar_codex::proto::{RequestId, ClientRequest, ServerNotification, ServerRequest, ThreadItem, UserInput, ThreadStatus, Thread, Turn, TurnError, ThreadSettings, CollaborationMode, Settings, InitializeResponse, ThreadStartResponse, ThreadResumeResponse, ThreadReadResponse, TurnStartResponse, ModelListResponse, Model, GetAccountRateLimitsResponse, DecodeError}` e os `*Params`/`*Notification` abaixo.
  - `ServerNotification::decode(method:&str, params:&Value) -> Result<ServerNotification,DecodeError>`
  - `ServerRequest::decode(method:&str, params:&Value) -> Result<ServerRequest,DecodeError>`
  - `ServerNotification::METHODS`, `ServerRequest::METHODS`, `ClientRequest::METHODS: &[&str]`
  - `hangar_codex::proto::error_kind(&serde_json::Error) -> &'static str`
  - `ClientRequest::into_parts(self) -> (&'static str, Value)`
  - `hangar_codex::version::{CHECKED:&str, from_user_agent(&str)->Option<&str>, differs(&str)->bool, diag_code(&str)->String}`

- [ ] **Step 1: Workspace e crate**

`crates/Cargo.toml`: acrescentar `"hangar-codex"` em `members` e, em `[workspace.dependencies]`:

```toml
hangar-codex = { path = "hangar-codex" }
tokio-tungstenite = "=0.29.0"
```

`crates/hangar-codex/Cargo.toml`:

```toml
[package]
name = "hangar-codex"
version.workspace = true
edition.workspace = true
publish = false

[dependencies]
serde.workspace = true
serde_json.workspace = true
tokio = { workspace = true, features = ["rt", "macros", "io-util", "process", "sync", "time", "net"] }
tokio-tungstenite.workspace = true
futures-util.workspace = true
tracing.workspace = true

[dev-dependencies]
tokio = { workspace = true, features = ["rt-multi-thread", "macros", "test-util"] }
```

Depois: `cd crates && cargo add -p hangar-codex --dev schemars@1` e fixar a versão resolvida com
`=` (padrão do workspace). Os tipos derivam `JsonSchema` só em teste (`cfg_attr(test,…)`):
conferir que `schemars` não aparece em `cargo tree -p hangar-server -e normal`. Em `crates/hangar-server/Cargo.toml`, em `[dependencies]`:
`hangar-codex.workspace = true`; na linha do `tokio-tungstenite` de `[dev-dependencies]`, trocar
por `tokio-tungstenite.workspace = true`.

`crates/hangar-codex/src/lib.rs`:

```rust
//! Protocolo do app-server do Codex: tipos conferidos contra o schema da versão conferida e o
//! cliente JSON-RPC. Sem axum: serve ao servidor e ao lançador.
pub mod proto;
pub mod version;
pub mod client;
#[cfg(test)]
mod schema_check;
```

Até a Task 5, criar `src/client.rs` vazio (`//! Cliente JSON-RPC (Task 5).`) e, até a Task 3,
`src/schema_check.rs` vazio.

- [ ] **Step 2: Testes dos tipos (falham sem `proto.rs`)**

No fim de `crates/hangar-codex/src/proto.rs` (arquivo novo, só o módulo de teste por enquanto):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn unknown_fields_and_missing_optionals_are_tolerated() {
        let n = ServerNotification::decode("turn/started",&json!({"threadId":"t","turn":{"id":"u","status":"inProgress","novo":1}})).unwrap();
        let ServerNotification::TurnStarted(n) = n else { panic!() };
        assert_eq!((n.thread_id.as_str(),n.turn.id.as_str()),("t","u"));
        let n = ServerNotification::decode("item/agentMessage/delta",&json!({"delta":"oi"})).unwrap();
        assert!(matches!(n,ServerNotification::AgentMessageDelta(d) if d.delta == "oi" && d.turn_id.is_empty()));
    }

    #[test]
    fn unknown_method_is_unknown_not_error() {
        assert!(matches!(ServerNotification::decode("thread/novidade",&json!({"x":1})),Ok(ServerNotification::Unknown)));
        assert!(matches!(ServerRequest::decode("foo/bar",&json!({})),Ok(ServerRequest::Unknown)));
    }

    #[test]
    fn wrong_type_on_known_method_is_error_with_method() {
        let err = ServerNotification::decode("item/agentMessage/delta",&json!({"delta":5})).unwrap_err();
        assert_eq!(err.method,"item/agentMessage/delta");
    }

    #[test]
    fn null_params_decode_as_empty() {
        assert!(matches!(ServerNotification::decode("turn/started",&Value::Null),Ok(ServerNotification::TurnStarted(_))));
    }

    #[test]
    fn presence_is_kept_for_settings() {
        let ServerNotification::ThreadSettingsUpdated(n) = ServerNotification::decode("thread/settings/updated",
            &json!({"threadId":"t","threadSettings":{"model":"m","effort":null}})).unwrap() else { panic!() };
        assert_eq!(n.thread_settings.effort,Some(None));
        assert_eq!(n.thread_settings.service_tier,None);
        assert_eq!(n.thread_settings.model,Some("m".into()));
    }

    #[test]
    fn thread_item_tags_and_fallback() {
        let item:ThreadItem = serde_json::from_value(json!({"type":"agentMessage","id":"a","text":"x","delivery":"async","questions":[{"title":"Q","options":["s"]}]})).unwrap();
        assert!(matches!(&item,ThreadItem::AgentMessage { delivery:Some(d),questions,.. } if d == "async" && questions[0].title == "Q"));
        let item:ThreadItem = serde_json::from_value(json!({"type":"imageGeneration","id":"z"})).unwrap();
        assert!(matches!(item,ThreadItem::Unknown));
    }

    #[test]
    fn client_requests_keep_the_wire_shape() {
        let (method,params) = ClientRequest::ThreadStart(ThreadStartParams { cwd:Some("/p".into()),model:None,
            approval_policy:Some("never".into()),sandbox:Some("danger-full-access".into()),service_tier:None }).into_parts();
        assert_eq!(method,"thread/start");
        assert_eq!(params,json!({"cwd":"/p","model":null,"approvalPolicy":"never","sandbox":"danger-full-access"}));
        let (_,params) = ClientRequest::ThreadSettingsUpdate(ThreadSettingsUpdateParams { thread_id:"t".into(),
            effort:Some(None),..Default::default() }).into_parts();
        assert_eq!(params,json!({"threadId":"t","effort":null}));
        let (_,params) = ClientRequest::TurnStart(TurnStartParams { thread_id:"t".into(),input:vec![json!({"type":"text","text":"oi"})],
            approval_policy:None,summary:None }).into_parts();
        assert_eq!(params,json!({"threadId":"t","input":[{"type":"text","text":"oi"}]}));
        let (method,params) = ClientRequest::AccountRateLimitsRead.into_parts();
        assert_eq!((method,params),("account/rateLimits/read",json!({})));
    }

    #[test]
    fn methods_lists_match_the_variants() {
        for method in ServerNotification::METHODS { assert!(!matches!(ServerNotification::decode(method,&json!({})),Ok(ServerNotification::Unknown)),"{method}"); }
        for method in ServerRequest::METHODS { assert!(!matches!(ServerRequest::decode(method,&json!({})),Ok(ServerRequest::Unknown)),"{method}"); }
    }
}
```

`crates/hangar-codex/src/version.rs` (só os testes por enquanto, no fim do arquivo):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn reads_the_version_from_user_agent() {
        assert_eq!(from_user_agent("hangar/0.159.3 (CachyOS 1; x86_64)"),Some("0.159.3"));
        assert_eq!(from_user_agent("codex-cli/0.160.1-alpha.2 (x)"),Some("0.160.1-alpha.2"));
        assert_eq!(from_user_agent("sem barra"),None);
        assert_eq!(from_user_agent(""),None);
    }
    #[test]
    fn only_major_minor_counts() {
        assert!(!differs(CHECKED));
        assert!(!differs("0.159.9"));
        assert!(differs("0.160.1-alpha.2"));
        assert!(!differs("lixo"));
    }
    #[test]
    fn diag_code_fits_the_diary() {
        assert_eq!(diag_code("0.160.1-alpha.2"),"codex_0_160");
        assert_eq!(diag_code("lixo"),"codex_desconhecida");
    }
}
```

- [ ] **Step 3: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --lib`
Expected: FAIL de compilação (`ServerNotification`, `from_user_agent` não existem).

- [ ] **Step 4: Escrever `proto.rs`**

Acima do módulo de testes:

```rust
//! Tipos do app-server do Codex que o Hangar lê ou escreve. Escritos à mão a partir do schema
//! emitido pelo binário (`codex app-server generate-json-schema --experimental`): só o usado, todo
//! campo opcional e tolerante a campo novo. O teste do recorte (`schema_check.rs`) quebra quando um
//! campo usado some ou muda de nome na versão conferida.
use serde::{Deserialize,Deserializer,Serialize};
use serde_json::Value;

#[derive(Clone,Debug,PartialEq,Eq,Hash,PartialOrd,Ord,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(untagged)]
pub enum RequestId { Integer(i64), String(String) }

/// Campo ausente = `None`; `null` = `Some(None)`. O motor distingue "não veio" de "veio vazio".
fn present<'de,D:Deserializer<'de>,T:Deserialize<'de>>(d:D) -> Result<Option<Option<T>>,D::Error> {
    Option::<T>::deserialize(d).map(Some)
}

#[derive(Debug)]
pub struct DecodeError { pub method:String, pub error:serde_json::Error }

/// Só a categoria do erro: a mensagem do serde pode ecoar valores da conversa.
pub fn error_kind(error:&serde_json::Error) -> &'static str {
    use serde_json::error::Category;
    match error.classify() { Category::Io=>"io",Category::Syntax=>"syntax",Category::Data=>"data",Category::Eof=>"eof" }
}

macro_rules! wire {
    ($(#[$m:meta])* pub struct $name:ident { $($body:tt)* }) => {
        $(#[$m])*
        #[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
        #[cfg_attr(test,derive(schemars::JsonSchema))]
        #[serde(rename_all = "camelCase", default)]
        pub struct $name { $($body)* }
    };
}

// ---------- pedidos do cliente ----------

wire!(pub struct ClientInfo { pub name:String, pub title:Option<String>, pub version:String });
wire!(pub struct InitializeCapabilities { pub experimental_api:bool });
wire!(pub struct InitializeParams { pub client_info:ClientInfo, pub capabilities:InitializeCapabilities });

wire!(pub struct ThreadStartParams {
    pub cwd:Option<String>,
    pub model:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub approval_policy:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub sandbox:Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")] pub service_tier:Option<String>,
});
wire!(pub struct ThreadResumeParams { pub thread_id:String });
wire!(pub struct ThreadReadParams { pub thread_id:String, pub include_turns:bool });
wire!(pub struct ThreadCompactStartParams { pub thread_id:String });
wire!(pub struct ThreadUnsubscribeParams { pub thread_id:String });
wire!(pub struct ThreadBackgroundTerminalsTerminateParams { pub thread_id:String, pub process_id:String });
// `Settings` do `collaborationMode` é snake_case no schema; não usa a macro.
#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(default)]
pub struct CollaborationSettings { pub model:String, pub reasoning_effort:Option<String>, pub developer_instructions:Option<String> }
wire!(pub struct CollaborationMode { pub mode:String, pub settings:CollaborationSettings });
wire!(pub struct ThreadSettingsUpdateParams {
    pub thread_id:String,
    #[serde(skip_serializing_if = "Option::is_none")] pub model:Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub effort:Option<Option<String>>,
    #[serde(skip_serializing_if = "Option::is_none")] pub collaboration_mode:Option<CollaborationMode>,
    #[serde(skip_serializing_if = "Option::is_none")] pub service_tier:Option<String>,
});
wire!(pub struct TurnStartParams {
    pub thread_id:String,
    #[serde(skip_serializing_if = "Option::is_none")] pub approval_policy:Option<String>,
    /// Sem ele o pensamento só fica cifrado no rollout.
    #[serde(skip_serializing_if = "Option::is_none")] pub summary:Option<String>,
    /// Montado pelo `prepare_prompt` (texto, skill, imagem); passa como veio.
    pub input:Vec<Value>,
});
wire!(pub struct TurnSteerParams { pub thread_id:String, pub expected_turn_id:String, pub input:Vec<Value> });
wire!(pub struct TurnInterruptParams { pub thread_id:String, pub turn_id:String });
wire!(pub struct ModelListParams {});
wire!(pub struct SkillsListParams { pub cwds:Vec<String> });

#[derive(Clone,Debug,PartialEq,Serialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "method", content = "params")]
pub enum ClientRequest {
    #[serde(rename = "initialize")] Initialize(InitializeParams),
    #[serde(rename = "thread/start")] ThreadStart(ThreadStartParams),
    #[serde(rename = "thread/resume")] ThreadResume(ThreadResumeParams),
    #[serde(rename = "thread/read")] ThreadRead(ThreadReadParams),
    #[serde(rename = "thread/settings/update")] ThreadSettingsUpdate(ThreadSettingsUpdateParams),
    #[serde(rename = "thread/compact/start")] ThreadCompactStart(ThreadCompactStartParams),
    #[serde(rename = "thread/unsubscribe")] ThreadUnsubscribe(ThreadUnsubscribeParams),
    #[serde(rename = "thread/backgroundTerminals/terminate")] ThreadBackgroundTerminalsTerminate(ThreadBackgroundTerminalsTerminateParams),
    #[serde(rename = "turn/start")] TurnStart(TurnStartParams),
    #[serde(rename = "turn/steer")] TurnSteer(TurnSteerParams),
    #[serde(rename = "turn/interrupt")] TurnInterrupt(TurnInterruptParams),
    #[serde(rename = "model/list")] ModelList(ModelListParams),
    #[serde(rename = "skills/list")] SkillsList(SkillsListParams),
    #[serde(rename = "account/rateLimits/read")] AccountRateLimitsRead,
}

impl ClientRequest {
    pub const METHODS:&[&str] = &["initialize","thread/start","thread/resume","thread/read","thread/settings/update",
        "thread/compact/start","thread/unsubscribe","thread/backgroundTerminals/terminate","turn/start","turn/steer",
        "turn/interrupt","model/list","skills/list","account/rateLimits/read"];

    pub fn into_parts(self) -> (&'static str,Value) {
        let mut value = serde_json::to_value(&self).expect("pedido serializa");
        let method = value["method"].as_str().and_then(|m|Self::METHODS.iter().find(|k|**k == m)).copied().expect("método conhecido");
        let params = value.get_mut("params").map(Value::take).unwrap_or_else(||Value::Object(Default::default()));
        (method,params)
    }
}

// ---------- respostas ----------

wire!(pub struct InitializeResponse { pub user_agent:String });
wire!(pub struct TurnError { pub message:String, pub additional_details:Option<String>, pub codex_error_info:Option<Value> });
wire!(pub struct Turn { pub id:String, pub status:String, pub error:Option<TurnError>, pub items:Vec<ThreadItem> });

#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase")]
pub enum ThreadStatus {
    Idle,
    Active,
    NotLoaded,
    SystemError,
    #[default] #[serde(other)] Unknown,
}

#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct Thread {
    pub id:String,
    pub path:Option<String>,
    pub status:ThreadStatus,
    pub turns:Vec<Turn>,
    #[serde(deserialize_with = "present")] pub model:Option<Option<String>>,
    #[serde(deserialize_with = "present")] pub reasoning_effort:Option<Option<String>>,
}

#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadStartResponse {
    pub thread:Thread,
    pub model:Option<String>,
    pub reasoning_effort:Option<String>,
    #[serde(deserialize_with = "present")] pub service_tier:Option<Option<String>>,
}
pub type ThreadResumeResponse = ThreadStartResponse;
wire!(pub struct ThreadReadResponse { pub thread:Thread });
wire!(pub struct TurnStartResponse { pub turn:Turn });
wire!(pub struct ReasoningEffortOption { pub reasoning_effort:String, pub description:String });
#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct Model {
    pub model:String,
    pub display_name:String,
    pub description:String,
    pub hidden:bool,
    pub supported_reasoning_efforts:Vec<ReasoningEffortOption>,
    pub default_reasoning_effort:String,
    /// Lido como `Value`: a tela recebe a lista como veio.
    pub service_tiers:Option<Vec<Value>>,
    pub default_service_tier:Option<String>,
}
wire!(pub struct ModelListResponse { pub data:Vec<Model> });
wire!(pub struct RateLimitWindow { pub used_percent:Option<f64>, pub window_duration_mins:Option<i64>, pub resets_at:Option<i64> });
wire!(pub struct RateLimitSnapshot { pub limit_id:Option<String>, pub primary:Option<RateLimitWindow>, pub secondary:Option<RateLimitWindow> });
wire!(pub struct GetAccountRateLimitsResponse { pub rate_limits:RateLimitSnapshot });
wire!(pub struct TokenUsageBreakdown { pub input_tokens:i64, pub output_tokens:i64, pub cached_input_tokens:i64, pub reasoning_output_tokens:i64, pub total_tokens:i64 });
wire!(pub struct ThreadTokenUsage { pub last:TokenUsageBreakdown, pub total:TokenUsageBreakdown, pub model_context_window:Option<i64> });

// ---------- itens ----------

wire!(pub struct AsyncUserInputQuestion { pub title:String, pub options:Option<Vec<Value>> });

#[derive(Clone,Debug,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum UserInput {
    Text { #[serde(default)] text:String },
    #[serde(other)] Unknown,
}

#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "type", rename_all = "camelCase", rename_all_fields = "camelCase")]
pub enum ThreadItem {
    AgentMessage { #[serde(default)] id:String, #[serde(default)] text:String, #[serde(default)] delivery:Option<String>,
        #[serde(default)] questions:Vec<AsyncUserInputQuestion> },
    UserMessage { #[serde(default)] id:String, #[serde(default)] content:Vec<UserInput> },
    Reasoning { #[serde(default)] id:String },
    CommandExecution { #[serde(default)] id:String, #[serde(default)] process_id:Option<String> },
    ContextCompaction { #[serde(default)] id:String },
    #[default] #[serde(other)] Unknown,
}

// ---------- notificações ----------

wire!(pub struct ServerRequestResolvedNotification { pub thread_id:String, pub request_id:Option<RequestId> });
wire!(pub struct TurnStartedNotification { pub thread_id:String, pub turn:Turn });
wire!(pub struct TurnCompletedNotification { pub thread_id:String, pub turn:Turn });
wire!(pub struct ThreadStatusChangedNotification { pub thread_id:String, pub status:ThreadStatus });
#[derive(Clone,Debug,Default,PartialEq,Serialize,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(rename_all = "camelCase", default)]
pub struct ThreadSettings {
    pub model:Option<String>,
    #[serde(deserialize_with = "present")] pub effort:Option<Option<String>>,
    #[serde(deserialize_with = "present")] pub collaboration_mode:Option<Option<CollaborationMode>>,
    #[serde(deserialize_with = "present")] pub service_tier:Option<Option<String>>,
}
wire!(pub struct ThreadSettingsUpdatedNotification { pub thread_id:String, pub thread_settings:ThreadSettings });
wire!(pub struct AgentMessageDeltaNotification { pub thread_id:String, pub turn_id:String, pub item_id:String, pub delta:String });
wire!(pub struct ReasoningSummaryTextDeltaNotification { pub thread_id:String, pub turn_id:String, pub delta:String, pub summary_index:i64 });
wire!(pub struct ReasoningTextDeltaNotification { pub thread_id:String, pub turn_id:String, pub delta:String });
wire!(pub struct ReasoningSummaryPartAddedNotification { pub thread_id:String, pub turn_id:String, pub summary_index:i64 });
wire!(pub struct ModelReroutedNotification { pub thread_id:String, pub to_model:Option<String> });
wire!(pub struct ItemStartedNotification { pub thread_id:String, pub turn_id:String, pub item:ThreadItem });
pub type ItemCompletedNotification = ItemStartedNotification;
wire!(pub struct ModelSafetyBufferingUpdatedNotification { pub thread_id:String, pub turn_id:String, pub show_buffering_ui:Option<bool> });
wire!(pub struct ThreadTokenUsageUpdatedNotification { pub thread_id:String, pub token_usage:Option<ThreadTokenUsage> });
wire!(pub struct AccountRateLimitsUpdatedNotification { pub rate_limits:Option<RateLimitSnapshot> });
wire!(pub struct ErrorNotification { pub thread_id:String, pub error:TurnError, pub will_retry:bool });
wire!(pub struct HookOutputEntry { pub kind:String, pub text:Option<String> });
wire!(pub struct HookRunSummary { pub event_name:String, pub status:String, pub source_path:Option<String>, pub entries:Vec<HookOutputEntry> });
wire!(pub struct HookCompletedNotification { pub thread_id:String, pub run:HookRunSummary });

#[derive(Clone,Debug,PartialEq,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "method", content = "params")]
pub enum ServerNotification {
    #[serde(rename = "serverRequest/resolved")] ServerRequestResolved(ServerRequestResolvedNotification),
    #[serde(rename = "turn/started")] TurnStarted(TurnStartedNotification),
    #[serde(rename = "turn/completed")] TurnCompleted(TurnCompletedNotification),
    #[serde(rename = "thread/status/changed")] ThreadStatusChanged(ThreadStatusChangedNotification),
    #[serde(rename = "thread/settings/updated")] ThreadSettingsUpdated(ThreadSettingsUpdatedNotification),
    #[serde(rename = "item/agentMessage/delta")] AgentMessageDelta(AgentMessageDeltaNotification),
    #[serde(rename = "item/reasoning/summaryTextDelta")] ReasoningSummaryTextDelta(ReasoningSummaryTextDeltaNotification),
    #[serde(rename = "item/reasoning/textDelta")] ReasoningTextDelta(ReasoningTextDeltaNotification),
    #[serde(rename = "item/reasoning/summaryPartAdded")] ReasoningSummaryPartAdded(ReasoningSummaryPartAddedNotification),
    #[serde(rename = "model/rerouted")] ModelRerouted(ModelReroutedNotification),
    #[serde(rename = "item/started")] ItemStarted(ItemStartedNotification),
    #[serde(rename = "item/completed")] ItemCompleted(ItemCompletedNotification),
    #[serde(rename = "model/safetyBuffering/updated")] ModelSafetyBufferingUpdated(ModelSafetyBufferingUpdatedNotification),
    #[serde(rename = "thread/tokenUsage/updated")] ThreadTokenUsageUpdated(ThreadTokenUsageUpdatedNotification),
    #[serde(rename = "account/rateLimits/updated")] AccountRateLimitsUpdated(AccountRateLimitsUpdatedNotification),
    #[serde(rename = "error")] Error(ErrorNotification),
    #[serde(rename = "hook/completed")] HookCompleted(HookCompletedNotification),
    /// Método que o Hangar não usa: ignorado sem erro.
    #[serde(skip)] Unknown,
}

// ---------- pedidos do servidor ----------

wire!(pub struct CommandExecutionRequestApprovalParams { pub thread_id:String, pub command:Option<String>, pub cwd:Option<String>, pub reason:Option<String> });
wire!(pub struct FileChangeRequestApprovalParams { pub thread_id:String, pub grant_root:Option<String>, pub reason:Option<String> });
wire!(pub struct ToolRequestUserInputQuestion { pub id:String, pub header:String, pub question:String, pub is_other:Option<bool>,
    pub is_secret:Option<bool>, pub options:Option<Vec<Value>> });
wire!(pub struct ToolRequestUserInputParams { pub thread_id:String, pub questions:Vec<ToolRequestUserInputQuestion> });
wire!(pub struct ThreadOnlyParams { pub thread_id:String });

#[derive(Clone,Debug,PartialEq,Deserialize)]
#[cfg_attr(test,derive(schemars::JsonSchema))]
#[serde(tag = "method", content = "params")]
pub enum ServerRequest {
    #[serde(rename = "item/commandExecution/requestApproval")] CommandExecutionApproval(CommandExecutionRequestApprovalParams),
    #[serde(rename = "item/fileChange/requestApproval")] FileChangeApproval(FileChangeRequestApprovalParams),
    #[serde(rename = "item/tool/requestUserInput")] ToolRequestUserInput(ToolRequestUserInputParams),
    /// Atendidos na 5B; na 5A continuam recusados com `-32601`, mas já reconhecidos.
    #[serde(rename = "item/permissions/requestApproval")] PermissionsApproval(ThreadOnlyParams),
    #[serde(rename = "mcpServer/elicitation/request")] McpServerElicitation(ThreadOnlyParams),
    #[serde(skip)] Unknown,
}

macro_rules! decoder {
    ($ty:ident, [$($method:literal),* $(,)?]) => {
        impl $ty {
            pub const METHODS:&[&str] = &[$($method),*];
            pub fn decode(method:&str,params:&Value) -> Result<Self,DecodeError> {
                if !Self::METHODS.contains(&method) { return Ok(Self::Unknown); }
                let params = if params.is_null() { Value::Object(Default::default()) } else { params.clone() };
                serde_json::from_value(serde_json::json!({"method":method,"params":params}))
                    .map_err(|error|DecodeError { method:method.into(),error })
            }
        }
    };
}

decoder!(ServerNotification,["serverRequest/resolved","turn/started","turn/completed","thread/status/changed",
    "thread/settings/updated","item/agentMessage/delta","item/reasoning/summaryTextDelta","item/reasoning/textDelta",
    "item/reasoning/summaryPartAdded","model/rerouted","item/started","item/completed","model/safetyBuffering/updated",
    "thread/tokenUsage/updated","account/rateLimits/updated","error","hook/completed"]);
decoder!(ServerRequest,["item/commandExecution/requestApproval","item/fileChange/requestApproval","item/tool/requestUserInput",
    "item/permissions/requestApproval","mcpServer/elicitation/request"]);
```

Notas para quem implementa:
- `CollaborationSettings` e `ThreadOnlyParams` não têm o nome do schema: o teste do recorte
  (Task 3) os casa por `LOCAL_NAMES` (`Settings`; `PermissionsRequestApprovalParams` e
  `McpServerElicitationRequestParams`).
- Se `#[serde(skip)]` numa variante de enum com `tag`/`content` não compilar com `Deserialize`,
  troque por `#[serde(skip_deserializing)]`; o `decode` nunca chega a ela pelo serde.
- `ThreadStatus::Active` ignora `activeFlags` (o motor não lê).

- [ ] **Step 5: Escrever `version.rs`**

Acima dos testes:

```rust
//! Versão do Codex conferida contra o recorte do schema. Outra major.minor no `initialize` vira
//! aviso: os tipos continuam tolerantes, mas campo renomeado pode faltar.
#[macro_export]
#[doc(hidden)]
macro_rules! checked_version { () => { "0.159.3" } }

pub const CHECKED:&str = checked_version!();

/// `"<cliente>/<versão> (…)"` → `<versão>`.
pub fn from_user_agent(user_agent:&str) -> Option<&str> {
    let rest = user_agent.split_once('/')?.1;
    let version = rest.split_whitespace().next()?;
    (!version.is_empty()).then_some(version)
}

fn major_minor(version:&str) -> Option<(u64,u64)> {
    let mut parts = version.split(|c:char|c == '.' || c == '-');
    Some((parts.next()?.parse().ok()?,parts.next()?.parse().ok()?))
}

/// Versão ilegível não avisa: sem número não há o que comparar.
pub fn differs(installed:&str) -> bool {
    match (major_minor(installed),major_minor(CHECKED)) { (Some(a),Some(b)) => a != b, _ => false }
}

/// Código do diário (`[a-z0-9_]{1,64}`).
pub fn diag_code(installed:&str) -> String {
    major_minor(installed).map_or_else(||"codex_desconhecida".into(),|(major,minor)|format!("codex_{major}_{minor}"))
}
```

- [ ] **Step 6: `RequestId` vem do crate novo**

Em `crates/hangar-server/src/runtime/protocol.rs`, trocar o bloco `pub enum RequestId` (linhas
40-45, com os derives) por:

```rust
pub use hangar_codex::proto::RequestId;
```

- [ ] **Step 7: Rodar e ver passar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --lib && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex --test runtime_actor`
Expected: PASS.

- [ ] **Step 8: Commit**

```bash
git add crates/Cargo.toml crates/Cargo.lock crates/hangar-codex crates/hangar-server/Cargo.toml crates/hangar-server/src/runtime/protocol.rs
git commit -m "feat(codex): typed app-server protocol crate"
```

---

### Task 3: Recorte do schema e teste de campo renomeado

**Files:**
- Create: `crates/hangar-codex/src/schema_check.rs`, `crates/hangar-codex/schema/0.159.3.json`, `scripts/conferir-codex-schema`

**Interfaces:**
- Consumes: tipos da Task 2, `checked_version!()`.
- Produces: `schema/<versão>.json` com `{"version", "definitions", "methods":{"ServerNotification":[…],"ServerRequest":[…],"ClientRequest":[…]}}`;
  teste `schema_check::fields_used_exist_in_checked_schema`; regerador
  `schema_check::regenerate_slice` (`#[ignore]`, lê `CODEX_SCHEMA_DIR`).

- [ ] **Step 1: Escrever o teste do recorte**

`crates/hangar-codex/src/schema_check.rs`:

```rust
//! Todo campo e todo método que os tipos de `proto` usam existem no schema da versão conferida.
//! O schema vem de `codex app-server generate-json-schema --experimental`; o recorte guarda só as
//! definições alcançadas a partir dos tipos usados. Regerar: `scripts/conferir-codex-schema`.
use crate::proto::*;
use serde_json::{Map,Value,json};
use std::collections::{BTreeMap,BTreeSet};

const SLICE:&str = include_str!(concat!(env!("CARGO_MANIFEST_DIR"),"/schema/",crate::checked_version!(),".json"));

/// Tipos nossos sem nome no schema → definições do schema que eles representam.
const LOCAL_NAMES:&[(&str,&[&str])] = &[
    ("CollaborationSettings",&["Settings"]),
    ("ThreadOnlyParams",&["PermissionsRequestApprovalParams","McpServerElicitationRequestParams"]),
];

fn schema_names(ours:&str) -> Vec<String> {
    LOCAL_NAMES.iter().find(|(name,_)|*name == ours).map_or_else(||vec![ours.to_owned()],|(_,names)|names.iter().map(|n|(*n).to_owned()).collect())
}

fn ours() -> Vec<Value> {
    macro_rules! s { ($($t:ty),*) => { vec![$(serde_json::to_value(schemars::schema_for!($t)).unwrap()),*] } }
    s!(ClientRequest,ServerNotification,ServerRequest,InitializeResponse,ThreadStartResponse,ThreadReadResponse,
       TurnStartResponse,ModelListResponse,GetAccountRateLimitsResponse)
}

/// Definições nomeadas dos nossos tipos: nome → propriedades usadas.
fn our_defs() -> BTreeMap<String,BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for root in ours() {
        let mut defs:Vec<(String,Value)> = root["$defs"].as_object().cloned().unwrap_or_default().into_iter().collect();
        if let Some(title) = root["title"].as_str() { defs.push((title.into(),root.clone())); }
        for (name,def) in defs {
            if let Some(props) = def["properties"].as_object() {
                out.entry(name).or_insert_with(BTreeSet::new).extend(props.keys().cloned());
            }
        }
    }
    out
}

/// Nossos ramos com tag (`type`/`method`): (enum, valor da tag) → propriedades além da tag.
fn our_tagged() -> BTreeMap<(String,String),BTreeSet<String>> {
    let mut out = BTreeMap::new();
    for root in ours() {
        let mut defs:Vec<(String,Value)> = root["$defs"].as_object().cloned().unwrap_or_default().into_iter().collect();
        if let Some(title) = root["title"].as_str() { defs.push((title.into(),root.clone())); }
        for (name,def) in defs {
            for branch in def["oneOf"].as_array().into_iter().chain(def["anyOf"].as_array()).flatten() {
                for tag in ["type","method"] {
                    let value = &branch["properties"][tag];
                    let Some(value) = value["const"].as_str().or_else(||value["enum"][0].as_str()) else { continue };
                    if value == "Unknown" { continue; }
                    let props = branch["properties"].as_object().map(|p|p.keys().filter(|k|*k != tag).cloned().collect()).unwrap_or_default();
                    out.insert((name.clone(),value.to_owned()),props);
                }
            }
        }
    }
    out
}

fn slice() -> Value { serde_json::from_str(SLICE).expect("recorte inválido") }

fn their_props(def:&Value) -> BTreeSet<String> {
    def["properties"].as_object().map(|p|p.keys().cloned().collect()).unwrap_or_default()
}

fn their_branch<'a>(def:&'a Value,tag:&str,value:&str) -> Option<&'a Value> {
    def["oneOf"].as_array().into_iter().chain(def["anyOf"].as_array()).flatten().find(|branch|{
        let v = &branch["properties"][tag];
        v["const"] == value || v["enum"].as_array().is_some_and(|e|e.iter().any(|x|x == value))
    })
}

#[test]
fn fields_used_exist_in_checked_schema() {
    let slice = slice();
    assert_eq!(slice["version"],crate::version::CHECKED);
    let defs = &slice["definitions"];
    let mut missing = Vec::new();
    for (ours,props) in our_defs() {
        for name in schema_names(&ours) {
            let def = &defs[&name];
            if def.is_null() { missing.push(format!("{name} (definição)")); continue; }
            let theirs = their_props(def);
            for prop in &props { if !theirs.contains(prop) { missing.push(format!("{name}.{prop}")); } }
        }
    }
    for ((name,value),props) in our_tagged() {
        if ["ClientRequest","ServerNotification","ServerRequest"].contains(&name.as_str()) {
            let methods = slice["methods"][&name].as_array().cloned().unwrap_or_default();
            if !methods.iter().any(|m|m == value.as_str()) { missing.push(format!("{name} método {value}")); }
            continue;
        }
        match their_branch(&defs[&name],"type",&value) {
            None => missing.push(format!("{name} tipo {value}")),
            Some(branch) => { let theirs = their_props(branch); for p in props { if !theirs.contains(&p) { missing.push(format!("{name}::{value}.{p}")); } } }
        }
    }
    assert!(missing.is_empty(),"campos usados pelo Hangar que não existem no Codex {}: {missing:#?}",crate::version::CHECKED);
}

/// `CODEX_SCHEMA_DIR=<saída do generate-json-schema --experimental> cargo test -p hangar-codex -- --ignored regenerate_slice`
#[test]
#[ignore]
fn regenerate_slice() {
    let dir = std::path::PathBuf::from(std::env::var("CODEX_SCHEMA_DIR").expect("CODEX_SCHEMA_DIR"));
    // v2 vence a raiz, que vence v1: o mesmo nome aparece em mais de uma pasta.
    let mut all:Map<String,Value> = Map::new();
    let mut methods = json!({});
    for sub in ["v2","","v1"] {
        let mut files:Vec<_> = std::fs::read_dir(dir.join(sub)).unwrap().flatten().map(|e|e.path())
            .filter(|p|p.extension().is_some_and(|e|e == "json") && !p.file_name().unwrap().to_string_lossy().starts_with("codex_app_server_protocol")).collect();
        files.sort();
        for file in files {
            let value:Value = serde_json::from_slice(&std::fs::read(&file).unwrap()).unwrap();
            for (name,def) in value["definitions"].as_object().cloned().unwrap_or_default() { all.entry(name).or_insert(def); }
            if let Some(title) = value["title"].as_str() {
                let mut root = value.clone(); root.as_object_mut().unwrap().remove("definitions");
                if ["ClientRequest","ServerNotification","ServerRequest"].contains(&title) && sub.is_empty() {
                    methods[title] = json!(root["oneOf"].as_array().unwrap().iter().filter_map(|b|b["properties"]["method"]["enum"][0].as_str()).collect::<Vec<_>>());
                }
                all.entry(title.to_owned()).or_insert(root);
            }
        }
    }
    let mut wanted:Vec<String> = our_defs().into_keys().flat_map(|n|schema_names(&n)).collect();
    wanted.extend(our_tagged().into_keys().map(|(n,_)|n).filter(|n|!["ClientRequest","ServerNotification","ServerRequest"].contains(&n.as_str())));
    let mut out = Map::new();
    while let Some(name) = wanted.pop() {
        if out.contains_key(&name) { continue; }
        let Some(def) = all.get(&name).cloned() else { continue };
        let text = def.to_string();
        for part in text.split("\"$ref\":\"").skip(1) {
            if let Some(reference) = part.split('"').next() { wanted.push(reference.rsplit('/').next().unwrap().to_owned()); }
        }
        out.insert(name,def);
    }
    let slice = json!({"version":crate::version::CHECKED,"definitions":out,"methods":methods});
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("schema").join(format!("{}.json",crate::version::CHECKED));
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path,serde_json::to_string_pretty(&slice).unwrap() + "\n").unwrap();
}
```

- [ ] **Step 2: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --lib schema_check`
Expected: FAIL de compilação (`schema/0.159.3.json` não existe).

- [ ] **Step 3: Script de conferência**

`scripts/conferir-codex-schema` (executável, `chmod +x`):

```bash
#!/usr/bin/env bash
# Refaz o recorte do schema do Codex com o binário instalado e mostra o que mudou nos campos usados.
set -euo pipefail
raiz="$(cd "$(dirname "$0")/.." && pwd)"
conferida="$(grep -oE '"[0-9]+\.[0-9]+\.[0-9]+[^"]*"' "$raiz/crates/hangar-codex/src/version.rs" | head -1 | tr -d '"')"
instalada="$(codex --version | awk '{print $2}')"
if [ "$instalada" != "$conferida" ]; then
  echo "Codex instalado: $instalada; conferida no código: $conferida."
  echo "Para conferir $instalada: troque o valor de checked_version!() em crates/hangar-codex/src/version.rs e rode de novo."
  exit 1
fi
tmp="$(mktemp -d)"; trap 'rm -rf "$tmp"' EXIT
codex app-server generate-json-schema --experimental --out "$tmp" >/dev/null
(cd "$raiz/crates" && CODEX_SCHEMA_DIR="$tmp" CARGO_BUILD_JOBS=4 cargo test -q -p hangar-codex --lib -- --ignored regenerate_slice)
(cd "$raiz/crates" && CARGO_BUILD_JOBS=4 cargo test -q -p hangar-codex --lib schema_check::fields_used_exist_in_checked_schema)
git -C "$raiz" diff --stat -- crates/hangar-codex/schema
```

- [ ] **Step 4: Gerar o recorte**

Run: `scripts/conferir-codex-schema`
Expected: cria `crates/hangar-codex/schema/0.159.3.json` e o teste passa. Se o teste listar
campos faltando, o nome na Task 2 está errado: corrigir o tipo (nunca o recorte à mão). O recorte
deve ficar bem abaixo dos 736 KB do arquivo inteiro; anotar o tamanho na mensagem de commit.

Se a máquina tiver outro Codex que não o 0.159.3: o script recusa. Trocar `checked_version!()`
para a versão instalada, rodar de novo e anotar a troca no commit.

- [ ] **Step 5: Provar que o teste pega rename**

Temporariamente, em `proto.rs`, renomear `pub effort:Option<Option<String>>` de
`ThreadSettingsUpdateParams` para `pub reasoning_effort:…`. Rodar
`cargo test -p hangar-codex --lib schema_check` — esperado FAIL listando
`ThreadSettingsUpdateParams.reasoningEffort`. Desfazer.

- [ ] **Step 6: Commit**

```bash
git add crates/hangar-codex/src/schema_check.rs crates/hangar-codex/schema scripts/conferir-codex-schema
git commit -m "test(codex): check used protocol fields against the checked schema slice"
```

---

### Task 4: Motor sobre os tipos, aviso de versão e falha de decodificação

**Files:**
- Modify: `crates/hangar-server/src/runtime/protocol.rs` (`Effect`)
- Modify: `crates/hangar-server/src/runtime/codex.rs`
- Modify: `crates/hangar-server/src/runtime/actor.rs:22-36` e o laço de efeitos (~`:547-614`)
- Modify: `crates/hangar-server/tests/runtime_codex.rs`
- Modify: `messages/pt.json`, `messages/en.json`, `frontend/src/lib/problema.ts`, `mobile/src/chat/SessionProblem.tsx`
- Modify: `docs/decisoes/harnesses.md`

**Interfaces:**
- Consumes: `hangar_codex::proto::*`, `hangar_codex::version::*` (Task 2).
- Produces: `Effect::Diag { event:DiagEvent, code:String }` e
  `enum DiagEvent { CodexVersion, CodexDecode }` em `runtime/protocol.rs`, com
  `DiagEvent::event(&self)->&'static str` e `DiagEvent::reason(&self)->&'static str`;
  problema `codex_versao_nao_conferida`.

- [ ] **Step 1: Testes novos do motor (falham)**

No fim de `crates/hangar-server/tests/runtime_codex.rs`:

```rust
fn diags(effects:&[Effect]) -> Vec<(DiagEvent,String)> {
    effects.iter().filter_map(|e|match e { Effect::Diag { event,code }=>Some((*event,code.clone())),_=>None }).collect()
}

#[test]
fn other_codex_version_warns_once_on_initialize() {
    let mut engine = Engine::new(json!({"name":"session","thread_id":"thread-1","headless":true}),1,clock(10.0));
    let effects = engine.bootstrap(true,"boot".into()).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    let effects = line(&mut engine,json!({"id":id,"result":{"userAgent":"hangar/9.1.0 (x)"}}),11.0);
    assert_eq!(diags(&effects),vec![(DiagEvent::CodexVersion,"codex_9_1".into())]);
    assert_eq!(engine.view()["problema"],"codex_versao_nao_conferida");
    assert!(engine.view()["problema_detalhe"].as_str().unwrap().contains("9.1.0"));
}

#[test]
fn checked_codex_version_is_silent() {
    let mut engine = Engine::new(json!({"name":"session","thread_id":"thread-1","headless":true}),1,clock(10.0));
    let effects = engine.bootstrap(true,"boot".into()).unwrap();
    let id = frames(&effects)[0]["id"].clone();
    let ua = format!("hangar/{} (x)",hangar_codex::version::CHECKED);
    let effects = line(&mut engine,json!({"id":id,"result":{"userAgent":ua}}),11.0);
    assert!(diags(&effects).is_empty());
    assert!(engine.view()["problema"].is_null());
}

#[test]
fn notification_with_wrong_type_is_dropped_and_reported() {
    let mut engine = engine();
    line(&mut engine,json!({"method":"turn/started","params":{"threadId":"thread-1","turn":{"id":"turn-1"}}}),10.0);
    let effects = line(&mut engine,json!({"method":"item/agentMessage/delta","params":{"threadId":"thread-1","turnId":"turn-1","delta":5}}),11.0);
    assert_eq!(diags(&effects),vec![(DiagEvent::CodexDecode,"item_agentmessage_delta".into())]);
    assert!(effects.iter().any(|e|matches!(e,Effect::Policy { kind,.. } if kind == "unknown_private")));
    let effects = line(&mut engine,json!({"method":"turn/completed","params":{"threadId":"thread-1","turn":{"id":"turn-1","status":"completed"}}}),12.0);
    assert!(effects.iter().any(|e|matches!(e,Effect::WakeQueue)));
    assert_eq!(engine.view()["state"],"idle");
}

#[test]
fn unknown_notification_is_silent() {
    let mut engine = engine();
    let effects = line(&mut engine,json!({"method":"thread/novidade","params":{"threadId":"thread-1"}}),10.0);
    assert!(diags(&effects).is_empty());
    assert!(effects.is_empty());
}

#[test]
fn unknown_server_request_still_gets_method_not_found() {
    let mut engine = engine();
    let effects = line(&mut engine,json!({"id":77,"method":"foo/bar","params":{"threadId":"thread-1"}}),10.0);
    let reply = frames(&effects).into_iter().find(|f|f["id"] == 77).unwrap();
    assert_eq!(reply["error"]["code"],-32601);
    assert!(reply["error"]["message"].as_str().unwrap().contains("foo/bar"));
}
```

E no topo do arquivo, acrescentar `DiagEvent` ao `use` de `protocol::*` (já coberto pelo glob).

- [ ] **Step 2: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex`
Expected: FAIL de compilação (`Effect::Diag`, `DiagEvent`).

- [ ] **Step 3: `Effect::Diag`**

Em `runtime/protocol.rs`, no `enum Effect`, antes de `Stop`:

```rust
    /// Linha no diário exportável (`/internal/diag`), uma por minuto por código.
    Diag { event: DiagEvent, code: String },
```

e logo depois do `enum Effect`:

```rust
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DiagEvent { CodexVersion, CodexDecode }

impl DiagEvent {
    pub fn event(self) -> &'static str { match self { Self::CodexVersion => "rust.codex_version", Self::CodexDecode => "rust.codex_decode" } }
    pub fn reason(self) -> &'static str {
        match self {
            Self::CodexVersion => "versão do Codex diferente da conferida; campo renomeado pode faltar",
            Self::CodexDecode => "notificação do Codex com formato inesperado foi ignorada",
        }
    }
}
```

- [ ] **Step 4: O ator manda ao diário**

Em `runtime/actor.rs`, no `PolicyClient`: campo `diag:crate::diag::DiagClient`, criado em `new`
com `crate::diag::DiagClient::new(upstream,secret.clone())`. No laço de efeitos, ao lado de
`Effect::Stop`:

```rust
                Effect::Diag { event,code } => {
                    // Versão é do servidor, não da sessão: sem nome, o limite de 1/min vale para todos.
                    let session = if event == DiagEvent::CodexVersion { "" } else { target.name.as_str() };
                    engine.policy.diag.report(event.event(),session,&code,event.reason());
                }
```

Conferir com `rg -n "match effect|Effect::Stop" crates/hangar-server/src` se há outro `match`
exaustivo de `Effect`; acrescentar o braço onde houver (teste incluso).

- [ ] **Step 5: Saída tipada no motor**

Em `runtime/codex.rs`, acrescentar:

```rust
use hangar_codex::proto::{self as wire,ClientRequest};

impl Engine {
    fn send(&mut self,operation_id:String,request:ClientRequest,continuation:Option<Value>,effects:&mut Vec<Effect>) {
        let (method,params) = request.into_parts();
        self.rpc(operation_id,method,params,continuation,effects);
    }
}
```

e trocar cada `self.rpc(…, "<método>", json!({…}), …)` pelo `self.send(…, ClientRequest::…, …)`
correspondente, mantendo as mesmas chaves no fio (o teste `client_requests_keep_the_wire_shape`
da Task 2 e os testes do motor conferem). Mapa:

| Chamada hoje | Vira |
|---|---|
| `"initialize"` | `ClientRequest::Initialize(wire::InitializeParams { client_info:wire::ClientInfo { name:"hangar".into(),title:None,version:"0.1.0".into() },capabilities:wire::InitializeCapabilities { experimental_api:true } })` |
| `"thread/resume"` (bootstrap, verificação do Fast) | `ClientRequest::ThreadResume(wire::ThreadResumeParams { thread_id })` |
| `"thread/start"` | `ClientRequest::ThreadStart(wire::ThreadStartParams { cwd:self.metadata["cwd"].as_str().map(Into::into),model:self.model.clone(),approval_policy:Some(approval(..).into()),sandbox:Some(sandbox(..).into()),service_tier:self.service_tier.clone() })` |
| `"thread/settings/update"` com `serviceTier` | `ThreadSettingsUpdate { thread_id, service_tier:Some(tier), ..Default::default() }` |
| idem com `model`+`effort` (`SetModel`/`SetEffort`) | `{ thread_id, model:Some(<payload ou self.model>), effort:Some(<payload ou self.effort>), .. }` (dentro do `Some`, `None` sai `null`, como hoje) |
| idem com `effort` (bootstrap) | `{ thread_id, effort:Some(Some(effort)), .. }` |
| idem com `collaborationMode` (`set_mode`) | `{ thread_id, collaboration_mode:Some(wire::CollaborationMode { mode, settings:wire::CollaborationSettings { model, reasoning_effort:self.effort.clone(), developer_instructions:None } }), .. }` |
| `"turn/start"` (Input) | `TurnStart { thread_id, approval_policy:Some(..), summary:Some("detailed".into()), input }` |
| `"turn/start"` (resposta assíncrona) | `TurnStart { thread_id, approval_policy:None, summary:None, input:vec![json!({"type":"text","text":text})] }` |
| `"turn/steer"` | `TurnSteer { thread_id, expected_turn_id, input }` |
| `"turn/interrupt"` | `TurnInterrupt { thread_id, turn_id }` |
| `"thread/read"` | `ThreadRead { thread_id, include_turns }` |
| `"thread/compact/start"` | `ThreadCompactStart { thread_id }` |
| `"model/list"` | `ModelList(Default::default())` |
| `"skills/list"` | `SkillsList { cwds:vec![self.metadata["cwd"].as_str().unwrap_or_default().into()] }` |
| `"account/rateLimits/read"` | `AccountRateLimitsRead` |
| `"thread/backgroundTerminals/terminate"` | `ThreadBackgroundTerminalsTerminate { thread_id, process_id }` |
| `"thread/unsubscribe"` (`close_voice_thread`, `VoiceClose`) | `ThreadUnsubscribe { thread_id }` |

O `VoiceRpc` continua repassando `method`/`params` crus: é um túnel da voz, e os pedidos dela
passam pelo filtro de métodos que já existe. A notificação `initialized` continua `json!`.

- [ ] **Step 6: Entrada tipada: notificações**

No começo de `fn notification`, depois do bloco da voz e do desvio de outra thread (que seguem
lendo `params["threadId"]` cru, porque roteiam linhas inteiras), e depois do ramo de pedidos do
servidor (`line.get("id")`), trocar o `match method { … }` por:

```rust
        let notification = match wire::ServerNotification::decode(method,params) {
            Ok(notification) => notification,
            Err(failure) => {
                // Formato inesperado num método conhecido: a linha é ignorada e fica registrada.
                tracing::warn!(session=%self.state.session,method=%failure.method,error=wire::error_kind(&failure.error),"notificação do Codex fora do formato");
                effects.push(Effect::Diag { event:DiagEvent::CodexDecode,code:decode_code(&failure.method) });
                self.policy("unknown_private",json!({"kind":format!("decode:{}",failure.method),"event":line}),effects);
                return Ok(());
            }
        };
        use wire::ServerNotification as N;
        match notification {
            N::ServerRequestResolved(n) => {
                let Some(request_id) = n.request_id else { return Ok(()) };
                let notice = self.server_requests.iter().find(|(key,_)|key == &request_id).and_then(|(_,request)|unsupported_notice(request));
                if let Some(text) = notice { self.policy("local_output",json!({"text":text}),effects); }
                self.server_requests.retain(|(id,_)|id != &request_id); self.answering.remove(&request_id); self.request_epochs.remove(&request_id);
            }
            N::TurnStarted(n) => {
                self.in_progress = true; self.turn_id = Some(n.turn.id).filter(|id|!id.is_empty()); self.state_revision += 1;
                self.first_response_start = self.turn_id.clone().map(|id|(id,self.clock.monotonic_s));
                self.response_started = false; self.state.codex_buffering = false; self.clear_preview(effects);
                self.state.problema = None; self.state.problema_detalhe = None;
                self.running_commands.clear();
            }
            N::TurnCompleted(n) => {
                if !n.turn.id.is_empty() && self.turn_id.as_deref().is_some_and(|current|current != n.turn.id) { return Ok(()); }
                self.in_progress = false; self.turn_id = None; self.state_revision += 1; self.compacting = false;
                self.first_response_start = None;
                self.state.codex_buffering = false; self.response_started = false; self.clear_preview(effects);
                self.server_requests.clear(); self.answering.clear(); self.request_epochs.clear();
                if n.turn.status == "failed" {
                    let error = n.turn.error.unwrap_or_default();
                    let class = error.codex_error_info.as_ref().and_then(error_class);
                    if class.is_some() || !matches!(self.state.problema.as_deref(),Some("codex_limite_uso" | "codex_sem_login")) {
                        self.state.problema = Some(class.unwrap_or("headless_turno_erro").into());
                        self.state.problema_detalhe = Some(error.message).filter(|m|!m.is_empty());
                    }
                } else if retry_problem(self.state.problema.as_deref()) { self.state.problema = None; self.state.problema_detalhe = None; }
                self.changed(effects,true); effects.push(Effect::WakeQueue); return Ok(());
            }
            N::ThreadStatusChanged(n) => {
                match n.status {
                    wire::ThreadStatus::Active => self.in_progress = true,
                    wire::ThreadStatus::Idle => { self.in_progress = false; self.turn_id = None; self.state.codex_buffering = false; },
                    _ => return Ok(()),
                }
                self.state_revision += 1;
            }
            N::ThreadSettingsUpdated(n) => {
                if n.thread_id != self.thread_id { return Ok(()); }
                let settings = n.thread_settings;
                if let Some(model) = settings.model { self.model = Some(model); }
                if let Some(effort) = settings.effort { self.effort = effort; }
                if let Some(mode) = settings.collaboration_mode { self.mode = Some(mode.map_or_else(||"default".into(),|m|m.mode)); }
                self.settings_revision += 1;
                if let Some(tier) = settings.service_tier.map(|t|t.unwrap_or_else(||"default".into())).filter(|t|["priority","default"].contains(&t.as_str())) {
                    self.service_tier = Some(tier.clone());
                    self.policy("session.patch_meta",json!({"service_tier":tier}),effects);
                    if let Some(pending) = self.service_tier_pending.as_mut() { if pending.tier == tier { pending.candidate = true; } }
                    self.verify_service_tier(effects);
                }
            }
            N::AgentMessageDelta(n) => {
                if !n.turn_id.is_empty() && self.turn_id.as_deref().is_some_and(|current|current != n.turn_id) { return Ok(()); }
                if !n.delta.is_empty() && self.first_response_start.as_ref().is_some_and(|(id,_)|*id == n.turn_id) {
                    if let Some((_,started)) = self.first_response_start.take() {
                        effects.push(Effect::Publish { channel:"rate".into(),data:json!({"first_response":true,
                            "seconds":self.clock.monotonic_s-started,"conversation":self.thread_id}) });
                    }
                }
                self.response_started = true; self.state.codex_buffering = false;
                if let Some(text) = self.preview.append(&n.delta,self.clock.monotonic_s) { self.publish(text,effects); }
                return Ok(());
            }
            N::ReasoningSummaryTextDelta(wire::ReasoningSummaryTextDeltaNotification { turn_id,delta,.. })
            | N::ReasoningTextDelta(wire::ReasoningTextDeltaNotification { turn_id,delta,.. }) => {
                if !turn_id.is_empty() && self.turn_id.as_deref().is_some_and(|current|current != turn_id) { return Ok(()); }
                if let Some(text) = self.thinking.append(&delta,self.clock.monotonic_s) { self.publish_on("thinking",text,effects); }
                return Ok(());
            }
            N::ReasoningSummaryPartAdded(n) => {
                if !n.turn_id.is_empty() && self.turn_id.as_deref().is_some_and(|current|current != n.turn_id) { return Ok(()); }
                let piece = if n.summary_index > 0 { "\n\n" } else { "" };
                if let Some(text) = self.thinking.append(piece,self.clock.monotonic_s) { self.publish_on("thinking",text,effects); }
                return Ok(());
            }
            N::ModelRerouted(n) => {
                if n.thread_id != self.thread_id { return Ok(()); }
                self.model = n.to_model.or(self.model.clone());
            }
            N::ItemStarted(n) | N::ItemCompleted(n) => {
                let started = method == "item/started";
                self.async_questions.observe(&self.thread_id,&n.item,&params["item"]);
                match &n.item {
                    wire::ThreadItem::ContextCompaction { .. } => self.compacting = started,
                    wire::ThreadItem::AgentMessage { .. } => self.clear_preview(effects),
                    wire::ThreadItem::Reasoning { .. } if started => {
                        if let Some(text) = self.thinking.clear() { self.publish_on("thinking",text,effects); }
                    }
                    wire::ThreadItem::CommandExecution { id,process_id } => match (started,process_id,n.turn_id.is_empty()) {
                        (true,Some(process),false) => { self.running_commands.insert(id.clone(),(n.turn_id.clone(),process.clone())); }
                        _ => { self.running_commands.remove(id); }
                    },
                    _ => {},
                }
                if !matches!(n.item,wire::ThreadItem::UserMessage { .. }) && retry_problem(self.state.problema.as_deref()) {
                    self.state.problema = None; self.state.problema_detalhe = None;
                }
            }
            N::ModelSafetyBufferingUpdated(n) => {
                if !self.in_progress || self.response_started || n.thread_id != self.thread_id { return Ok(()); }
                if self.turn_id.as_deref().is_some_and(|turn|n.turn_id != turn) { return Ok(()); }
                if let Some(buffering) = n.show_buffering_ui { self.state.codex_buffering = buffering; }
            }
            N::ThreadTokenUsageUpdated(_) => { if params["tokenUsage"].is_object() { self.token_usage = params["tokenUsage"].clone(); } }
            N::AccountRateLimitsUpdated(n) => {
                if n.rate_limits.as_ref().is_some_and(|r|r.limit_id.as_deref().is_none_or(|id|id == "codex")) { self.rate_limits = params["rateLimits"].clone(); }
                else { return Ok(()); }
            }
            N::Error(n) => {
                self.state.problema = Some(n.error.codex_error_info.as_ref().and_then(error_class)
                    .unwrap_or(if n.will_retry { "codex_sem_conexao" } else { "headless_turno_erro" }).into());
                self.state.problema_detalhe = Some(n.error.message).filter(|m|!m.is_empty());
            }
            N::HookCompleted(n) if n.run.event_name == "userPromptSubmit" && ["blocked","stopped"].contains(&n.run.status.as_str()) => {
                let source = n.run.source_path.as_deref().unwrap_or("hook");
                let normalized = source.replace('\\',"/");
                let parts:Vec<_> = normalized.split('/').collect();
                let origin = parts.iter().position(|part|*part == "cache").and_then(|index|parts.get(index+2)).copied().unwrap_or(source);
                let reason = n.run.entries.iter().find(|entry|["stop","feedback","error"].contains(&entry.kind.as_str()) && entry.text.is_some())
                    .and_then(|entry|entry.text.as_deref()).unwrap_or("");
                self.state.problema = Some("codex_prompt_bloqueado".into());
                self.state.problema_detalhe = Some(format!("{origin}: {reason}").trim_matches([' ',':']).chars().take(300).collect());
            }
            _ => return Ok(()),
        }
        self.changed(effects,true);
        Ok(())
```

com, no nível do módulo:

```rust
/// `item/agentMessage/delta` → `item_agentmessage_delta` (o diário aceita `[a-z0-9_]{1,64}`).
fn decode_code(method:&str) -> String {
    method.chars().map(|c|if c.is_ascii_alphanumeric() { c.to_ascii_lowercase() } else { '_' }).take(64).collect()
}

fn error_class(info:&Value) -> Option<&'static str> {
    let info = info.as_str().or_else(||info.as_object().and_then(|fields|fields.keys().next()).map(String::as_str))?;
    match info { "usageLimitExceeded" | "rateLimitExceeded"=>Some("codex_limite_uso"),"unauthorized"=>Some("codex_sem_login"),_=>None }
}
```

(o `error_class` antigo recebia o objeto `error` e lia `codexErrorInfo`; os outros chamadores
passam a mandar `&error["codexErrorInfo"]` ou o campo tipado.)

Pontos que mudam de forma, com o mesmo efeito:
- `token_usage` e `rate_limits` continuam guardados como `Value` (vão crus ao `format_status`,
  que a 5-0 leva para o Rust); o tipo só confere o formato.
- `AsyncQuestions::observe` recebe o item tipado e o cru: usa o tipado para decidir
  (`ThreadItem::AgentMessage { delivery:Some("async"),questions,.. }` com perguntas,
  `ThreadItem::UserMessage { content,.. }` para o texto) e guarda o cru em `during_load` (o
  snapshot grava como veio). Assinatura nova:
  `fn observe(&mut self,thread:&str,item:&wire::ThreadItem,raw:&Value)`; o `hydrate` decodifica
  cada item cru com `serde_json::from_value::<wire::ThreadItem>(item.clone()).unwrap_or(wire::ThreadItem::Unknown)`
  e chama `observe(thread,&typed,item)`.

- [ ] **Step 7: Entrada tipada: respostas e aviso de versão**

Em `fn reply`, no `match rpc.method.as_str()`:

```rust
            "initialize" => {
                self.initialized = true;
                let response:wire::InitializeResponse = serde_json::from_value(result.clone()).unwrap_or_default();
                if let Some(installed) = hangar_codex::version::from_user_agent(&response.user_agent) {
                    if hangar_codex::version::differs(installed) {
                        effects.push(Effect::Diag { event:DiagEvent::CodexVersion,code:hangar_codex::version::diag_code(installed) });
                        self.state.problema = Some("codex_versao_nao_conferida".into());
                        self.state.problema_detalhe = Some(format!("instalado {installed}, conferido {}",hangar_codex::version::CHECKED));
                    }
                }
            }
            "thread/resume" | "thread/start" => {
                let response:wire::ThreadStartResponse = serde_json::from_value(result.clone()).unwrap_or_default();
                if !response.thread.id.is_empty() && response.thread.id != self.thread_id {
                    self.finish_service_tier(Disposition::Unknown,json!({"error":"A conversa mudou antes de confirmar Fast"}),effects);
                    self.clear_preview(effects); self.thread_id = response.thread.id.clone();
                }
                if rpc.settings_revision == self.settings_revision {
                    self.model = response.model.clone().or(self.model.clone());
                    self.effort = response.reasoning_effort.clone().or(self.effort.clone());
                    self.restore_service_tier(&result,rpc.settings_revision,effects);
                }
                self.restore_thread(&response.thread,&rpc);
                self.async_questions.hydrate(&self.thread_id,&result["thread"]);
                self.policy("session.patch_meta",json!({"thread_id":self.thread_id,"rollout_path":response.thread.path}),effects);
            }
            "turn/start" => {
                if rpc.state_revision == self.state_revision {
                    let response:wire::TurnStartResponse = serde_json::from_value(result.clone()).unwrap_or_default();
                    self.turn_id = Some(response.turn.id).filter(|id|!id.is_empty()); self.in_progress = true; self.state_revision += 1;
                }
            }
            "thread/read" => {
                let response:wire::ThreadReadResponse = serde_json::from_value(result.clone()).unwrap_or_default();
                self.restore_thread(&response.thread,&rpc);
                if rpc.params["includeTurns"] == true { self.async_questions.hydrate(&self.thread_id,&result["thread"]); }
            }
            "account/rateLimits/read" => {
                let response:wire::GetAccountRateLimitsResponse = serde_json::from_value(result.clone()).unwrap_or_default();
                if response.rate_limits.limit_id.as_deref().is_none_or(|id|id == "codex") { self.rate_limits = result["rateLimits"].clone(); }
            }
```

(`thread/compact/start` e `thread/settings/update` ficam como estão: leem `rpc.params`, que é o
que o próprio Hangar mandou.) `restore_thread` passa a receber `&wire::Thread`:

```rust
    fn restore_thread(&mut self,thread:&wire::Thread,rpc:&Rpc) {
        if rpc.state_revision == self.state_revision {
            match thread.status {
                wire::ThreadStatus::Active => {
                    self.in_progress = true;
                    if rpc.params["includeTurns"] == true {
                        self.turn_id = thread.turns.iter().rev().find(|turn|turn.status == "inProgress").map(|turn|turn.id.clone());
                    }
                }
                wire::ThreadStatus::Idle => { self.in_progress = false; self.turn_id = None; self.state.codex_buffering = false; },
                _ => {},
            }
        }
        if rpc.settings_revision == self.settings_revision {
            if let Some(Some(model)) = &thread.model { self.model = Some(model.clone()); }
            if let Some(effort) = &thread.reasoning_effort { self.effort = effort.clone(); }
        }
    }
```

O `model/list` passa a montar a resposta a partir de `wire::ModelListResponse` (mesmas chaves de
saída de hoje: `model`, `displayName`, `description`, `efforts[{value,description}]`,
`defaultEffort`, `serviceTiers`, `defaultServiceTier`, sem os `hidden`). O `cut_check` lê
`wire::ThreadReadResponse` (`thread.status`, `thread.turns.last().status == "interrupted"`). O
catálogo do Fast (`service_tier_catalog`) lê `wire::ModelListResponse`
(`model.service_tiers` com `id == "priority"` e `hidden != true` — `hidden` do tier é lido do
`Value` de cada tier).

- [ ] **Step 8: Pedidos do servidor tipados na tela**

`view()` e `blocking_question()` passam a decodificar o pedido guardado:

```rust
    fn decoded(request:&Value) -> wire::ServerRequest {
        wire::ServerRequest::decode(request["method"].as_str().unwrap_or(""),&request["params"]).unwrap_or(wire::ServerRequest::Unknown)
    }
```

Na `view`: `wire::ServerRequest::CommandExecutionApproval(p)` monta
`` format!("Rodar `{}`{}",p.command.as_deref().unwrap_or("?"),p.cwd.map_or(…)) `` e
`FileChangeApproval(p)` o "Editar arquivos{ em grantRoot}"; o `reason` vem de `p.reason`. Em
`blocking_question`, `ToolRequestUserInput(p)` gera a mesma lista de hoje a partir de
`p.questions` (`id`, `header`, `question`, `isOther` padrão `false`, `isSecret` padrão `false`,
`options` padrão `[]`). O ramo de recusa em `notification` continua pelo nome do método (a lista
`["item/commandExecution/requestApproval","item/fileChange/requestApproval","item/tool/requestUserInput"]`);
a 5B troca isso pelos tipos.

- [ ] **Step 9: Aviso na tela**

`messages/pt.json`, perto de `problema_codex_sem_conexao`:

```json
  "problema_codex_versao_nao_conferida": "Este Codex é de uma versão que o Hangar ainda não conferiu; se faltar alguma informação na tela, é por isso",
```

`messages/en.json`:

```json
  "problema_codex_versao_nao_conferida": "This Codex is a version Hangar has not checked yet; if something is missing on screen, this is why",
```

`frontend/src/lib/problema.ts`: `case 'codex_versao_nao_conferida': return m.problema_codex_versao_nao_conferida();`
`mobile/src/chat/SessionProblem.tsx`: `codex_versao_nao_conferida: m.problema_codex_versao_nao_conferida,`
O nativo lê `problema_<código>` das mensagens sozinho (`desktop-native/src/app.rs:146`).

- [ ] **Step 10: Rodar e ver passar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --test runtime_codex --test runtime_contract --test runtime_actor && CARGO_BUILD_JOBS=4 cargo test -p hangar-server --lib diag`
Expected: PASS, inclusive `codex_public_events_match_the_python_oracle` sem mudar o golden. Front:
`cd frontend && npx vitest run src/lib/i18nGuard.test.ts`.

- [ ] **Step 11: Regra nova**

Em `docs/decisoes/harnesses.md`, "Regras vigentes", depois da regra "Codex sem terminal: o
app-server é do CANO":

```markdown
- **Protocolo do Codex no Rust é tipado e tolerante** (`crates/hangar-codex`): todo campo usado
  existe no recorte do schema da versão conferida (`schema/<versão>.json`, teste
  `schema_check`); campo novo é ignorado; formato inesperado num método conhecido descarta só
  aquela linha e vai ao diário (`rust.codex_decode`); outra major.minor no `initialize` vira
  `codex_versao_nao_conferida`. Atualizar a versão conferida: `scripts/conferir-codex-schema`.
```

- [ ] **Step 12: Commit**

```bash
git add crates/hangar-server/src/runtime/protocol.rs crates/hangar-server/src/runtime/codex.rs crates/hangar-server/src/runtime/actor.rs crates/hangar-server/tests/runtime_codex.rs messages/pt.json messages/en.json frontend/src/lib/problema.ts mobile/src/chat/SessionProblem.tsx docs/decisoes/harnesses.md
git commit -m "feat(codex): Rust engine reads and writes the typed protocol, warns on unchecked Codex versions"
```

---

### Task 5: Cliente JSON-RPC assíncrono (stdio e WebSocket)

**Files:**
- Modify: `crates/hangar-codex/src/client.rs`
- Create: `crates/hangar-codex/tests/client.rs`
- Modify: `docs/migracao-rust/README.md`

**Interfaces:**
- Consumes: `proto::{RequestId,ClientRequest,ServerNotification,ServerRequest}` (Task 2).
- Produces (para 5C/5E):
  - `pub struct Client` (`Clone`), `pub enum Incoming { Notification { method:String, params:Value }, Request { id:RequestId, method:String, params:Value } }`
  - `pub enum ClientError { Timeout, Closed, Rpc { code:i64, message:String }, Decode(String), Io(String) }`
  - `Client::over_lines(reader:impl AsyncRead+Unpin+Send+'static, writer:impl AsyncWrite+Unpin+Send+'static) -> (Client, mpsc::Receiver<Incoming>)`
  - `Client::spawn_stdio(command:tokio::process::Command) -> std::io::Result<(Client, mpsc::Receiver<Incoming>, tokio::process::Child)>`
  - `Client::connect_ws(url:&str) -> Result<(Client, mpsc::Receiver<Incoming>),ClientError>`
  - `async fn request<R:DeserializeOwned>(&self, request:ClientRequest, timeout:Duration) -> Result<R,ClientError>`
  - `async fn notify(&self, method:&str, params:Value) -> Result<(),ClientError>`
  - `async fn respond(&self, id:RequestId, outcome:Result<Value,(i64,String)>) -> Result<(),ClientError>`
  - `pub const MAX_LINE:usize = 16 * 1024 * 1024;` `pub const INCOMING_CAPACITY:usize = 1024;`

Quem recebe o `Receiver<Incoming>` tem de consumi-lo numa tarefa própria: a leitura espera
quando ele enche (canal com limite), e uma resposta atrás de notificações não lidas espera junto.

- [ ] **Step 1: Testes (falham)**

`crates/hangar-codex/tests/client.rs`:

```rust
use hangar_codex::client::*;
use hangar_codex::proto::*;
use serde_json::{Value,json};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt,AsyncWriteExt,BufReader};

/// Servidor falso no outro lado de um duplex: responde `model/list`, empurra uma notificação e um pedido.
async fn fake(stream:tokio::io::DuplexStream) {
    let (read,mut write) = tokio::io::split(stream);
    let mut lines = BufReader::new(read).lines();
    while let Ok(Some(line)) = lines.next_line().await {
        let msg:Value = serde_json::from_str(&line).unwrap();
        if msg["method"] == "model/list" {
            let out = [json!({"method":"turn/started","params":{"threadId":"t","turn":{"id":"u"}}}),
                json!({"id":"srv-1","method":"item/tool/requestUserInput","params":{"threadId":"t","questions":[]}}),
                json!({"id":msg["id"],"result":{"data":[{"model":"m"}]}})];
            for o in out { write.write_all(format!("{o}\n").as_bytes()).await.unwrap(); }
        } else if msg["method"] == "turn/interrupt" {
            write.write_all(format!("{}\n",json!({"id":msg["id"],"error":{"code":-32600,"message":"não"}})).as_bytes()).await.unwrap();
        } else if msg.get("result").is_some() {
            write.write_all(format!("{}\n",json!({"method":"echo","params":msg})).as_bytes()).await.unwrap();
        }
    }
}

#[tokio::test]
async fn request_notification_and_server_request() {
    let (ours,theirs) = tokio::io::duplex(1 << 16);
    tokio::spawn(fake(theirs));
    let (r,w) = tokio::io::split(ours);
    let (client,mut incoming) = Client::over_lines(r,w);
    let list:ModelListResponse = client.request(ClientRequest::ModelList(Default::default()),Duration::from_secs(5)).await.unwrap();
    assert_eq!(list.data[0].model,"m");
    assert!(matches!(incoming.recv().await,Some(Incoming::Notification { method,.. }) if method == "turn/started"));
    let Some(Incoming::Request { id,method,.. }) = incoming.recv().await else { panic!() };
    assert_eq!(method,"item/tool/requestUserInput");
    client.respond(id,Ok(json!({"answers":{}}))).await.unwrap();
    let Some(Incoming::Notification { params,.. }) = incoming.recv().await else { panic!() };
    assert_eq!(params["id"],"srv-1");
}

#[tokio::test]
async fn rpc_error_comes_back_typed() {
    let (ours,theirs) = tokio::io::duplex(1 << 16);
    tokio::spawn(fake(theirs));
    let (r,w) = tokio::io::split(ours);
    let (client,_incoming) = Client::over_lines(r,w);
    let err = client.request::<Value>(ClientRequest::TurnInterrupt(TurnInterruptParams { thread_id:"t".into(),turn_id:"u".into() }),Duration::from_secs(5)).await.unwrap_err();
    assert!(matches!(err,ClientError::Rpc { code:-32600,.. }));
}

#[tokio::test]
async fn server_dying_fails_pending_at_once() {
    let (ours,theirs) = tokio::io::duplex(1 << 16);
    let (r,w) = tokio::io::split(ours);
    let (client,mut incoming) = Client::over_lines(r,w);
    let call = tokio::spawn({ let client = client.clone(); async move {
        client.request::<Value>(ClientRequest::ModelList(Default::default()),Duration::from_secs(60)).await } });
    tokio::time::sleep(Duration::from_millis(50)).await;
    drop(theirs);
    let result = tokio::time::timeout(Duration::from_secs(2),call).await.expect("não pode esperar o prazo").unwrap();
    assert!(matches!(result,Err(ClientError::Closed)));
    assert!(incoming.recv().await.is_none());
}

#[tokio::test]
async fn timeout_is_reported() {
    let (ours,_theirs) = tokio::io::duplex(1 << 16);
    let (r,w) = tokio::io::split(ours);
    let (client,_incoming) = Client::over_lines(r,w);
    let err = client.request::<Value>(ClientRequest::ModelList(Default::default()),Duration::from_millis(50)).await.unwrap_err();
    assert!(matches!(err,ClientError::Timeout));
}

#[tokio::test]
async fn oversized_line_closes_the_connection() {
    let (ours,mut theirs) = tokio::io::duplex(1 << 20);
    let (r,w) = tokio::io::split(ours);
    let (_client,mut incoming) = Client::over_lines(r,w);
    tokio::spawn(async move {
        let chunk = vec![b'a';1 << 20];
        for _ in 0..(MAX_LINE / chunk.len() + 2) { if theirs.write_all(&chunk).await.is_err() { return; } }
    });
    assert!(tokio::time::timeout(Duration::from_secs(10),incoming.recv()).await.unwrap().is_none());
}

#[tokio::test]
async fn websocket_transport() {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    tokio::spawn(async move {
        use futures_util::{SinkExt,StreamExt};
        let (stream,_) = listener.accept().await.unwrap();
        let mut ws = tokio_tungstenite::accept_async(stream).await.unwrap();
        while let Some(Ok(message)) = ws.next().await {
            let Ok(text) = message.to_text() else { continue };
            let msg:Value = serde_json::from_str(text).unwrap();
            let reply = json!({"id":msg["id"],"result":{"userAgent":"x/0.159.3 (y)"}});
            ws.send(tokio_tungstenite::tungstenite::Message::text(reply.to_string())).await.unwrap();
        }
    });
    let (client,_incoming) = Client::connect_ws(&format!("ws://{address}")).await.unwrap();
    let init:InitializeResponse = client.request(ClientRequest::Initialize(Default::default()),Duration::from_secs(5)).await.unwrap();
    assert_eq!(init.user_agent,"x/0.159.3 (y)");
}
```


- [ ] **Step 2: Rodar e ver falhar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --test client`
Expected: FAIL de compilação (`Client` não existe).

- [ ] **Step 3: Implementar `client.rs`**

```rust
//! Cliente JSON-RPC do app-server do Codex. O núcleo fala por dois canais de texto (uma mensagem
//! JSON por item); stdio e WebSocket só convertem o transporte nesses canais.
use crate::proto::{ClientRequest,RequestId};
use serde::de::DeserializeOwned;
use serde_json::{Value,json};
use std::collections::HashMap;
use std::sync::{Arc,Mutex,atomic::{AtomicI64,Ordering}};
use std::time::Duration;
use tokio::io::{AsyncBufReadExt,AsyncRead,AsyncReadExt,AsyncWrite,AsyncWriteExt,BufReader};
use tokio::sync::{mpsc,oneshot};

pub const MAX_LINE:usize = 16 * 1024 * 1024;
pub const INCOMING_CAPACITY:usize = 1024;
const OUTGOING_CAPACITY:usize = 256;

#[derive(Debug)]
pub enum ClientError { Timeout, Closed, Rpc { code:i64, message:String }, Decode(String), Io(String) }

#[derive(Debug)]
pub enum Incoming { Notification { method:String, params:Value }, Request { id:RequestId, method:String, params:Value } }

type Pending = Arc<Mutex<Option<HashMap<RequestId,oneshot::Sender<Result<Value,ClientError>>>>>>;

#[derive(Clone)]
pub struct Client { out:mpsc::Sender<String>, pending:Pending, next:Arc<AtomicI64> }

impl Client {
    /// Núcleo: `lines_in` fecha quando a conexão acaba; aí todo pedido em voo falha com `Closed`.
    fn start(mut lines_in:mpsc::Receiver<String>,out:mpsc::Sender<String>) -> (Self,mpsc::Receiver<Incoming>) {
        let pending:Pending = Arc::new(Mutex::new(Some(HashMap::new())));
        let (tx,rx) = mpsc::channel(INCOMING_CAPACITY);
        let reader_pending = pending.clone();
        tokio::spawn(async move {
            while let Some(line) = lines_in.recv().await {
                let Ok(msg) = serde_json::from_str::<Value>(&line) else { continue };
                let id = msg.get("id").filter(|id|!id.is_null()).and_then(|id|serde_json::from_value::<RequestId>(id.clone()).ok());
                match (id,msg["method"].as_str()) {
                    (Some(id),Some(method)) => {
                        if tx.send(Incoming::Request { id,method:method.into(),params:msg["params"].clone() }).await.is_err() { break; }
                    }
                    (None,Some(method)) => {
                        if tx.send(Incoming::Notification { method:method.into(),params:msg["params"].clone() }).await.is_err() { break; }
                    }
                    (Some(id),None) => {
                        let waiter = reader_pending.lock().unwrap().as_mut().and_then(|map|map.remove(&id));
                        if let Some(waiter) = waiter {
                            let outcome = match msg.get("error").filter(|e|!e.is_null()) {
                                Some(error) => Err(ClientError::Rpc { code:error["code"].as_i64().unwrap_or(0),
                                    message:error["message"].as_str().unwrap_or("").into() }),
                                None => Ok(msg.get("result").cloned().unwrap_or(Value::Null)),
                            };
                            let _ = waiter.send(outcome);
                        }
                    }
                    (None,None) => {},
                }
            }
            // Conexão acabou: quem espera resposta sai agora, não no prazo.
            if let Some(map) = reader_pending.lock().unwrap().take() {
                for (_,waiter) in map { let _ = waiter.send(Err(ClientError::Closed)); }
            }
        });
        (Self { out,pending,next:Arc::new(AtomicI64::new(1)) },rx)
    }

    pub fn over_lines(reader:impl AsyncRead+Unpin+Send+'static,mut writer:impl AsyncWrite+Unpin+Send+'static) -> (Self,mpsc::Receiver<Incoming>) {
        let (lines_tx,lines_rx) = mpsc::channel::<String>(INCOMING_CAPACITY);
        tokio::spawn(async move {
            let mut reader = BufReader::new(reader);
            let mut buffer = Vec::new();
            loop {
                buffer.clear();
                // `take` limita a linha: acima do teto a conexão é encerrada.
                let read = (&mut reader).take(MAX_LINE as u64 + 1).read_until(b'\n',&mut buffer).await;
                match read {
                    Ok(0) | Err(_) => break,
                    Ok(_) if buffer.len() > MAX_LINE => { tracing::warn!("linha do app-server do Codex acima do teto; conexão encerrada"); break; }
                    Ok(_) => {
                        let Ok(text) = std::str::from_utf8(&buffer) else { continue };
                        let text = text.trim_end();
                        if !text.is_empty() && lines_tx.send(text.to_owned()).await.is_err() { break; }
                    }
                }
            }
        });
        let (out_tx,mut out_rx) = mpsc::channel::<String>(OUTGOING_CAPACITY);
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await {
                if writer.write_all(line.as_bytes()).await.is_err() || writer.write_all(b"\n").await.is_err() || writer.flush().await.is_err() { break; }
            }
        });
        Self::start(lines_rx,out_tx)
    }

    pub fn spawn_stdio(mut command:tokio::process::Command) -> std::io::Result<(Self,mpsc::Receiver<Incoming>,tokio::process::Child)> {
        command.stdin(std::process::Stdio::piped()).stdout(std::process::Stdio::piped()).kill_on_drop(true);
        let mut child = command.spawn()?;
        let stdout = child.stdout.take().ok_or_else(||std::io::Error::other("sem stdout"))?;
        let stdin = child.stdin.take().ok_or_else(||std::io::Error::other("sem stdin"))?;
        let (client,incoming) = Self::over_lines(stdout,stdin);
        Ok((client,incoming,child))
    }

    pub async fn connect_ws(url:&str) -> Result<(Self,mpsc::Receiver<Incoming>),ClientError> {
        use futures_util::{SinkExt,StreamExt};
        use tokio_tungstenite::tungstenite::{Message,protocol::WebSocketConfig};
        let config = WebSocketConfig::default().max_message_size(Some(MAX_LINE)).max_frame_size(Some(MAX_LINE));
        let (socket,_) = tokio_tungstenite::connect_async_with_config(url,Some(config),false).await.map_err(|e|ClientError::Io(e.to_string()))?;
        let (mut sink,mut stream) = socket.split();
        let (lines_tx,lines_rx) = mpsc::channel::<String>(INCOMING_CAPACITY);
        tokio::spawn(async move {
            while let Some(Ok(message)) = stream.next().await {
                match message {
                    Message::Text(text) => { if lines_tx.send(text.to_string()).await.is_err() { break; } }
                    Message::Close(_) => break,
                    _ => {},
                }
            }
        });
        let (out_tx,mut out_rx) = mpsc::channel::<String>(OUTGOING_CAPACITY);
        tokio::spawn(async move {
            while let Some(line) = out_rx.recv().await { if sink.send(Message::text(line)).await.is_err() { break; } }
            let _ = sink.close().await;
        });
        Ok(Self::start(lines_rx,out_tx))
    }

    pub async fn request<R:DeserializeOwned>(&self,request:ClientRequest,timeout:Duration) -> Result<R,ClientError> {
        let id = RequestId::Integer(self.next.fetch_add(1,Ordering::Relaxed));
        let (tx,rx) = oneshot::channel();
        match self.pending.lock().unwrap().as_mut() { Some(map) => { map.insert(id.clone(),tx); }, None => return Err(ClientError::Closed) }
        let (method,params) = request.into_parts();
        let line = json!({"jsonrpc":"2.0","id":id,"method":method,"params":params}).to_string();
        if self.out.send(line).await.is_err() { self.forget(&id); return Err(ClientError::Closed); }
        let value = match tokio::time::timeout(timeout,rx).await {
            Err(_) => { self.forget(&id); return Err(ClientError::Timeout); }
            Ok(Err(_)) => return Err(ClientError::Closed),
            Ok(Ok(outcome)) => outcome?,
        };
        serde_json::from_value(value).map_err(|e|ClientError::Decode(format!("{method}: {}",crate::proto::error_kind(&e))))
    }

    fn forget(&self,id:&RequestId) { if let Some(map) = self.pending.lock().unwrap().as_mut() { map.remove(id); } }

    pub async fn notify(&self,method:&str,params:Value) -> Result<(),ClientError> {
        self.out.send(json!({"jsonrpc":"2.0","method":method,"params":params}).to_string()).await.map_err(|_|ClientError::Closed)
    }

    pub async fn respond(&self,id:RequestId,outcome:Result<Value,(i64,String)>) -> Result<(),ClientError> {
        let line = match outcome {
            Ok(result) => json!({"jsonrpc":"2.0","id":id,"result":result}),
            Err((code,message)) => json!({"jsonrpc":"2.0","id":id,"error":{"code":code,"message":message}}),
        };
        self.out.send(line.to_string()).await.map_err(|_|ClientError::Closed)
    }
}

```

Nota para quem implementa:
- Se a assinatura de `connect_async_with_config`/`WebSocketConfig` da 0.29 diferir, ajuste pela
  doc da versão (`cargo doc -p tokio-tungstenite --open` não é preciso: `rg` no
  `~/.cargo/registry/src/*/tokio-tungstenite-0.29.0/src/lib.rs`).

- [ ] **Step 4: Rodar e ver passar**

Run: `cd crates && CARGO_BUILD_JOBS=4 cargo test -p hangar-codex --test client --lib`
Expected: PASS (6 testes do cliente + os da Task 2/3).

- [ ] **Step 5: README da migração**

Em `docs/migracao-rust/README.md`, linha da parte 5 na tabela "Partes": trocar `—` por
`**5A feita na hangar-server-parte5-codex** (tipos do protocolo em crates/hangar-codex, motor
tipado, aviso de versão, cliente stdio/WebSocket); 5B–5I na spec parte5-codex/spec.md`.

- [ ] **Step 6: Commit e limpeza**

```bash
git add crates/hangar-codex/src/client.rs crates/hangar-codex/tests/client.rs crates/hangar-codex/Cargo.toml crates/Cargo.lock docs/migracao-rust/README.md
git commit -m "feat(codex): async JSON-RPC client over stdio and WebSocket"
rm -rf crates/target
```

---

## Revisão final da 5A

Depois da Task 5, um revisor de contexto limpo lê o diff inteiro da branch contra a `main` com
foco em: paridade do motor (nenhum teste antigo mudou), a lista de Review Focus, desempenho
(nenhum clone novo de linha grande por notificação além do `decode`), e o que a 5B herda
(`ServerRequest::PermissionsApproval`/`McpServerElicitation` só reconhecidos). Não há uso real
nesta subparte: o motor só entra em produção na 5B.
