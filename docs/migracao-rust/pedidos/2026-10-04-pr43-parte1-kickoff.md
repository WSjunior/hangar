# Revisar e preparar o PR #43 (base `hangar-server-parte1`)

PR #43 do WSjunior, branch `fix/conta-chave-terminal`: "Troca de conta em sessão com terminal mantém a
chave do runtime em vez de responder 500" (+106/-2, 4 arquivos). Base: `hangar-server-parte1`.
Pasta deste cwd: branch local `pr43` = cabeça do PR.

Contexto: a migração vai reescrever a troca de conta (plano "dono único", Task 4: fechar no Rust →
ação → reabrir no Rust; `docs/migracao-rust/dono-unico/` na branch `plan/rust-single-owner`). Este
conserto vale até lá para quem testa pelo canal, e o teste dele precisa ficar como regressão.

## O que fazer

1. Leia descrição e diff (`gh pr view 43`, `gh pr diff 43`), `docs/migracao-rust/README.md` e o
   `CLAUDE.md` da raiz. Confirme a causa do 500 no código e reproduza com teste que falha sem o
   conserto (o PR deve ter; se não tiver, escreva).
2. Revisão por subagentes (`ecc:python-reviewer` e `ecc:silent-failure-hunter`; `ecc:rust-reviewer`
   se tocar `crates/`). Regra da migração atual: o Rust é o único dono do que já migrou; o conserto
   não pode passar a sessão ao Python com o Rust vivo.
3. Problema real → corrija na branch do PR (`git push https://github.com/WSjunior/hangar.git
   pr43:fix/conta-chave-terminal`; nunca force; sem comentar no PR).
4. Faça merge de `origin/hangar-server-parte1` na branch do PR (nunca rebase) para ela ficar em dia.
5. Testes: pytest dos arquivos tocados e dos de runtime (`tests/test_runtime_*.py`,
   `tests/test_trocar_conta.py`); `cargo test --locked --workspace` em `crates/` se tocar Rust.
   Checks do PR verdes job por job.
6. Não junte: me avise e eu junto na `hangar-server-parte1`.

Mande para `migracao-rust-2`: causa, o que mudou, testes com números, link dos checks.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; rodar instaladores; `push --force`;
mexer na `main` ou direto na `hangar-server-parte1`.
