# Parte 5: o que a 5B (e seguintes) herda

Entrada do plano da 5B. Origem: revisão final da 5A (PR #107) e o recado da metade Claude ao
concluir a 5-0 (07/10/2026, `7cb358073` na `hangar-server-parte5-claude`, contrato 37; detalhe em
`docs/migracao-rust/parte5-claude/registro-5-0.md` daquela branch).

## Da 5-0 (caminho de escrita comum)

- Ligar o Codex é mudar as linhas Codex de `crates/hangar-server/src/session_write/table.rs`
  (`decide`). O health anuncia `owns` a partir dela e o Python lê em `rust_owns`.
- `_born_in_rust = headless && rust_owns(provider, true)`: Codex com terminal precisa de gancho
  próprio na 5C.
- `start_sessions` pode rodar antes do primeiro `configure_transport` e ver o `owns` padrão (só
  Claude): tratar quando o Codex passar a ser do Rust.
- `format_status` do Codex já roda no Rust, mas `default_model`/`default_effort` não vêm no
  payload do motor Codex (decidir na 5E).
- A porta de entrada é por nome, e a troca de agente a segura (recusa na hora).

## Da 5A

- **Aviso de versão:** conferida 0.159.3; o notebook roda 0.160.1, então toda sessão Codex mostra
  `codex_versao_nao_conferida` (no nativo marca a sessão como problema na lista). Antes do uso
  real: `scripts/conferir-codex-schema` com a 0.160, ou mover o aviso para fora de `problema`.
- `turn/completed` ilegível com `status: "failed"` não marca `headless_turno_erro` no caminho cru.
- `command` vazio na aprovação ainda oferece "Sempre permitir" (tratar vazio como ilegível).
- O cliente aborta a leitura sem drenar as últimas linhas quando o escritor morre.
- Pedido do servidor de outra thread (subagente) ainda é descartado pelo filtro de thread.
- `spawn_stdio` no Windows: resolver `codex.cmd` (PATHEXT) e matar a árvore (`taskkill /T`).
