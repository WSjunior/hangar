mod fake;

use std::sync::Arc;

use fake::*;
use hangar_server::mods::bridge::mint;
use hangar_server::mods::model::*;
use hangar_server::mods::state::*;
use serde_json::{Value, json};

#[test]
fn mint_matches_python() {
    // python3 -c "import hmac,hashlib;print(hmac.new(b'dono-token',b'plugin:mods-s',hashlib.sha256).hexdigest()[:32])"
    assert_eq!(mint("dono-token", "mods-s"), "fe9b420d49e252b4c98ce09870cf7747");
    assert_eq!(mint("", "mods-s"), "1d50c42b88fb27d333798979117ec477", "sem token, o segredo é \"hangar\"");
}

/// O `reqwest` do crate não tem a função `json`: o corpo vai pronto, com o tipo.
async fn post(url: String, body: Value, owner: bool) -> reqwest::Response {
    let mut request = client().post(url).header("content-type", "application/json").body(body.to_string());
    if owner { request = request.header("authorization", format!("Bearer {OWNER}")); }
    request.send().await.unwrap()
}

async fn json_of(response: reqwest::Response) -> Value {
    serde_json::from_str(&response.text().await.unwrap()).unwrap()
}

/// O mod de um clique do app: o plugin do Hangar casa o press e manda a URL, como faz no `claude -p`
/// (Task 12) e fará na sessão com terminal da fase 3. Implementado direto no tipo local (A2): o `call`
/// só copia o endereço.
#[derive(Clone)]
struct PluginLike { server: Arc<std::sync::OnceLock<std::net::SocketAddr>> }
impl SurfaceLink for PluginLike {
    fn call(&self, _: ModsCall, _: std::time::Instant) -> CallFuture {
        let server = *self.server.get().unwrap();
        Box::pin(async move {
            let base = format!("http://{server}/api/plugin");
            let token = mint(OWNER, "mods-s");
            let start = json_of(post(format!("{base}/press-start"), json!({"sessao": "mods-s", "token": token,
                "requestId": "vitrine-botoes", "element": "V45-url"}), false).await).await;
            assert_eq!(start["fromApp"], true);
            let again = json_of(post(format!("{base}/press-start"), json!({"sessao": "mods-s", "token": token,
                "requestId": "vitrine-botoes", "element": "V45-url"}), false).await).await;
            assert_eq!(again["fromApp"], false, "uma vez só");
            let opened = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": token,
                "attempt": start["attempt"], "url": "https://example.com/vitrine"}), false).await;
            assert_eq!(opened.status().as_u16(), 200);
            Ok(json!({"element": "V45-url"}))
        })
    }
}

async fn setup() -> (Arc<Fake>, std::net::SocketAddr, Mods, PluginLike) {
    let plugin = PluginLike { server: Arc::new(std::sync::OnceLock::new()) };
    let (python, server, mods) = serve_mods("mods-s", Arc::new(plugin.clone())).await;
    plugin.server.set(server).unwrap();
    (python, server, mods, plugin)
}

#[tokio::test]
async fn url_of_an_app_click_goes_back_in_the_press_answer() {
    let (_python, server, _mods, _plugin) = setup().await;
    let response = post(format!("http://{server}/api/sessions/mods-s/plugin/press"),
        json!({"site": "vitrine-botoes", "key": "V45-url"}), true).await;
    assert_eq!(json_of(response).await, json!({"ok": true, "opened": "https://example.com/vitrine"}));
}

#[tokio::test]
async fn bridge_checks_token_and_url_and_passes_other_sessions() {
    let (python, server, mods, _plugin) = setup().await;
    let base = format!("http://{server}/api/plugin");
    let wrong = post(format!("{base}/press-start"), json!({"sessao": "mods-s", "token": "x", "requestId": "a", "element": "b"}), false).await;
    assert_eq!(wrong.status().as_u16(), 403);
    let attempt = mods.begin_click("mods-s", "a", "b");
    let ftp = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": mint(OWNER, "mods-s"), "attempt": attempt, "url": "ftp://x"}), false).await;
    assert_eq!(ftp.status().as_u16(), 400);
    let late = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": mint(OWNER, "mods-s"), "attempt": "outra", "url": "https://x"}), false).await;
    assert_eq!(late.status().as_u16(), 409, "fora do clique o mod abre no servidor");
    let other = post(format!("{base}/press-start"), json!({"sessao": "com-terminal", "token": "t", "requestId": "a", "element": "b"}), false).await;
    assert_eq!(other.text().await.unwrap(), "from-python");
    assert_eq!(python.hits_to("/api/plugin/press-start"), 1);
}

#[tokio::test]
async fn opened_checks_token_case_size_and_passes_invalid_bodies() {
    let (python, server, mods, _plugin) = setup().await;
    let base = format!("http://{server}/api/plugin");
    let token = mint(OWNER, "mods-s");
    let attempt = mods.begin_click("mods-s", "a", "b");
    let wrong = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": "x", "attempt": attempt, "url": "https://x"}), false).await;
    assert_eq!(wrong.status().as_u16(), 403);
    let empty = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": token, "attempt": attempt, "url": ""}), false).await;
    assert_eq!(empty.status().as_u16(), 400, "URL vazia não tem limite no Pydantic: é o esquema que recusa");
    // Mais de 8192 bytes, menos de 8192 caracteres: o teto conta caracteres, como o Pydantic do Python.
    let wide = format!("https://{}", "á".repeat(4100));
    let accepted = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": token, "attempt": attempt, "url": wide}), false).await;
    assert_eq!(accepted.status().as_u16(), 200);
    let upper = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": token, "attempt": attempt, "url": "HTTPS://Example.com/x"}), false).await;
    assert_eq!(upper.status().as_u16(), 200, "o esquema não distingue caixa");
    // Corpo que não é o de uma rota da ponte: o Rust não sabe de quem é, e o Python responde.
    let broken = post(format!("{base}/opened"), json!({"sessao": "com-terminal"}), false).await;
    assert_eq!(broken.text().await.unwrap(), "from-python");
    assert_eq!(python.hits_to("/api/plugin/opened"), 1);
}

