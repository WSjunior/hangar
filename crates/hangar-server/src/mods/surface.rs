//! Cliente da superfície remota de um `claude -p` (pedidos `ui_*` no stream-json): o Hangar entra
//! como `desktop`, pede a faixa e os painéis, segue os avisos e leva os pedidos dos apps ao mod.
//! Máquina de estados pura, como o `ClaudeEngine`: entra linha e relógio, sai efeito.
use std::collections::{BTreeMap, BTreeSet};

use serde_json::{Value, json};

use super::model::*;
use super::tree;
use crate::runtime::protocol::RequestId;

/// Prazo do desenho: a árvore de 398 KB veio em 0,35 s; a folga cobre máquina ocupada.
const RENDER_S: f64 = 10.0;
/// Prazo de clique, fechar, mostrar e digitar, e do desenho de novo que um clique ou uma digitação pede
/// (S4). Curto porque o app desiste em 8 s: a resposta tem que chegar antes, senão o app mostra erro
/// com o clique ainda rodando no mod. Também é o mínimo que precisa sobrar do prazo de quem pediu para
/// a ação sair: com menos, a rota responderia antes do mod e o clique rodaria depois do erro no app.
const CALL_S: f64 = 3.0;
/// O pedido de app mais longo: o desenho de novo antes do clique e o clique depois dele (6 s). O
/// fechar, com a confirmação pelo rol, fica em 5 s.
pub const APP_CALL_MAX_S: f64 = 2.0 * CALL_S;
const ATTACH_S: f64 = 15.0;
/// Esperas antes de cada nova ligação, ou novo pedido do rol, depois de um sem resposta (o pedido pode
/// ter se perdido com o cano travado); esgotadas, a ligação desliga a superfície até o processo religar,
/// e o rol fica à espera do próximo aviso `ui_panes`.
const ATTACH_RETRY_S: [f64; 3] = [1.0, 2.0, 4.0];
/// Pedidos de desenho juntados depois de um `ui_invalidate` (S3).
const BATCH_S: f64 = 0.1;
/// O fechar só vale com o aviso `ui_panes` sem o painel.
const CLOSE_CONFIRM_S: f64 = 2.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Phase { Idle, Attaching, Ready, Off }

enum Pending {
    Attach,
    Panes,
    Render { instance: String },
    /// Clique ou digitação levados ao mod pelo `handle` do desenho guardado.
    /// `until`: quando quem pediu deixa de esperar, no relógio da superfície.
    Act { token: u64, call: ModsCall, retried: bool, until: f64 },
    /// Desenho pedido para tentar de novo um clique ou uma digitação (S4).
    Refresh { token: u64, call: ModsCall, until: f64 },
    Close { token: u64, site: String },
    Show { token: u64, site: String },
}

impl Pending {
    fn token(&self) -> Option<u64> {
        match self {
            Pending::Act { token, .. } | Pending::Refresh { token, .. }
            | Pending::Close { token, .. } | Pending::Show { token, .. } => Some(*token),
            Pending::Attach | Pending::Panes | Pending::Render { .. } => None,
        }
    }
    fn limit(&self) -> f64 {
        match self {
            Pending::Attach => ATTACH_S,
            Pending::Panes | Pending::Render { .. } => RENDER_S,
            _ => CALL_S,
        }
    }
}

struct Waiting { pending: Pending, deadline: f64 }

pub struct Surface {
    prefix: String,
    counter: u64,
    phase: Phase,
    waiting: BTreeMap<String, Waiting>,
    panes: Vec<PaneItem>,
    shown: Option<String>,
    trees: BTreeMap<String, Value>,
    dirty: BTreeSet<String>,
    flush_at: Option<f64>,
    closing: Vec<(u64, String, f64)>,
    working: bool,
    /// A vista mudou desde a última publicação (árvore guardada, rol, ou tudo limpo). A comparação com a
    /// anterior fica no `Mods`: aqui não se guarda cópia da vista para comparar.
    changed: bool,
    /// Novas ligações já feitas depois de uma sem resposta, e quando sai a próxima.
    attach_retries: usize,
    attach_at: Option<f64>,
    /// O mesmo para o pedido do rol.
    panes_retries: usize,
    panes_at: Option<f64>,
}

fn reply(token: u64, result: Result<Value, ModsError>) -> SurfaceEffect {
    SurfaceEffect::Reply { token, result }
}

