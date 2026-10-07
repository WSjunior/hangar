# Parte 5: contrato entre a metade Claude e a metade Codex

Versão 2 (07/10/2026, mudanças da metade Codex sobre a v1). Metade Claude: sessão `rust-parte5-claude`, notebook, branch
`hangar-server-parte5-claude`. Metade Codex: sessão `jefferson-felizardo::rust-parte5-codex`, PC
de casa, branch `hangar-server-parte5-codex`. Sem push nas duas: o texto viaja por recado e cada
lado guarda a cópia na própria branch; quem junta por último leva a versão final.

## Quem faz o quê

| Peça | Dono | Quando |
|---|---|---|
| **5-0 caminho de escrita**: tabela de despacho (rota × provedor × modo → Rust/Python) em `rs/session_write/`, `enum Provider { Claude, Codex }` sobre o ator, sem trait; rotas `/input`, `/steer`, `/interrupt`, `/select`, `/select/submit`, `/answer`, `/keys`, `/term-input`, `DELETE …/queue/{id}` reivindicadas no Rust | Claude escreve, Codex revisa | primeira junção, antes da 5B |
| `runtime_coordinator`: `_born_in_rust`, `settle_before_queue`, `op` e `await_mode` perguntam à tabela se o Rust é dono do provedor e do modo (`rc:515`, `:1026`, `:1248`) | Claude, na 5-0 | 5-0 |
| `prepare_prompt`, `format_status` (texto dos dois provedores) e `skill_catalog` no Rust; cota do Claude como fato empurrado pelo Python; a do Codex passa a ser do próprio Rust na 5E e o `format_status` lê dela sem ir ao Python. Texto do Codex bate por golden com `format_status_line` (`codex/adapter.py:319-345`), inclusive `model || default_model` e `effort || default_effort`, com os campos vindos do motor Codex | Claude, na 5-0 | 5-0 |
| Apagar ramos mortos da política (`answer_body`, `session.marker`, `diag.error`, `quota`) | Claude, na 5-0 | 5-0 |
| Linhas Codex da tabela viradas para `Rust` (sem terminal na 5B, com terminal na 5C) | Codex | depois da 5-0 |
| Rotas só do Codex: `/model`, `/models`, `/question/skip`, `/service-tier`, `/codex/*`, `/limits`, `/codex-permissions` | Codex | 5B/5C |
| `/rename`, `DELETE`, `/recarregar`, `/keys`, `/term-input` e `DELETE …/queue/{id}` com provedor Codex: repassados ao Python pela tabela (Codex com terminal só vira na 5C) | Codex vira quando plugar 5B/5C/5E | — |
| Criar sessão (`POST /api/sessions`) | **parte 6**, as duas metades concordam (aguarda o dono). Na 5E a metade Codex leva para o Rust só as checagens Codex da criação (escolha de conta, catálogo de modelo, reserva), chamadas pela rota Python por `/internal` | 5E (Codex) |
| `/commands` (leitura) | fora da 5-0: Claude na C4, Codex na 5B | — |
| Plugin do Claude (C2), fatos do terminal (C3), administração sem empréstimo (C4), ciclo de vida do Claude (C5) | Claude | depois da 5-0 |

## Regras dos arquivos que as duas mexem

- **Contrato interno:** cada junção na `main` sobe um número (`RUST_SERVER_PROTOCOL` em
  `backend/app/rust_server.py` e `INTERNAL_PROTOCOL` em `crates/hangar-server/src/lib.rs`, juntos).
  Quem junta primeiro pega o próximo livre; o outro rebaseia e pega o seguinte. Hoje: 36.
- **`runtime_policy.py` e `internal_api.py`:** cada lado tira só os ramos do próprio provedor;
  os comuns saem na 5-0.
- **`routes.rs` e `migration_status.rs`:** a reivindicação das rotas comuns é da 5-0; depois dela
  cada lado só muda linhas da tabela de despacho e as rotas exclusivas dele.
- **Formato de erro e golden:** toda rota que o Rust passa a atender devolve o mesmo corpo do
  Python, provado por fixture gerada pela rota Python.
- **Uso real:** no backend do notebook do dono, pelo canal de testes, feito por ele com a sessão
  `hangar`. Nenhuma das metades sobe nada lá; cada uma avisa quando a branch estiver pronta.

## Em aberto

- Criar sessão: parte 5 ou parte 6 (decisão do dono).
- Revisão da 5-0: a metade Codex recebe o plano da 5-0 antes da execução e o diff no fim.