#[tokio::test]
async fn bridge_applies_the_python_limits_before_the_token() {
    let (_python, server, _mods, _plugin) = setup().await;
    let base = format!("http://{server}/api/plugin");
    let cases = [
        ("press-start", json!({"sessao": "mods-s", "token": "x", "requestId": "", "element": "b"})),
        ("press-start", json!({"sessao": "mods-s", "token": "x", "requestId": "é".repeat(65), "element": "b"})),
        ("press-start", json!({"sessao": "mods-s", "token": "x", "requestId": "a", "element": ""})),
        ("press-start", json!({"sessao": "mods-s", "token": "x", "requestId": "a", "element": "é".repeat(257)})),
        ("opened", json!({"sessao": "mods-s", "token": "x", "attempt": "", "url": "https://x"})),
        ("opened", json!({"sessao": "mods-s", "token": "x", "attempt": "é".repeat(65), "url": "https://x"})),
        ("opened", json!({"sessao": "mods-s", "token": "x", "attempt": "a", "url": format!("https://{}", "a".repeat(8192 - 8 + 1))})),
    ];
    for (route, body) in cases {
        let response = post(format!("{base}/{route}"), body.clone(), false).await;
        assert_eq!(response.status().as_u16(), 422, "{route} {body}");
    }
    // No limite exato, contado em caracteres e não em bytes, passa da validação e cai no token.
    let edge = post(format!("{base}/press-start"), json!({"sessao": "mods-s", "token": "x",
        "requestId": "é".repeat(64), "element": "é".repeat(256)}), false).await;
    assert_eq!(edge.status().as_u16(), 403);
}

#[tokio::test]
async fn renamed_session_is_found_by_the_name_its_process_was_born_with() {
    // M3: renomear fecha e reabre a sessão no Rust com o mesmo `claude -p`, que segue mandando à ponte o
    // nome e o token de nascimento. A ponte acha a sessão por esse nome, e a URL volta ao aparelho.
    let (_python, server, mods, plugin) = setup().await;
    mods.forget("mods-s", 1);
    mods.attach_process("renomeada", "mods-s", 2, Arc::new(plugin));
    assert_eq!(mods.bridge_session("mods-s").as_deref(), Some("renomeada"));
    let response = post(format!("http://{server}/api/sessions/renomeada/plugin/press"),
        json!({"site": "vitrine-botoes", "key": "V45-url"}), true).await;
    assert_eq!(json_of(response).await, json!({"ok": true, "opened": "https://example.com/vitrine"}));
}

#[tokio::test]
async fn old_name_token_never_acts_on_a_new_session_with_that_name() {
    // O token da ponte é derivado só do nome. Renomeada a sessão sem relançar o processo, e criada outra
    // com o nome antigo, os dois processos vivos têm o mesmo token: a ponte não atende nenhum dos dois, e
    // o clique em aberto da sessão nova não é tomado nem recebe URL de fora.
    let (python, server, mods, plugin) = setup().await;
    mods.forget("mods-s", 1);
    mods.attach_process("renomeada", "mods-s", 2, Arc::new(plugin));
    mods.attach_process("mods-s", "processo-novo", 3, Arc::new(mods_support_free::Quiet));
    let attempt = mods.begin_click("mods-s", "a", "b");
    let base = format!("http://{server}/api/plugin");
    let token = mint(OWNER, "mods-s");
    let start = post(format!("{base}/press-start"), json!({"sessao": "mods-s", "token": token, "requestId": "a", "element": "b"}), false).await;
    assert_eq!(start.text().await.unwrap(), "from-python");
    let opened = post(format!("{base}/opened"), json!({"sessao": "mods-s", "token": token, "attempt": attempt, "url": "https://x"}), false).await;
    assert_eq!(opened.text().await.unwrap(), "from-python");
    assert_eq!((python.hits_to("/api/plugin/press-start"), python.hits_to("/api/plugin/opened")), (1, 1));
    assert!(mods.match_click("mods-s", "a", "b").is_some(), "o clique da sessão nova segue em aberto, sem dono de fora");
    // A renomeada sai do Rust: o nome volta a ter um processo vivo só, e a ponte o atende.
    mods.forget("renomeada", 2);
    assert_eq!(mods.bridge_session("mods-s").as_deref(), Some("mods-s"));
}

/// Superfície que não é chamada nos testes da ponte.
mod mods_support_free {
    pub struct Quiet;
    impl hangar_server::mods::state::SurfaceLink for Quiet {
        fn call(&self, _: hangar_server::mods::model::ModsCall, _: std::time::Instant) -> hangar_server::mods::state::CallFuture {
            Box::pin(async { Err(hangar_server::mods::model::missing()) })
        }
    }
}
