mod fake;

use std::sync::Arc;

use fake::*;
use hangar_server::mods::bridge::mint;
use hangar_server::mods::model::*;
use hangar_server::mods::state::*;
use hangar_server::routes::AppState;
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
struct PluginLike { server: std::sync::OnceLock<std::net::SocketAddr> }
impl SurfaceLink for PluginLike {
    fn call(&self, _: ModsCall) -> CallFuture {
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
    let (python, upstream) = spawn_fake().await;
    let state = AppState::new(config(upstream, "127.0.0.1"));
    let mods = state.mods.clone();
    let server = spawn_state(state).await;
    let plugin = PluginLike { server: std::sync::OnceLock::new() };
    plugin.server.set(server).unwrap();
    mods.attach("mods-s", 1, Arc::new(plugin.clone()));
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
