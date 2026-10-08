//! Escolhas só desta criação: o catálogo pertence à máquina, conta e pasta; o rascunho só vale depois de Aplicar.
use super::*;
use crate::api::dto::{ClaudeCustomizationCatalog, ClaudeCustomizationPlugin, ClaudeCustomizationSkill, ClaudeCustomizations};

#[derive(Clone, Debug, PartialEq, Eq)]
pub(in crate::app) struct CustomizationContext {
    machine: String,
    config: Option<String>,
    cwd: String,
}

#[derive(Clone, Copy)]
enum CatalogRow { Plugin(usize), PluginSkill(usize, usize), Skill(usize) }

fn is_customization_eligible(compact: bool, transfer: bool, provider: &str, fresh: bool, baton: bool) -> bool {
    !compact && !transfer && provider == "claude" && fresh && !baton
}

#[derive(Default)]
pub(super) struct CustomizationSelection {
    context: Option<CustomizationContext>,
    catalog: Remote<ClaudeCustomizationCatalog>,
    applied: ClaudeCustomizations,
    draft: Option<ClaudeCustomizations>,
    expanded: HashSet<String>,
    tab: usize,
    rows: Vec<CatalogRow>,
    scroll: UniformListScrollHandle,
}

impl CustomizationSelection {
    fn set_context(&mut self, context: Option<CustomizationContext>) -> bool {
        if self.context == context { return false; }
        self.context = context;
        self.catalog.reset();
        self.applied = Default::default();
        self.draft = None;
        self.expanded.clear();
        self.rows.clear();
        self.tab = 0;
        self.scroll = UniformListScrollHandle::new();
        true
    }

    pub(super) fn is_open(&self) -> bool { self.draft.is_some() }

    fn open(&mut self) { self.draft = Some(self.applied.clone()); }
    fn cancel(&mut self) { self.draft = None; }
    fn restore(&mut self) { if let Some(draft) = &mut self.draft { *draft = Default::default(); } }

    fn apply(&mut self) {
        if self.catalog.loading || self.catalog.ok().is_none() { return; }
        if let Some(draft) = self.draft.take() { self.applied = draft; }
    }

    fn payload(&self, context: Option<&CustomizationContext>) -> Option<&ClaudeCustomizations> {
        if context.is_none() || self.context.as_ref() != context || self.catalog.ok().is_none() { return None; }
        (!self.applied.plugins.is_empty() || !self.applied.skills.is_empty() || !self.applied.blocked_skills.is_empty()).then_some(&self.applied)
    }

    fn plugin_enabled(&self, plugin: &ClaudeCustomizationPlugin) -> bool {
        self.draft.as_ref().unwrap_or(&self.applied).plugins.get(&plugin.id).copied().unwrap_or(plugin.enabled)
    }

    fn skill_enabled(&self, skill: &ClaudeCustomizationSkill) -> bool {
        !skill.blocked && self.draft.as_ref().unwrap_or(&self.applied).skills.get(&skill.name).copied().unwrap_or(skill.enabled)
    }

    fn plugin_skill_enabled(&self, skill: &ClaudeCustomizationSkill) -> bool {
        skill.enabled && !skill.blocked && !self.draft.as_ref().unwrap_or(&self.applied).blocked_skills.contains(&skill.name)
    }

    fn set_plugin(&mut self, id: &str, enabled: bool) {
        let Some(base) = self.catalog.ok().and_then(|c| c.plugins.iter().find(|p| p.id == id)).map(|p| p.enabled) else { return };
        let Some(draft) = &mut self.draft else { return };
        if enabled == base { draft.plugins.remove(id); } else { draft.plugins.insert(id.to_owned(), enabled); }
    }

    fn set_skill(&mut self, name: &str, enabled: bool) {
        let Some(base) = self.catalog.ok().and_then(|c| c.skills.iter().find(|s| s.name == name && !s.blocked)).map(|s| s.enabled) else { return };
        let Some(draft) = &mut self.draft else { return };
        if enabled == base { draft.skills.remove(name); } else { draft.skills.insert(name.to_owned(), enabled); }
    }

