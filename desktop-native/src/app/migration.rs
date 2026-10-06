//! Página temporária "Migração para Rust" (sai na parte 7): quem atende a porta, versões, consumo e quem
//! atende cada área, de `GET /api/migration/status` a cada 3 s enquanto ela está aberta. Os textos são os do web.
use super::*;
use super::device::{DeviceReply, size_text};
use super::settings::{Page, settings_box};

const EVERY: Duration = Duration::from_secs(3);

#[derive(Clone, Copy)]
enum Owner { Rust, Python, Mixed }

struct AreaRow { label: String, owner: Owner, counts: String, routes: Vec<(String, bool)> }

/// O que a página mostra, montado na chegada da resposta.
pub(super) struct MigrationView {
    rust_serving: bool,
    facts: Vec<(String, String)>,
    python_error: Option<String>,
    procs: Vec<(String, Option<(String, String)>)>,
    window: String,
    /// `None`: o Python atende sozinho, e com o Rust fora toda área é dele.
    areas: Option<Vec<AreaRow>>,
    private: Vec<(String, u64)>,
}

fn t(key: &str) -> String { tr_shared(key, &[]) }

/// Texto de um código do servidor; código novo, que esta versão não conhece, aparece cru.
fn coded(prefix: &str, code: &str) -> String {
    crate::i18n::tr_web(&format!("{prefix}{code}"), &HashMap::new()).unwrap_or_else(|| code.to_owned())
}

pub(super) fn parse(v: &Value) -> Option<MigrationView> {
    let served = v.get("served_by")?.as_str()?;
    let s = |p: &str| v.pointer(p).and_then(Value::as_str).map(str::to_owned);
    let n = |p: &str| v.pointer(p).and_then(Value::as_u64).map(|n| n.to_string()).unwrap_or_else(|| "—".into());
    let rust_serving = served == "rust";
    let mut facts = vec![(t("migration_served"), t(if rust_serving { "migration_served_rust" } else { "migration_served_python" }))];
    if let Some(mode) = s("/python/mode") { facts.push((t("migration_mode"), coded("migration_mode_", &mode))); }
    if let Some(reason) = s("/python/reason") { facts.push((t("migration_reason"), format!("{} ({reason})", coded("migration_reason_", &reason)))); }
    facts.push((t("migration_contract"), tr_shared("migration_contract_value", &[("rust", &n("/rust/protocol")), ("python", &n("/python/protocol"))])));
    if let Some(version) = s("/rust/version") {
        let commit = s("/rust/commit").map(|c| c.chars().take(10).collect()).unwrap_or_else(|| t("migration_binary_local"));
        facts.push((t("migration_binary"), format!("{version} · {commit}")));
    }
    if let Some(path) = s("/python/binary/path") { facts.push((String::new(), path)); }
    if let Some(version) = s("/python/version") {
        facts.push((t("migration_checkout"), format!("{} · {version}", s("/python/branch").unwrap_or_else(|| "—".into()))));
        facts.push((t("migration_channel"), s("/python/update_branch").unwrap_or_else(|| t("migration_channel_none"))));
    }
    let usage = |p: &Value| -> Option<(String, String)> {
        if p.is_null() || p.get("count").and_then(Value::as_u64) == Some(0) { return None; }
        let cpu = p.get("cpu_percent").and_then(Value::as_f64)
            .map_or_else(|| t("migration_cpu_wait"), |c| format!("{c:.1}%").replace('.', &tr("decimal")));
        Some((size_text(p.get("rss_bytes").and_then(Value::as_u64).unwrap_or(0)), cpu))
    };
    let procs = v.pointer("/python/processes").map(|p| vec![
        (t("migration_proc_python"), usage(&p["python"])),
        (t("migration_proc_rust"), usage(&p["rust"])),
        (tr_shared("migration_proc_cano", &[("n", &p["cano"]["count"].as_u64().unwrap_or(0).to_string())]), usage(&p["cano"])),
    ]).unwrap_or_default();
    let areas = v.get("areas").and_then(Value::as_array).map(|areas| areas.iter().map(|a| {
        let routes: Vec<(String, bool)> = a["routes"].as_array().map(|r| r.iter().map(|r| (
            format!("{} {}", r["method"].as_str().unwrap_or("?"), r["path"].as_str().unwrap_or("?")), r["rust"].as_bool().unwrap_or(false)
        )).collect()).unwrap_or_default();
        let rust = routes.iter().filter(|(_, r)| *r).count();
        let owner = if rust == 0 { Owner::Python } else if rust == routes.len() { Owner::Rust } else { Owner::Mixed };
        AreaRow { label: coded("migration_area_", a["key"].as_str().unwrap_or("?")), owner,
            counts: format!("{} × {}", a["rust"].as_u64().unwrap_or(0), a["python"].as_u64().unwrap_or(0)), routes }
    }).collect());
    let private = v.get("private").and_then(Value::as_array).map(|p| p.iter()
        .map(|p| (coded("migration_private_", p["key"].as_str().unwrap_or("?")), p["rust"].as_u64().unwrap_or(0))).collect()).unwrap_or_default();
    Some(MigrationView {
        rust_serving, facts, procs, areas, private,
        python_error: s("/python_error").map(|code| tr_shared("migration_python_error", &[("codigo", &code)])),
        window: tr_shared("migration_window", &[("minutos", &n("/window_minutes"))]),
    })
}

