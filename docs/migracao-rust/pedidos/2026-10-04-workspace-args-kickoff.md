# Levar ao núcleo Rust os argumentos de Git/arquivos que ainda caem no Python

## Contexto

O PR #30 moveu Git e arquivos para o `hangar-workspace` (Rust), mas na junção com a `main` cinco
argumentos que a `main` usa não existiam no núcleo Rust. A ponte (`backend/app/workspace_bridge.py`,
`delegate(..., python_args=...)`) tira esses argumentos do pedido quando estão no padrão e, fora do
padrão, roda a função inteira no Python:

- `create_worktree(new_branch=..., base=...)` — `backend/app/git_ops.py` (`_PYTHON_ARGS`);
- `remove_worktree(force=...)` — idem; usado agora pela remoção em lote com perda confirmada
  (`worktrees.delete(..., confirm=True)`, PR #35, juntado em `1b2be6f9`);
- `rows` de `find_elsewhere` (`backend/app/api.py` `_cited_elsewhere`), `citation_cwds` e
  `cited_elsewhere` (`backend/app/transcript.py`).

O dono quer que o que a `main` acrescentou em partes já migradas fique no Rust, não no Python.

## O que fazer

1. Para cada argumento, leia o Python atual (comportamento e erros, incluindo os casos que os
   testes de `test_worktrees.py` e `test_workspace_*.py` cobrem) e implemente igual no
   `crates/hangar-workspace` e nas rotas/contrato (`crates/hangar-api/src/workspace.rs`,
   `crates/hangar-server/src/workspace_routes.rs`). Mesmos códigos de erro e formato de resposta.
2. Tire os argumentos de `python_args` só quando o Rust os atender; a reserva para o Python continua
   valendo para falha antes de efeito (regra da migração). Mutação com resultado incerto nunca se
   repete.
3. Paridade: estenda `examples/workspace_contract.rs` e `test_workspace_parity.py` para os cinco, e o
   teste que compara assinaturas Python × campos Rust. Teste que falha sem o conserto para cada um.
4. Uso real num backend de teste isolado desta branch (`HOME` temporário, portas livres diferentes
   de 8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio; a branch tem `bb80cf31`), com
   repositório Git de teste: criar worktree com branch nova e com base, remover worktree suja com
   `force` pela remoção em lote confirmada, arquivo citado em outra pasta. Confira no
   `hangar-server.log`/diário que foi o Rust que atendeu (sem `workspace.reserva_python`). Pare tudo e
   apague o `HOME` no fim. Anote em `docs/migracao-rust/git-arquivos/argumentos-restantes.md`.
5. `cargo test --locked --workspace`, `cargo check` do `desktop-native` se tocar no núcleo
   compartilhado, pytest dos arquivos tocados. Revisão independente por subagente
   (`ecc:rust-reviewer` + `ecc:silent-failure-hunter`) antes do push.

## Entrega

Branch `feat/workspace-missing-args` (este cwd, de `origin/hangar-server-parte1` `1b2be6f9`).
Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Se mudar rota
`/internal` ou campo do contrato interno, suba `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos
para o próximo número livre (hoje 12 → 13). Push só desta branch, CI do `server.yml` verde nos
três sistemas. Mande para `migracao-rust-2`: o que foi para o Rust, testes com números, casos reais,
link do CI. Não junte na `hangar-server-parte1`.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux
padrão ou em sessão real; rodar instaladores; mexer na `main` ou na `hangar-server-parte1`.