    fn set_plugin_skill(&mut self, id: &str, name: &str, enabled: bool) {
        let Some(plugin) = self.catalog.ok().and_then(|c| c.plugins.iter().find(|p| p.id == id)) else { return };
        if !self.plugin_enabled(plugin) || !plugin.skills.iter().any(|s| s.name == name && s.enabled && !s.blocked) { return; }
        let Some(draft) = &mut self.draft else { return };
        draft.blocked_skills.retain(|s| s != name);
        if !enabled { draft.blocked_skills.push(name.to_owned()); draft.blocked_skills.sort(); }
    }

    fn summary(&self, context: Option<&CustomizationContext>) -> String {
        let Some(delta) = self.payload(context) else { return tr("create_customizations_default") };
        if delta.plugins.len() == 1 && delta.skills.is_empty() && delta.blocked_skills.is_empty()
            && let Some((id, false)) = delta.plugins.iter().next() {
            let name = self.catalog.ok().and_then(|c| c.plugins.iter().find(|p| &p.id == id)).map(|p| p.name.as_str()).unwrap_or(id);
            return tr("create_customizations_plugin_disabled").replace("{name}", name);
        }
        let count = delta.plugins.len() + delta.skills.len() + delta.blocked_skills.len();
        tr(if count == 1 { "create_customizations_changes_one" } else { "create_customizations_changes" }).replace("{n}", &count.to_string())
    }

    fn is_plugin_expanded(&self, id: &str, query: &str) -> bool { self.expanded.contains(id) || !query.trim().is_empty() }

    fn refilter(&mut self, query: &str) {
        self.rows.clear();
        let Some(catalog) = self.catalog.ok() else { return };
        let query = query.trim().to_lowercase();
        let matches = |name: &str, description: &str| query.is_empty() || name.to_lowercase().contains(&query) || description.to_lowercase().contains(&query);
        let mut rows = Vec::new();
        if self.tab == 0 {
            for (pi, plugin) in catalog.plugins.iter().enumerate() {
                let parent_matches = matches(&plugin.name, &plugin.description) || (!query.is_empty() && plugin.id.to_lowercase().contains(&query));
                let child_matches = |skill: &ClaudeCustomizationSkill| matches(&skill.name, &skill.description);
                if !parent_matches && !plugin.skills.iter().any(child_matches) { continue; }
                rows.push(CatalogRow::Plugin(pi));
                if self.is_plugin_expanded(&plugin.id, &query) {
                    rows.extend(plugin.skills.iter().enumerate().filter(|(_, s)| parent_matches || child_matches(s))
                        .map(|(si, _)| CatalogRow::PluginSkill(pi, si)));
                }
            }
        } else {
            rows.extend(catalog.skills.iter().enumerate().filter(|(_, s)| matches(&s.name, &s.description)).map(|(si, _)| CatalogRow::Skill(si)));
        }
        self.rows = rows;
    }
}

impl NewSession {
    fn customization_context(&self) -> Option<CustomizationContext> {
        is_customization_eligible(self.compact, self.is_transfer(), self.provider, self.target().is_none(), self.baton.is_some()).then(|| {
            Some(CustomizationContext { machine: self.link.api.identity(), config: self.config.clone(), cwd: self.picked.clone()? })
        }).flatten()
    }