impl Hangar {
    /// Página aberta: lê agora e de novo a cada 3 s até ela sair da tela. Reabrir troca a tarefa (a velha é cancelada).
    pub(super) fn migration_opened(&mut self, cx: &mut Context<Self>) {
        self.device.migration_poll = Some(cx.spawn(async move |this, cx| loop {
            let open = this.update(cx, |this, cx| {
                if this.settings != Some(Page::Migration) { return false; }
                this.load_migration(cx);
                true
            });
            if !matches!(open, Ok(true)) { break; }
            cx.background_executor().timer(EVERY).await;
        }));
    }

    fn load_migration(&mut self, cx: &mut Context<Self>) {
        let Some(api) = self.api.clone() else { return };
        if self.device.migration.loading { return; }
        let seq = self.device.migration.start();
        let done = self.device_send_later();
        self.runtime.spawn(async move { done(DeviceReply::Migration(seq, api.server_read(&["migration", "status"], &[], 10).await)).await });
        cx.notify();
    }

    pub(super) fn render_migration(&mut self, _cx: &mut Context<Self>) -> AnyElement {
        let heading = |key: &str| div().mt(px(28.)).mb(px(10.)).text_size(px(13.)).font_weight(FontWeight::SEMIBOLD).child(t(key));
        let note = |text: String, color: Hsla| div().px_4().py(px(18.)).text_size(px(13.)).text_color(color).whitespace_normal().child(text);
        let line = || div().mt(px(-1.)).border_t_1().border_color(theme::border()).px_4().py(px(10.)).flex().items_start().gap(px(14.)).text_size(px(13.));
        let top = div().flex().flex_col()
            .child(div().text_xl().font_weight(FontWeight::SEMIBOLD).child(t("migration_title")))
            .child(div().mt(px(6.)).text_color(theme::muted()).child(t("migration_temporary")));
        let state = &self.device.migration;
        let view = match (&self.api, &state.value, state.loading) {
            (None, ..) => return div().flex().flex_col().child(top).child(settings_box().mt(px(28.)).child(note(tr("settings_offline"), theme::muted()))).into_any_element(),
            (_, Some(Ok(Some(view))), _) => view,
            (_, Some(Ok(None)), _) => return div().flex().flex_col().child(top).child(settings_box().mt(px(28.)).child(note(t("migration_empty"), theme::muted()))).into_any_element(),
            (_, Some(Err(error)), _) => return div().flex().flex_col().child(top).child(settings_box().mt(px(28.))
                .child(note(tr_shared("migration_error", &[("erro", error)]), theme::danger()))).into_any_element(),
            (_, None, _) => return div().flex().flex_col().child(top).child(settings_box().mt(px(28.)).child(note(t("migration_loading"), theme::muted()))).into_any_element(),
        };
        let owner_text = |o: Owner| match o {
            Owner::Rust => (t("migration_owner_rust"), theme::success()),
            Owner::Python => (t("migration_owner_python"), theme::muted()),
            Owner::Mixed => (t("migration_owner_mixed"), theme::warning()),
        };
        let facts = settings_box().children(view.facts.iter().enumerate().map(|(i, (label, value))| line()
            .child(div().w(px(170.)).flex_shrink_0().text_color(theme::muted()).child(label.clone()))
            .child(div().flex_1().min_w_0().whitespace_normal()
                .when(i == 0, |el| el.font_weight(FontWeight::MEDIUM).text_color(if view.rust_serving { theme::success() } else { theme::warning() }))
                .when(label.is_empty(), |el| el.font_family(theme::MONO).text_size(px(12.)).text_color(theme::muted()))
                .child(value.clone()))))
            .children(view.python_error.clone().map(|text| note(text, theme::danger())));
        let usage = settings_box()
            .child(line().text_color(theme::muted()).text_size(px(12.)).child(div().flex_1()).child(div().w(px(110.)).child(t("migration_memory"))).child(div().w(px(90.)).child(t("migration_cpu"))))
            .children(view.procs.iter().map(|(label, value)| {
                let row = line().child(div().flex_1().min_w_0().child(label.clone()));
                match value {
                    Some((mem, cpu)) => row.child(div().w(px(110.)).font_family(theme::MONO).text_size(px(12.)).child(mem.clone()))
                        .child(div().w(px(90.)).font_family(theme::MONO).text_size(px(12.)).child(cpu.clone())),
                    None => row.child(div().w(px(200.)).text_color(theme::muted()).child(t("migration_proc_none"))),
                }
            }));
        let areas = match &view.areas {
            None => settings_box().child(note(t("migration_all_python"), theme::muted())),
            Some(areas) => settings_box()
                .child(note(view.window.clone(), theme::muted()).py(px(10.)))
                .children(areas.iter().map(|a| {
                    let (owner, color) = owner_text(a.owner);
                    line()
                        .child(div().flex_1().min_w_0().flex().flex_col().gap(px(2.)).child(a.label.clone())
                            .children(a.routes.iter().map(|(route, rust)| div().font_family(theme::MONO).text_size(px(11.))
                                .text_color(if *rust { theme::success() } else { theme::faint() }).child(route.clone()))))
                        .child(div().w(px(70.)).text_color(color).child(owner))
                        .child(div().w(px(80.)).font_family(theme::MONO).text_size(px(12.)).child(a.counts.clone()))
                })),
        };
        let private = (!view.private.is_empty() && view.areas.is_some()).then(|| div().flex().flex_col()
            .child(heading("migration_private"))
            .child(settings_box().children(view.private.iter().map(|(label, n)| line()
                .child(div().flex_1().child(label.clone()))
                .child(div().w(px(80.)).font_family(theme::MONO).text_size(px(12.)).child(n.to_string()))))));
        div().flex().flex_col().child(top)
            .child(heading("migration_port")).child(facts)
            .child(heading("migration_usage")).child(usage)
            .child(heading("migration_areas")).child(areas)
            .children(private)
            .into_any_element()
    }
}