impl Surface {
    /// `prefix` separa os pedidos desta vida dos de um ator anterior ligado ao mesmo processo do Claude
    /// Code: a resposta atrasada de um pedido velho não pode ser tomada como de um pedido novo.
    pub fn new(prefix: String) -> Self {
        Self { prefix, counter: 0, phase: Phase::Idle, waiting: BTreeMap::new(), panes: Vec::new(), shown: None,
            trees: BTreeMap::new(), dirty: BTreeSet::new(), flush_at: None,
            closing: Vec::new(), working: false, changed: false, attach_retries: 0, attach_at: None,
            panes_retries: 0, panes_at: None }
    }

    pub fn owns(&self, id: &RequestId) -> bool {
        matches!(id, RequestId::String(id) if id.strip_prefix(self.prefix.as_str()).is_some_and(|rest| rest.starts_with(':')))
    }

    pub fn is_ready(&self) -> bool {
        self.phase == Phase::Ready
    }

    /// Processo novo, ou o mesmo depois de uma reconexão: liga de novo. `ui_attach` repetido é aceito
    /// e reenvia o rol (E3), então o estado anterior não precisa ser guardado.
    pub fn start(&mut self, now: f64) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        self.reset(&mut out);
        self.phase = Phase::Attaching;
        self.attach_retries = 0;
        self.attach(now, &mut out);
        out
    }

    fn attach(&mut self, now: f64, out: &mut Vec<SurfaceEffect>) {
        self.attach_at = None;
        self.request("ui_attach", json!({"surface": SURFACE, "client_id": CLIENT_ID, "viewport": viewport(), "answers": ["ui_copy"]}),
            Pending::Attach, now, out);
    }

    pub fn on_response(&mut self, id: &RequestId, response: &Value, now: f64) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        let RequestId::String(id) = id else { return out };
        let Some(waiting) = self.waiting.remove(id) else { return out };
        let ok = response["subtype"] == "success";
        let body = response.get("response").cloned().unwrap_or(Value::Null);
        match waiting.pending {
            Pending::Attach if ok => {
                self.phase = Phase::Ready;
                self.attach_retries = 0;
                self.render(BAND_SITE, now, &mut out);
                self.panes_retries = 0;
                self.ask_panes(now, &mut out);
            }
            // Recusada: o rol aceito durante a ligação e os desenhos em voo saem junto.
            Pending::Attach => self.turn_off(&mut out),
            Pending::Panes => if ok {
                self.panes_retries = 0;
                self.apply_panes(&body, now, &mut out);
            },
            Pending::Render { instance } => {
                if ok { self.store(&instance, &body, &mut out); }
                // O que ficou sujo enquanto este desenho estava em voo sai na próxima janela.
                self.arm(now);
            }
            Pending::Act { token, call, retried, until } => match (ok, body["handled"] == true) {
                (false, _) => out.push(reply(token, Err(no_answer()))),
                (true, true) => {
                    let mut answer = json!({"element": body["element"]});
                    if matches!(call, ModsCall::Input { .. }) { answer["value"] = body["value"].clone(); }
                    out.push(reply(token, Ok(answer)));
                }
                (true, false) if retried => out.push(reply(token, Err(stale()))),
                // Nada rodou no mod (P03): o desenho que o servidor tem está vencido.
                (true, false) => self.refresh(token, call, now, until, &mut out),
            },
            Pending::Refresh { token, call, until } => {
                if ok { self.store(call.site(), &body, &mut out); }
                self.act(token, call, true, now, until, &mut out);
                self.arm(now);
            }
            Pending::Close { token, site } => {
                if !ok { out.push(reply(token, Err(no_answer()))); }
                else if body["closed"] == false { out.push(reply(token, Err(close_refused()))); }
                else if !self.panes.iter().any(|pane| pane.id == site) { out.push(reply(token, Ok(json!({})))); }
                // `closed: true` vale também para id desconhecido (E1): a prova é o rol sem o painel.
                else { self.closing.push((token, site, now + CLOSE_CONFIRM_S)); }
            }
            Pending::Show { token, site } => {
                if !ok { out.push(reply(token, Err(no_answer()))); }
                // Id desconhecido devolve o `shown_id` de antes, sem erro (E1).
                else if body["shown_id"] == site.as_str() { out.push(reply(token, Ok(json!({"shown_id": site})))); }
                else { out.push(reply(token, Err(pane_missing()))); }
            }
        }
        out
    }

    /// Avisos sem pedido. São do processo, não do cliente, e nenhum traz `client_id`.
    pub fn on_notice(&mut self, event: &Value, now: f64) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        match event["subtype"].as_str() {
            Some("ui_panes") if matches!(self.phase, Phase::Attaching | Phase::Ready) => self.apply_panes(event, now, &mut out),
            Some("ui_invalidate") if self.phase == Phase::Ready => {
                match event["instances"].as_array() {
                    // A lista traz também instâncias já fechadas (P11): só as montadas são pedidas.
                    Some(instances) => for instance in instances {
                        if instance["surface"] != SURFACE { continue; }
                        if let Some(id) = instance["instance_id"].as_str().filter(|id| self.mounted(id)) {
                            self.dirty.insert(id.to_owned());
                        }
                    },
                    None => {
                        self.dirty.insert(BAND_SITE.to_owned());
                        self.dirty.extend(self.panes.iter().map(|pane| pane.id.clone()));
                    }
                }
                self.arm(now);
            }
            Some("ui_toast") => if let (Some(plugin), Some(text)) = (event["plugin"].as_str(), event["text"].as_str()) {
                out.push(SurfaceEffect::Toast { plugin: plugin.to_owned(), text: text.to_owned(),
                    timeout_ms: event["timeout_ms"].as_u64().unwrap_or(TOAST_DEFAULT_MS) });
            },
            // `ui_status` fica fora desta entrega (spec, "Fonte superfície", passo 5).
            _ => {}
        }
        out
    }

    /// Pedido `ui_copy` do Claude Code: sempre `copied: true` e na hora. Erro ou silêncio deixam o mod
    /// esperando 5 s (P07). O pedido chega antes da resposta do clique que o causou.
    pub fn on_copy(&mut self, request_id: &Value, request: &Value) -> Vec<SurfaceEffect> {
        let mut out = vec![SurfaceEffect::Write { until: None, frame: json!({"type": "control_response", "response": {
            "subtype": "success", "request_id": request_id, "response": {"copied": true}}}) }];
        // Antes de ligar só chega pedido velho, do snapshot do cano: responde e não entrega.
        if self.phase == Phase::Ready && let Some(text) = request["text"].as_str() {
            out.push(SurfaceEffect::Copied { plugin: request["plugin"].as_str().unwrap_or("").to_owned(), text: text.to_owned() });
        }
        out
    }

    /// O processo do Claude Code saiu: pedidos em aberto respondem com código e a faixa some.
    pub fn on_exit(&mut self) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        self.turn_off(&mut out);
        out
    }

    /// A faixa recebe `isWorking`: a troca pede um desenho novo dela.
    pub fn set_working(&mut self, working: bool, now: f64) {
        if self.working == working { return; }
        self.working = working;
        if self.phase == Phase::Ready { self.mark(BAND_SITE, now); }
    }

    pub fn tick(&mut self, now: f64) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        if self.attach_at.is_some_and(|at| now + 1e-9 >= at) && self.phase == Phase::Attaching {
            self.attach(now, &mut out);
        }
        if self.panes_at.is_some_and(|at| now + 1e-9 >= at) && self.phase == Phase::Ready {
            self.ask_panes(now, &mut out);
        }
        if self.flush_at.is_some_and(|at| now + 1e-9 >= at) {
            self.flush_at = None;
            for instance in std::mem::take(&mut self.dirty) {
                if self.mounted(&instance) { self.render(&instance, now, &mut out); }
            }
        }
        let expired: Vec<String> = self.waiting.iter().filter(|(_, waiting)| now + 1e-9 >= waiting.deadline)
            .map(|(id, _)| id.clone()).collect();
        for id in expired {
            let Some(waiting) = self.waiting.remove(&id) else { continue };
            match waiting.pending {
                Pending::Attach => match ATTACH_RETRY_S.get(self.attach_retries) {
                    Some(wait) => { self.attach_retries += 1; self.attach_at = Some(now + wait); }
                    None => self.turn_off(&mut out),
                },
                // Desenho sem resposta (pedido descartado com o canal do cano cheio, por exemplo) volta a
                // ficar sujo e sai de novo na próxima janela; sem isso a tela do app ficava parada.
                Pending::Render { instance } => { self.dirty.insert(instance); }
                Pending::Refresh { token, call, .. } => {
                    self.dirty.insert(call.site().to_owned());
                    out.push(reply(token, Err(no_answer())));
                }
                // Rol sem resposta: sem pedir de novo, um painel aberto nesse meio só aparecia no próximo
                // aviso `ui_panes`, quando algum painel mudasse.
                Pending::Panes => if let Some(wait) = ATTACH_RETRY_S.get(self.panes_retries) {
                    self.panes_retries += 1;
                    self.panes_at = Some(now + wait);
                },
                pending => if let Some(token) = pending.token() { out.push(reply(token, Err(no_answer()))); },
            }
        }
        // Desenho vencido deixa de estar em voo: o que ficou sujo atrás dele sai na próxima janela.
        self.arm(now);
        let (late, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.closing).into_iter().partition(|(_, _, until)| now + 1e-9 >= *until);
        self.closing = open;
        for (token, ..) in late { out.push(reply(token, Err(no_answer()))); }
        out
    }

    pub fn deadline(&self) -> Option<f64> {
        self.flush_at.into_iter().chain(self.attach_at).chain(self.panes_at).chain(self.waiting.values().map(|waiting| waiting.deadline))
            .chain(self.closing.iter().map(|(_, _, until)| *until)).min_by(f64::total_cmp)
    }

    /// Pedido de um app. Clique e digitação vão pelo `handle` do último desenho, com a `key` junto;
    /// sem o elemento no desenho guardado, pede o desenho de novo e procura uma vez (S4). `until` é o
    /// prazo de quem pediu (a rota), no relógio da superfície: com menos de `CALL_S` dele, nada sai ao
    /// mod e a resposta é a de mod sem resposta.
    pub fn call(&mut self, token: u64, call: ModsCall, now: f64, until: f64) -> Vec<SurfaceEffect> {
        let mut out = Vec::new();
        if self.phase != Phase::Ready || !self.mounted(call.site()) {
            // Show e Close falam de painel; só Press e Input falam de botão ou campo.
            let error = if matches!(call, ModsCall::Show { .. } | ModsCall::Close { .. }) { pane_missing() } else { missing() };
            out.push(reply(token, Err(error)));
            return out;
        }
        if !Self::in_time(now, until) {
            out.push(reply(token, Err(no_answer())));
            return out;
        }
        match call {
            ModsCall::Show { site } if site != BAND_SITE => self.request("ui_pane_show",
                json!({"id": site, "surface": SURFACE, "client_id": CLIENT_ID}), Pending::Show { token, site }, now, &mut out),
            ModsCall::Close { site } if site != BAND_SITE => self.request("ui_close",
                json!({"id": site, "client_id": CLIENT_ID}), Pending::Close { token, site }, now, &mut out),
            ModsCall::Show { .. } | ModsCall::Close { .. } => out.push(reply(token, Err(pane_missing()))),
            call @ (ModsCall::Press { .. } | ModsCall::Input { .. }) => self.act(token, call, false, now, until, &mut out),
        }
        out
    }

    fn ask_panes(&mut self, now: f64, out: &mut Vec<SurfaceEffect>) {
        self.panes_at = None;
        self.request("ui_panes", json!({"client_id": CLIENT_ID}), Pending::Panes, now, out);
    }

    fn request(&mut self, subtype: &str, mut body: Value, pending: Pending, now: f64, out: &mut Vec<SurfaceEffect>) {
        self.counter += 1;
        let id = format!("{}:{}", self.prefix, self.counter);
        body["subtype"] = json!(subtype);
        let deadline = now + pending.limit();
        // Só o que age no mod leva o prazo: depois dele a superfície já respondeu que falhou.
        let until = matches!(subtype, "ui_press" | "ui_input" | "ui_close").then_some(deadline);
        self.waiting.insert(id.clone(), Waiting { pending, deadline });
        out.push(SurfaceEffect::Write { until, frame: json!({"type": "control_request", "request_id": id, "request": body}) });
    }

    fn mounted(&self, instance: &str) -> bool {
        instance == BAND_SITE || self.panes.iter().any(|pane| pane.id == instance)
    }

    /// O pedido de desenho com as props completas que uma tela de verdade manda (S2): sem `scroll`, um
    /// mod que o lê some do desenho sem aviso.
    fn render_body(&self, instance: &str) -> Option<Value> {
        let (component, props) = if instance == BAND_SITE {
            ("AbovePrompt", json!({"hasSurvey": false, "isWorking": self.working, "bodyColumns": BAND_COLUMNS, "maxRows": 30,
                "scroll": {"offset": 0, "bodyRows": 30}, "view": {}}))
        } else {
            let pane = self.panes.iter().find(|pane| pane.id == instance)?;
            ("Pane", json!({"title": pane.title, "isFocused": false, "bodyColumns": pane.columns() - 2, "placement": "dock",
                "scroll": {"offset": 0, "bodyRows": 40}, "view": {}}))
        };
        Some(json!({"surface": SURFACE, "client_id": CLIENT_ID, "component": component, "instance_id": instance,
            "props": props, "viewport": viewport()}))
    }

    /// Com um desenho da instância em voo, só a marca como suja: a resposta arma a janela (`arm`), e o
    /// ator não acorda a cada 100 ms enquanto espera.
    fn render(&mut self, instance: &str, now: f64, out: &mut Vec<SurfaceEffect>) {
        if self.in_flight(instance) { self.dirty.insert(instance.to_owned()); return; }
        let Some(body) = self.render_body(instance) else { return };
        self.request("ui_render", body, Pending::Render { instance: instance.to_owned() }, now, out);
    }

    /// Há desenho da instância esperando resposta, pedido pela faixa, pelo painel ou pela nova tentativa
    /// de um clique. Sai dos pedidos em aberto, então não se perde quando um deles vence ou é descartado.
    fn in_flight(&self, instance: &str) -> bool {
        self.waiting.values().any(|waiting| match &waiting.pending {
            Pending::Render { instance: id } => id == instance,
            Pending::Refresh { call, .. } => call.site() == instance,
            _ => false,
        })
    }

    fn mark(&mut self, instance: &str, now: f64) {
        self.dirty.insert(instance.to_owned());
        self.arm(now);
    }

    /// Abre a janela de 100 ms quando há instância suja sem desenho em voo.
    fn arm(&mut self, now: f64) {
        if self.flush_at.is_none() && self.dirty.iter().any(|id| !self.in_flight(id)) {
            self.flush_at = Some(now + BATCH_S);
        }
    }

    fn store(&mut self, instance: &str, body: &Value, out: &mut Vec<SurfaceEffect>) {
        let Some(tree) = body.get("tree").filter(|tree| tree.is_object()) else { return };
        // Desenho de painel que fechou no meio não volta ao evento.
        if !self.mounted(instance) { return; }
        self.trees.insert(instance.to_owned(), tree.clone());
        self.changed = true;
        self.publish(out);
    }

    fn apply_panes(&mut self, body: &Value, now: f64, out: &mut Vec<SurfaceEffect>) {
        let Ok(panes) = serde_json::from_value::<Vec<PaneItem>>(body["panes"].clone()) else { return };
        // Painel novo, ou com título ou tamanho trocado: as props mudaram, o desenho também.
        let changed: Vec<String> = panes.iter().filter(|pane| self.panes.iter().find(|old| old.id == pane.id) != Some(*pane))
            .map(|pane| pane.id.clone()).collect();
        self.panes = panes;
        self.shown = body["shown_id"].as_str().map(str::to_owned);
        self.changed = true;
        let ids: BTreeSet<String> = self.panes.iter().map(|pane| pane.id.clone()).collect();
        let keep = |id: &String| id == BAND_SITE || ids.contains(id);
        self.trees.retain(|id, _| keep(id));
        self.dirty.retain(|id| keep(id));
        for id in changed { self.render(&id, now, out); }
        let (gone, open): (Vec<_>, Vec<_>) = std::mem::take(&mut self.closing).into_iter().partition(|(_, site, _)| !ids.contains(site));
        self.closing = open;
        for (token, ..) in gone { out.push(reply(token, Ok(json!({})))); }
        self.publish(out);
    }

    fn view(&self) -> Value {
        let above = self.trees.get(BAND_SITE).filter(|tree| !tree::is_engine_only(tree)).cloned().unwrap_or(Value::Null);
        // `columns` do painel é o `bodyColumns` que o mod recebeu, a largura do lugar nos apps (contrato).
        let panes: Vec<Value> = self.panes.iter().map(|pane| json!({"id": pane.id, "title": pane.title, "placement": "dock",
            "columns": pane.columns() - 2, "tree": self.trees.get(&pane.id).cloned().unwrap_or(Value::Null)})).collect();
        json!({"above": above, "panes": panes, "shown_id": self.shown, "columns": BAND_COLUMNS, "source": "surface"})
    }

    fn publish(&mut self, out: &mut Vec<SurfaceEffect>) {
        if std::mem::take(&mut self.changed) {
            out.push(SurfaceEffect::Publish { data: self.view() });
        }
    }

    fn reset(&mut self, out: &mut Vec<SurfaceEffect>) {
        for (_, waiting) in std::mem::take(&mut self.waiting) {
            if let Some(token) = waiting.pending.token() { out.push(reply(token, Err(no_answer()))); }
        }
        for (token, ..) in std::mem::take(&mut self.closing) { out.push(reply(token, Err(no_answer()))); }
        self.panes.clear();
        self.shown = None;
        self.trees.clear();
        self.changed = true;
        self.dirty.clear();
        self.flush_at = None;
        self.attach_at = None;
        self.panes_at = None;
    }

    /// Ligação recusada, sem resposta depois das novas tentativas, ou processo encerrado: limpa o rol e os desenhos (em voo
    /// também) e publica a interface vazia.
    fn turn_off(&mut self, out: &mut Vec<SurfaceEffect>) {
        self.reset(out);
        self.phase = Phase::Off;
        self.publish(out);
    }

    fn control(&self, site: &str, plugin: &str, key: &str, kind: &str) -> Option<tree::Control> {
        self.trees.get(site).and_then(|tree| tree::find(tree, Some(plugin), key, &[kind]))
    }

    /// A ação só sai com o prazo inteiro dela ainda dentro do de quem pediu.
    fn in_time(now: f64, until: f64) -> bool {
        until - now + 1e-9 >= CALL_S
    }

    fn refresh(&mut self, token: u64, call: ModsCall, now: f64, until: f64, out: &mut Vec<SurfaceEffect>) {
        match self.render_body(call.site()) {
            Some(body) => self.request("ui_render", body, Pending::Refresh { token, call, until }, now, out),
            None => out.push(reply(token, Err(missing()))),
        }
    }

    /// Clique ou digitação pelo `handle` do desenho guardado. Sem o elemento nele, pede o desenho de novo
    /// e procura uma vez (S4); na nova tentativa, a falta é desenho vencido. No `ui_input` vão `key`,
    /// `component` e `instance_id` juntos: o Claude Code acha o campo mesmo com o `handle` vencido, e a
    /// digitação não se perde num redesenho (P09).
    /// Na nova tentativa, o prazo de quem pediu é conferido de novo: o desenho pode ter gastado o que sobrava.
    fn act(&mut self, token: u64, call: ModsCall, retried: bool, now: f64, until: f64, out: &mut Vec<SurfaceEffect>) {
        if !Self::in_time(now, until) {
            out.push(reply(token, Err(no_answer())));
            return;
        }
        // O mesmo mod com a mesma `key` duas vezes no lugar: não há como saber qual, e nenhum é acionado.
        let kind = if matches!(call, ModsCall::Input { .. }) { "Input" } else { "Button" };
        if let ModsCall::Press { site, plugin, key } | ModsCall::Input { site, plugin, key, .. } = &call
            && self.trees.get(site.as_str()).is_some_and(|tree| tree::ambiguous(tree, Some(plugin), key, &[kind])) {
            out.push(reply(token, Err(missing())));
            return;
        }
        let found = match &call {
            ModsCall::Press { site, plugin, key } => self.control(site, plugin, key, "Button").map(|control| ("ui_press",
                json!({"plugin": control.plugin, "handle": control.handle, "key": key, "surface": SURFACE, "client_id": CLIENT_ID}))),
            ModsCall::Input { site, plugin, key, submit, value } => self.control(site, plugin, key, "Input").map(|control| ("ui_input",
                json!({"plugin": control.plugin, "handle": control.handle, "kind": if *submit { "submit" } else { "change" },
                    "value": value, "key": key, "component": if site == BAND_SITE { "AbovePrompt" } else { "Pane" },
                    "instance_id": site, "surface": SURFACE, "client_id": CLIENT_ID}))),
            ModsCall::Close { .. } | ModsCall::Show { .. } => None,
        };
        match found {
            Some((subtype, body)) => self.request(subtype, body, Pending::Act { token, call, retried, until }, now, out),
            None if retried => out.push(reply(token, Err(stale()))),
            None => self.refresh(token, call, now, until, out),
        }
    }
}
