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
