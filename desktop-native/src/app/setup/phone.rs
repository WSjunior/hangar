//! Tela 5: o QR do pareamento, pedido ao backend como o diálogo de pareamento já faz (`machines/pair.rs`). A escolha do
//! endereço é regra do assistente: Tailscale se respondeu, senão Wi-Fi (o diálogo escolhe o mais rápido). E a conferência da
//! conta Tailscale pelo `tailscale status`, para a tela 4 marcar a conta sem esperar o script.
use super::*;
use super::super::machines::{parse_reach, Kind, Reach, Status};
#[cfg(test)]
use super::super::machines::Address;
use std::process::Command;

pub(crate) struct Pairing { pub url: String, pub tailscale: bool, pub svg: Vec<u8> }

/// O que a tela 5 mostra. A imagem é feita na thread da janela, uma vez, e reaproveitada a cada quadro.
pub(crate) enum Qr { Idle, Loading, Shown { url: String, tailscale: bool, image: Arc<Image> }, NoAddress, Failed(String) }

pub(crate) fn pick(reach: &Reach) -> Option<Kind> {
    let answered = |kind: Kind| reach.addresses.iter().any(|a| a.kind == kind && a.status == Status::Ok);
    [Kind::Tailscale, Kind::Lan].into_iter().find(|kind| answered(*kind))
}

/// `Ok(None)`: nenhum endereço respondeu para o celular.
pub(crate) async fn load(api: Api) -> Result<Option<Pairing>, String> {
    let value = api.server_read(&["alcance"], &[], 30).await.map_err(|e| Hangar::failure(&e))?;
    let reach = parse_reach(&value).ok_or_else(|| tr("invalid_response"))?;
    let Some(kind) = pick(&reach) else { return Ok(None) };
    let value = api.server_read(&["alcance", "pareamento"], &[("endereco", kind.raw())], 15).await.map_err(|e| Hangar::failure(&e))?;
    match (value.get("url").and_then(Value::as_str), value.get("qr_svg").and_then(Value::as_str)) {
        (Some(url), Some(svg)) => Ok(Some(Pairing { url: url.to_owned(), tailscale: kind == Kind::Tailscale, svg: svg.as_bytes().to_vec() })),
        _ => Err(tr("invalid_response")),
    }
}

pub(crate) fn tailscale_running(json: &str) -> bool {
    serde_json::from_str::<Value>(json).ok().and_then(|v| v.get("BackendState")?.as_str().map(|state| state == "Running")).unwrap_or(false)
}

pub(crate) fn tailscale_status() -> bool {
    let path = super::system::refreshed_path();
    let Some(program) = super::system::find_program("tailscale", &path) else { return false };
    super::system::hidden(&mut Command::new(program)).args(["status", "--json"]).env("PATH", &path).output()
        .is_ok_and(|out| tailscale_running(&String::from_utf8_lossy(&out.stdout)))
}

#[cfg(test)]
mod tests {
    use super::*;
    // O glob da gpui_kit (via `use super::*`) traz um `test` que colide com o atributo padrão.
    use core::prelude::v1::test;

    fn reach(list: &[(Kind, Status)]) -> Reach {
        Reach { loopback: false, bind: String::new(),
            addresses: list.iter().map(|&(kind, status)| Address { kind, url: format!("https://{kind:?}"), status, ms: Some(10) }).collect() }
    }

    #[test]
    fn tailscale_wins_when_it_answered() {
        assert_eq!(pick(&reach(&[(Kind::Lan, Status::Ok), (Kind::Tailscale, Status::Ok)])), Some(Kind::Tailscale));
        assert_eq!(pick(&reach(&[(Kind::Lan, Status::Ok), (Kind::Tailscale, Status::Failed)])), Some(Kind::Lan));
        assert_eq!(pick(&reach(&[(Kind::Here, Status::Ok), (Kind::Public, Status::Ok), (Kind::Lan, Status::Unset)])), None);
    }

    #[test]
    fn tailscale_status_needs_running_backend() {
        assert!(tailscale_running(r#"{"BackendState":"Running","Self":{}}"#));
        assert!(!tailscale_running(r#"{"BackendState":"NeedsLogin"}"#));
        assert!(!tailscale_running("não é json"));
    }
}
