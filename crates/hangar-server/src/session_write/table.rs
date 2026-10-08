//! Tabela de despacho das escritas: quem atende cada rota para cada tipo de sessão. Só a tabela;
//! o corpo de cada rota mora nos módulos irmãos.
use axum::body::Bytes;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Provider { Claude, Codex }

impl Provider {
    /// `None` para provedor que o Rust não escreve (Pi, omp, Kimi): a rota segue ao Python.
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(name: &str) -> Option<Provider> {
        match name { "claude" => Some(Provider::Claude), "codex" => Some(Provider::Codex), _ => None }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WriteRoute { Input, Steer, Interrupt, Select, SelectSubmit, Answer, Keys, TermInput, QueueRemove }

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Owner { Rust, Python }

/// Entrada doente sempre vai ao Python (ele relança e responde como hoje). A metade Codex ainda é
/// toda dele.
pub fn decide(route: WriteRoute, provider: Provider, terminal: bool, healthy: bool) -> Owner {
    if !healthy { return Owner::Python; }
    match provider {
        Provider::Codex => Owner::Python,
        // Teclas cruas e a aba Submit só existem num pane: sem terminal quem responde é o Python, como hoje.
        Provider::Claude => match route {
            WriteRoute::Keys | WriteRoute::TermInput | WriteRoute::SelectSubmit if !terminal => Owner::Python,
            _ => Owner::Rust,
        },
    }
}

impl Provider {
    pub fn name(self) -> &'static str { match self { Provider::Claude => "claude", Provider::Codex => "codex" } }
}

/// Provedor+modo (`true` = sem terminal) que a tabela entrega ao Rust na entrada de texto: é o que a
/// saúde anuncia, para o Python não guardar a mesma lista.
pub fn owned_modes() -> Vec<(Provider, bool)> {
    let mut owned = Vec::new();
    for provider in [Provider::Claude, Provider::Codex] {
        for headless in [true, false] {
            if decide(WriteRoute::Input, provider, !headless, true) == Owner::Rust { owned.push((provider, headless)); }
        }
    }
    owned
}

/// O corpo tem a forma mínima que a rota espera? Só a forma: campo a mais ou de tipo errado é do
/// FastAPI, que o recusa quando o pedido é repassado.
pub(crate) fn body_ok(route: WriteRoute, body: &Bytes) -> bool {
    use WriteRoute::*;
    let object = || serde_json::from_slice::<serde_json::Value>(body).is_ok_and(|v| v.is_object());
    match route {
        Input | Select | Answer | Keys | TermInput => object(),
        Steer => body.is_empty() || object(),
        Interrupt | SelectSubmit | QueueRemove => true,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use WriteRoute::*;

    const ALL: [WriteRoute; 9] = [Input, Steer, Interrupt, Select, SelectSubmit, Answer, Keys, TermInput, QueueRemove];

    #[test]
    fn healthy_claude_with_a_terminal_is_rust_on_every_route() {
        for route in ALL { assert_eq!(decide(route, Provider::Claude, true, true), Owner::Rust, "{route:?}"); }
    }

    #[test]
    fn healthy_claude_without_a_terminal_is_rust_except_pane_only_routes() {
        for route in ALL {
            let want = if matches!(route, Keys | TermInput | SelectSubmit) { Owner::Python } else { Owner::Rust };
            assert_eq!(decide(route, Provider::Claude, false, true), want, "{route:?}");
        }
    }

    #[test]
    fn unhealthy_entry_goes_to_python() {
        for route in ALL { for terminal in [true, false] {
            assert_eq!(decide(route, Provider::Claude, terminal, false), Owner::Python, "{route:?}");
        } }
    }

    #[test]
    fn codex_is_python_for_now() {
        for route in ALL { for terminal in [true, false] {
            assert_eq!(decide(route, Provider::Codex, terminal, true), Owner::Python, "{route:?}");
        } }
    }

    #[test]
    fn owned_modes_come_from_the_table() {
        assert_eq!(owned_modes(), vec![(Provider::Claude, true), (Provider::Claude, false)]);
    }

    #[test]
    fn provider_names() {
        assert_eq!(Provider::from_str("claude"), Some(Provider::Claude));
        assert_eq!(Provider::from_str("codex"), Some(Provider::Codex));
        assert_eq!(Provider::from_str("pi"), None);
    }

    #[test]
    fn body_shape() {
        let b = |s: &'static str| Bytes::from_static(s.as_bytes());
        assert!(body_ok(Input, &b(r#"{"text":"oi","x":1}"#)));
        assert!(!body_ok(Input, &b("oi")) && !body_ok(Input, &b("[]")) && !body_ok(Input, &b("")));
        assert!(body_ok(Steer, &b("")) && body_ok(Steer, &b("{}")) && !body_ok(Steer, &b("x")));
        assert!(body_ok(Interrupt, &b("")) && body_ok(QueueRemove, &b("")));
    }
}
