# PRs abertos para a branch da migração (06/10/2026)

Você organiza os PRs abertos para a `hangar-server-parte1` e reporta à sessão coordenadora
`migracao-rust-3` (recados por `hangar-send migracao-rust-3 "..."`). Responda ao dono em pt-BR,
curto, uma pergunta por vez, opções letradas. O fluxo é o mesmo de
`docs/migracao-rust/pedidos/2026-10-05-organizar-prs-kickoff.md` (leia inteiro): uma sessão por PR,
revisão por subagentes da stack, correção na branch do PR com teste, sem comentar no PR. Leia
também `docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` e a seção "Desempenho: erros
que já custaram" de `docs/migracao-rust/README.md`. Repositório `jeffer1312/hangar`, `gh` logado
como `jeffer1312`.

## Estado de partida

- `main` já está toda dentro da `hangar-server-parte1` (`d1fd8b15c` juntou `origin/main`
  `bbee8084e`). Confira de novo com `git log origin/hangar-server-parte1..origin/main`: se a `main`
  andou, a sincronização é o passo 2 do fluxo antigo, antes de juntar os PRs.
- A `hangar-server-parte1` está em `ccd06c751`: **a lista de sessões e o estado agora são do Rust**
  (`crates/hangar-server/src/list/`, `ListHub`; o Python só fornece fatos por
  `/internal/list/facts`). Contrato interno **27**; mudou rota `/internal`, evento do
  `side-events` ou variável do filho → próximo número livre (28…), `RUST_SERVER_PROTOCOL` e
  `INTERNAL_PROTOCOL` juntos.

PRs abertos (confira com `gh pr list --state open`):

| PR | Base | Autor | O que é |
|---|---|---|---|
| #67 | `hangar-server-parte1` | WSjunior | `fix/worktree-merge-base`: detectar merges pela base remota de origem |
| #68 | `hangar-server-parte1` | WSjunior | `fix/native-desktop-usability`: resize do Terminal, Nova sessão e reinício no nativo |

Os dois aceitam push do mantenedor: `git push https://github.com/WSjunior/hangar.git <local>:<branch>`.

## Atenção nesses dois

- **#67 (worktrees):** a lista de worktrees (`/api/worktrees`, detalhe) e Git/arquivos são do Rust
  (`hangar-workspace`). Mudança de regra feita só no Python (`worktrees.py`, `git_ops.py`) não vale
  com o Rust de pé: leve ao Rust com teste de paridade. Mutações de worktree seguem no Python.
- **#68 (nativo):** `cargo test --locked` em `desktop-native/` (o CI do PR não testa o nativo).
- Regra do dono único: com o Rust de pé, falha em parte migrada vira erro com código, nunca passa
  ao Python.
- Merge de `origin/hangar-server-parte1` na branch do PR (nunca rebase) antes de terminar.

## Regras

- Contas: conta padrão; sessões filhas por `hangar-send --new <nome> <cwd>` sem `--conta`.
  Testes reais só com Haiku em backend isolado; nunca `claude-200-1` nem `claude-200-3`.
- Compilação: no máximo 2 `cargo` ao mesmo tempo, `CARGO_BUILD_JOBS=4`, `target/` por worktree e
  apagado ao terminar; desligue o plugin `rust-analyzer-lsp` nas worktrees das filhas
  (`.claude/settings.local.json`), ele sobe ~3 GB por sessão.
- **Não suba branch à toa:** cada push que toca `crates/` ou `desktop-native/` dispara Server e
  Native nos três sistemas e cria release. Teste no Linux da máquina; suba a branch do PR só quando
  estiver pronta.
- Sessões filhas não podem parar em cartão de permissão: vigia como no fluxo antigo.
- CI conferido **job por job**. Quem junta é a coordenação: avise `migracao-rust-3` com o PR, o
  hash e o resultado dos jobs; ela faz `gh pr merge N --merge --delete-branch`.
- Proibido: subir/reiniciar/parar o backend real; rodar instaladores; `push --force`; comentar em
  PR; mexer em workflow do CI; apagar branch que não seja a do PR juntado.
- Decisão que mude o que o dono vê → pergunte a ele e espere.
- Feche cada sessão filha ao terminar o PR dela; ao final, atualize o handoff (pendências novas) e
  mande o resumo à `migracao-rust-3`.