    pub(super) fn sync_customizations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let was_open = self.customizations.is_open();
        if self.customizations.set_context(self.customization_context()) && was_open {
            self.return_from_customizations(window, cx);
        }
    }

    fn has_customization_account(&self) -> bool {
        self.config.as_ref().is_some_and(|path| self.configs.ok().is_some_and(|list| list.iter().any(|account| &account.path == path)))
    }

    fn open_customizations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.creating || self.account_busy || self.configs.loading || self.sessions.loading || !self.has_customization_account() { return; }
        self.sync_customizations(window, cx);
        if self.customization_context().is_none() { return; }
        self.customizations.open();
        self.create_when_ready = false;
        self.customizations_search.update(cx, |input, cx| { input.set_value("", window, cx); input.focus(window, cx); });
        self.refilter_customizations(cx);
        if !self.customizations.catalog.loading && self.customizations.catalog.ok().is_none() { self.load_customizations(cx); }
        cx.notify();
    }

    fn load_customizations(&mut self, cx: &mut Context<Self>) {
        let Some(context) = self.customization_context().filter(|c| Some(c) == self.customizations.context.as_ref()) else { return };
        if !self.customizations.is_open() || self.customizations.catalog.loading || !self.has_customization_account() { return; }
        let seq = self.customizations.catalog.start();
        self.request(cx, move |api, send| Box::pin(async move {
            let mut query = vec![("cwd", context.cwd.as_str())];
            if let Some(config) = context.config.as_deref() { query.push(("config_dir", config)); }
            let result = api.server_read(&["claude", "customizations"], &query, 30).await;
            send(CreateReply::Customizations(seq, context, result)).await
        }));
        cx.notify();
    }

    pub(super) fn receive_customizations(&mut self, seq: u64, context: CustomizationContext, result: Result<Value, Failure>, cx: &mut Context<Self>) {
        if self.customization_context().as_ref() != Some(&context) || self.customizations.context.as_ref() != Some(&context) { return; }
        let catalog = result.map_err(|e| Hangar::fetch_failure(&e))
            .and_then(|v| serde_json::from_value(v).map_err(|_| tr("invalid_response")));
        if self.customizations.catalog.finish(seq, catalog) { self.refilter_customizations(cx); }
    }

    pub(super) fn refilter_customizations(&mut self, cx: &App) {
        self.customizations.refilter(&self.customizations_search.read(cx).value());
    }

    fn close_customizations(&mut self, apply: bool, window: &mut Window, cx: &mut Context<Self>) {
        if !self.customizations.is_open() { return; }
        if apply {
            if self.customizations.catalog.loading || self.customizations.catalog.ok().is_none() || !self.has_customization_account() { return; }
            if self.customization_context() != self.customizations.context { self.sync_customizations(window, cx); return; }
            self.customizations.apply();
        } else { self.customizations.cancel(); }
        self.return_from_customizations(window, cx);
    }

    fn return_from_customizations(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let focus = if self.customization_context().is_some() { self.customizations_focus.clone() } else { self.query.read(cx).focus_handle(cx) };
        window.on_next_frame(move |window, cx| focus.focus(window, cx));
        cx.notify();
    }

    pub(super) fn cancel_customizations(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if !self.customizations.is_open() { return false; }
        self.close_customizations(false, window, cx);
        true
    }

    pub(super) fn customization_payload(&self) -> Option<&ClaudeCustomizations> {
        if !self.has_customization_account() { return None; }
        self.customizations.payload(self.customization_context().as_ref())
    }

    pub(super) fn render_customizations_row(&self, cx: &mut Context<Self>) -> Option<Div> {
        let context = self.customization_context()?;
        Some(div().flex().flex_col().gap_2()
            .child(label(tr("create_customizations_title")))
            .child(div().flex().items_center().gap_3()
                .child(div().flex_1().min_w_0().text_sm().text_color(theme::muted()).whitespace_normal()
                    .child(self.customizations.summary(Some(&context))))
                .child(chrome::OwnFocus { id: "create-customizations-choose".into(), focus: Some(self.customizations_focus.clone()),
                    button: Button::new("create-customizations-choose").outline().small().label(tr("create_customizations_choose"))
                        .disabled(self.creating || self.account_busy || self.configs.loading || self.sessions.loading || !self.has_customization_account())
                        .on_click(cx.listener(|this, _, window, cx| this.open_customizations(window, cx))) })))
    }

    fn customization_row(&self, ix: usize, cx: &mut Context<Self>) -> Option<AnyElement> {
        let catalog = self.customizations.catalog.ok()?;
        let row = *self.customizations.rows.get(ix)?;
        let (id, name, description, checked, disabled, reason, nested) = match row {
            CatalogRow::Plugin(pi) => {
                let p = catalog.plugins.get(pi)?;
                (format!("create-customization-plugin-{}", p.id), p.name.clone(), p.description.clone(),
                    self.customizations.plugin_enabled(p), false, None, false)
            }
            CatalogRow::Skill(si) => {
                let s = catalog.skills.get(si)?;
                (format!("create-customization-skill-{}", s.name), s.name.clone(), s.description.clone(),
                    self.customizations.skill_enabled(s), s.blocked, s.blocked.then(|| tr("create_customizations_inherited")), false)
            }
            CatalogRow::PluginSkill(pi, si) => {
                let p = catalog.plugins.get(pi)?;
                let s = p.skills.get(si)?;
                let package_enabled = self.customizations.plugin_enabled(p);
                let reason = if s.blocked || !s.enabled { Some(tr("create_customizations_inherited")) }
                    else if !package_enabled { Some(tr("create_customizations_package_disabled")) } else { None };
                (format!("create-customization-plugin-skill-{}-{}", p.id, s.name), s.name.clone(), s.description.clone(),
                    package_enabled && self.customizations.plugin_skill_enabled(s), s.blocked || !s.enabled || !package_enabled, reason, true)
            }
        };
        let label = if description.is_empty() { name.clone() } else { format!("{name}. {description}") };
        let caption = reason.unwrap_or(description);
        let context = self.customizations.context.clone();
        let checkbox = Checkbox::new(SharedString::from(id.clone())).small().accessibility_label(label)
            .checked(checked).disabled(disabled).on_click(cx.listener(move |this, on: &bool, _, cx| {
                if this.customization_context() != context || !this.customizations.is_open() { return; }
                let Some(catalog) = this.customizations.catalog.ok() else { return };
                match row {
                    CatalogRow::Plugin(pi) => if let Some(p) = catalog.plugins.get(pi) {
                        let id = p.id.clone(); this.customizations.set_plugin(&id, *on);
                    },
                    CatalogRow::Skill(si) => if let Some(s) = catalog.skills.get(si) {
                        let name = s.name.clone(); this.customizations.set_skill(&name, *on);
                    },
                    CatalogRow::PluginSkill(pi, si) => if let Some((p, s)) = catalog.plugins.get(pi).and_then(|p| p.skills.get(si).map(|s| (p, s))) {
                        let (id, name) = (p.id.clone(), s.name.clone()); this.customizations.set_plugin_skill(&id, &name, *on);
                    },
                }
                cx.notify();
            }));
        let disclosure = if let CatalogRow::Plugin(pi) = row {
            let plugin = catalog.plugins.get(pi)?;
            (!plugin.skills.is_empty()).then(|| {
                let id = plugin.id.clone();
                let open = self.customizations.is_plugin_expanded(&id, &self.customizations_search.read(cx).value());
                let weak = cx.entity().downgrade();
                Disclosure::new(format!("create-customization-expand-{id}"), open, tr("create_customizations_plugin_skills"), true)
                    .name(tr("create_customizations_expand").replace("{name}", &plugin.name))
                    .on_change(move |open, cx| { let _ = weak.update(cx, |this, cx| {
                        if open { this.customizations.expanded.insert(id.clone()); } else { this.customizations.expanded.remove(&id); }
                        this.refilter_customizations(cx); cx.notify();
                    }); })
            })
        } else { None };
        Some(div().id(SharedString::from(format!("{id}-row"))).h(rems(4.5)).px_2().py_2().flex().items_center().gap_3()
            .border_b_1().border_color(theme::border()).when(nested, |el| el.pl_6())
            .child(checkbox)
            .child(div().flex_1().min_w_0().flex().flex_col().gap_1()
                .child(div().truncate().text_sm().text_color(if disabled { theme::muted() } else { theme::text() }).child(name))
                .child(div().id(SharedString::from(format!("{id}-description"))).truncate().text_xs().text_color(theme::muted())
                    .tooltip({ let caption = caption.clone(); move |window, cx| gpui_kit::component::tooltip::Tooltip::new(caption.clone()).build(window, cx) }).child(caption)))
            .children(disclosure).into_any_element())
    }

    pub(super) fn render_customizations(&self, cx: &mut Context<Self>) -> Div {
        let account = self.configs.ok().and_then(|list| list.iter().find(|c| Some(&c.path) == self.config.as_ref()))
            .map(|c| c.label.clone()).unwrap_or_else(|| self.config.clone().unwrap_or_else(|| tr("create_default")));
        let scope = tr("create_customizations_scope").replace("{machine}", &self.server_label()).replace("{account}", &account);
        let loading = self.customizations.catalog.loading || self.customizations.catalog.value.is_none();
        let body = match &self.customizations.catalog.value {
            _ if loading => div().id("create-customizations-loading").role(Role::Status).p_4().child(muted(tr("loading"))).into_any_element(),
            Some(Err(error)) => div().p_4().flex().flex_col().items_start().gap_3()
                .child(alert("create-customizations-error", tr("create_customizations_failed").replace("{reason}", error)))
                .child(muted(tr("create_customizations_failure_help")))
                .child(Button::new("create-customizations-retry").outline().small().label(tr("create_try_again"))
                    .on_click(cx.listener(|this, _, _, cx| this.load_customizations(cx)))).into_any_element(),
            Some(Ok(_)) if self.customizations.rows.is_empty() => div().p_4().child(muted(tr(if self.customizations_search.read(cx).value().trim().is_empty() {
                "create_customizations_empty" } else { "create_customizations_no_results" }))).into_any_element(),
            Some(Ok(_)) => uniform_list("create-customizations-list", self.customizations.rows.len(),
                cx.processor(|this, range: std::ops::Range<usize>, _, cx| range.filter_map(|ix| this.customization_row(ix, cx)).collect::<Vec<_>>()))
                .track_scroll(&self.customizations.scroll).size_full().into_any_element(),
            None => div().into_any_element(),
        };
        let tabs = TabBar::new("create-customizations-tabs").underline().small().selected_index(self.customizations.tab)
            .children([Tab::new().label(tr("create_customizations_plugins")), Tab::new().label(tr("create_customizations_skills"))])
            .on_click(cx.listener(|this, ix: &usize, _, cx| {
                this.customizations.tab = *ix;
                this.customizations.scroll = UniformListScrollHandle::new();
                this.refilter_customizations(cx); cx.notify();
            }));
        let ready = !loading && self.customizations.catalog.ok().is_some() && self.has_customization_account();
        div().flex_1().min_w_0().h_full().min_h_0().pl_5().flex().flex_col().gap_3()
            .child(div().flex_shrink_0().flex().flex_col().items_start().gap_2()
                .child(Button::new("create-customizations-back").ghost().small().icon(IconName::ChevronLeft).label(tr("create_customizations_back"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_customizations(false, window, cx))))
                .child(div().text_lg().font_weight(FontWeight::SEMIBOLD).child(tr("create_customizations_title")))
                .child(div().text_sm().text_color(theme::muted()).whitespace_normal().child(scope))
                .child(muted(tr("create_customizations_session_only"))))
            .child(Input::new(&self.customizations_search).small().cleanable(true).aria_label(tr("create_customizations_search")))
            .child(tabs)
            .child(div().relative().flex_1().min_h_0().child(body))
            .when_some(self.customizations.catalog.ok().filter(|c| !loading && !c.warnings.is_empty()),
                |el, catalog| el.child(div().id("create-customizations-warnings").role(Role::Status).max_h(rems(6.)).overflow_y_scroll()
                    .flex().flex_col().gap_1().children(catalog.warnings.iter().map(|w| muted(w.clone())))))
            .child(muted(tr("create_customizations_hooks_help")))
            .child(muted(tr("create_customizations_policy_help")))
            .child(div().flex_shrink_0().pt_3().border_t_1().border_color(theme::border()).flex().items_center().gap_2()
                .child(Button::new("create-customizations-restore").ghost().small().label(tr("create_customizations_restore")).disabled(!ready)
                    .on_click(cx.listener(|this, _, _, cx| { this.customizations.restore(); cx.notify(); })))
                .child(div().flex_1())
                .child(Button::new("create-customizations-cancel").outline().small().label(tr("cancel"))
                    .on_click(cx.listener(|this, _, window, cx| this.close_customizations(false, window, cx))))
                .child(Button::new("create-customizations-apply").primary().small().label(tr("create_customizations_apply")).disabled(!ready)
                    .on_click(cx.listener(|this, _, window, cx| this.close_customizations(true, window, cx)))))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn context(machine: &str, config: Option<&str>, cwd: &str) -> CustomizationContext {
        CustomizationContext { machine: machine.into(), config: config.map(str::to_owned), cwd: cwd.into() }
    }

    fn selection() -> CustomizationSelection {
        let mut state = CustomizationSelection::default();
        state.set_context(Some(context("machine", Some("account"), "/project")));
        state.catalog.set(Ok(serde_json::from_value(json!({"plugins": [
            {"id": "tools@market", "name": "tools", "enabled": true, "skills": [
                {"name": "tools:allowed", "enabled": true}, {"name": "tools:restricted", "enabled": false, "blocked": true}]},
            {"id": "extra@market", "name": "extra", "enabled": false, "skills": []}],
            "skills": [{"name": "standalone", "enabled": true}, {"name": "inactive", "enabled": false},
                {"name": "restricted", "enabled": false, "blocked": true}], "warnings": []})).unwrap()));
        state.open();
        state
    }

    #[test]
    fn eligibility_only_allows_fresh_claude_in_the_new_session_modal() {
        assert!(is_customization_eligible(false, false, "claude", true, false));
        assert!(!is_customization_eligible(false, false, "claude", true, true));
        assert!(!is_customization_eligible(true, false, "claude", true, false));
        assert!(!is_customization_eligible(false, true, "claude", true, false));
        assert!(!is_customization_eligible(false, false, "claude", false, false));
        assert!(!is_customization_eligible(false, false, "codex", true, false));
    }

    #[test]
    fn delta_omits_defaults_and_cannot_unblock_inherited_plugin_skills() {
        let mut state = selection();
        state.set_plugin("tools@market", true);
        state.set_skill("standalone", true);
        state.set_skill("restricted", true);
        state.set_plugin_skill("tools@market", "tools:restricted", true);
        state.set_plugin_skill("tools@market", "tools:restricted", false);
        state.apply();
        assert!(state.payload(state.context.as_ref()).is_none());
        state.open();
        state.set_plugin_skill("tools@market", "tools:allowed", false);
        state.set_skill("inactive", true);
        state.set_plugin("extra@market", true);
        state.apply();
        let payload = serde_json::to_value(state.payload(state.context.as_ref()).unwrap()).unwrap();
        assert_eq!(payload, json!({"plugins":{"extra@market":true}, "skills":{"inactive":true}, "blocked_skills":["tools:allowed"]}));
        state.open();
        state.set_plugin("tools@market", false);
        state.set_plugin_skill("tools@market", "tools:allowed", true);
        assert_eq!(state.draft.as_ref().unwrap().blocked_skills, ["tools:allowed"]);
        state.set_plugin("tools@market", true);
        state.apply();
        assert_eq!(serde_json::to_value(state.payload(state.context.as_ref()).unwrap()).unwrap(), payload);
        state.open();
        state.set_plugin_skill("tools@market", "tools:allowed", true);
        state.set_skill("inactive", false);
        state.set_plugin("extra@market", false);
        state.apply();
        assert!(state.payload(state.context.as_ref()).is_none());
    }

    #[test]
    fn search_shows_matching_plugin_skills_and_restores_manual_expansion() {
        let mut state = selection();
        state.refilter("allowed");
        assert!(matches!(state.rows.as_slice(), [CatalogRow::Plugin(0), CatalogRow::PluginSkill(0, 0)]));
        assert!(state.is_plugin_expanded("tools@market", "allowed"));
        assert!(state.expanded.is_empty());
        state.refilter("");
        assert!(matches!(state.rows.as_slice(), [CatalogRow::Plugin(0), CatalogRow::Plugin(1)]));
        assert!(!state.is_plugin_expanded("tools@market", ""));
        state.expanded.insert("tools@market".into());
        state.refilter("restricted");
        assert!(matches!(state.rows.as_slice(), [CatalogRow::Plugin(0), CatalogRow::PluginSkill(0, 1)]));
        state.refilter("");
        assert!(matches!(state.rows.as_slice(), [CatalogRow::Plugin(0), CatalogRow::PluginSkill(0, 0), CatalogRow::PluginSkill(0, 1), CatalogRow::Plugin(1)]));
        assert!(state.is_plugin_expanded("tools@market", ""));
    }

    #[test]
    fn cancel_preserves_applied_and_restore_is_also_a_draft() {
        let mut state = selection();
        state.set_plugin("tools@market", false);
        state.apply();
        let applied = state.applied.clone();
        state.open();
        assert_eq!(state.draft.as_ref(), Some(&applied));
        state.restore();
        state.cancel();
        assert_eq!(state.applied, applied);
        state.open();
        state.restore();
        state.apply();
        assert!(state.payload(state.context.as_ref()).is_none());
    }

    #[test]
    fn machine_account_folder_and_unavailable_context_discard_choices_and_stale_replies() {
        for replacement in [Some(context("other", Some("account"), "/project")), Some(context("machine", None, "/project")),
            Some(context("machine", Some("account"), "/other")), None] {
            let mut state = selection();
            state.set_skill("standalone", false);
            state.apply();
            state.open();
            let old = state.context.clone();
            let seq = state.catalog.start();
            assert!(state.payload(replacement.as_ref()).is_none());
            assert!(state.set_context(replacement));
            assert!(!state.is_open());
            assert!(state.payload(old.as_ref()).is_none());
            assert!(!state.catalog.finish(seq, Ok(Default::default())));
            assert!(state.catalog.ok().is_none());
            assert_eq!(state.applied, ClaudeCustomizations::default());
        }
    }
}
