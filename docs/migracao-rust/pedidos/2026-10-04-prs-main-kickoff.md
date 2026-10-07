# Revisar e juntar os PRs abertos para a main, depois trazer a main para a branch da migração

## PRs (repositório `jeffer1312/hangar`, `gh` logado como `jeffer1312`)

| PR | Branch | Observação |
|---|---|---|
| #41 | WSjunior `fix/native-send-and-tool-tree` (`c8407ae4`) | já revisado e corrigido hoje pela coordenação (app nativo); falta só conferir os checks e juntar |
| #40 | WSjunior `feat/session-model-label` | nome do modelo sem a barra do Hangar, rótulo Bypass, card da barra lateral (18 arquivos) |
| #42 | jeffer1312 `feat/worktrees-redesign` | worktrees: idade, sessões, uso de disco, criar pela lista (+3218, 16 arquivos). Atenção: Git/arquivos já foram migrados para o Rust na `hangar-server-parte1` (PR #30, `crates/hangar-workspace`); anote o que este PR muda em operações de Git/worktree para a etapa 2 |

Ordem: #41, #40, #42 (do menor para o maior).

## Para cada PR

1. Leia descrição e diff inteiro (`gh pr view N`, `gh pr diff N`), o `CLAUDE.md` da raiz e as
   "Regras vigentes" de `docs/decisoes/*.md` que o diff tocar.
2. Revisão por subagentes conforme a stack, em paralelo (`ecc:rust-reviewer` para `desktop-native`,
   `ecc:typescript-reviewer` para Svelte/TS, `ecc:python-reviewer`, `ecc:silent-failure-hunter`).
3. Problema real → corrija na branch do PR, um commit por problema com teste que falha sem ele
   (branches do WSjunior aceitam push do mantenedor:
   `git push https://github.com/WSjunior/hangar.git <local>:<branch>`; a do #42 é deste repositório).
   Nunca `push --force`; não comente nos PRs (o dono não quer comentários).
4. Checks verdes **job por job** (`gh pr checks N`; o resumo da rodada pode ficar verde com job
   vermelho) e o app nativo compilando com `cargo test --locked` em `desktop-native/` quando o PR
   tocar nele (o CI do PR não compila o nativo).
5. `gh pr merge N --merge --delete-branch`.

## Etapa 2 — trazer a main para a branch da migração

Depois dos três: no worktree deste cwd, `git fetch origin`; crie a branch `sync-main-into-parte1` a
partir de `origin/hangar-server-parte1` e faça **merge** de `origin/main` (nunca rebase). Resolva os
conflitos preservando os dois lados. Onde a `main` mudou Python de Git/arquivos/worktree que na
migração já é atendido pelo Rust (`backend/app/git_ops.py`, `filetree.py`, `fs.py`, `worktrees.py`
× `crates/hangar-workspace`), leve a mudança também ao Rust, com teste de paridade
(`test_workspace_parity.py`, `examples/workspace_contract.rs`) — não deixe a parte migrada só no
Python. Rode `cargo test --locked --workspace` em `crates/`, o pytest dos arquivos tocados e
`npm run check` na raiz. Push dessa branch e me avise; eu junto na `hangar-server-parte1`.
A sessão `pr43-parte1` está juntando o PR #43 na `hangar-server-parte1` ao mesmo tempo: faça o
merge de `origin/hangar-server-parte1` mais recente logo antes do push.

## Entrega

Mande para `migracao-rust-2`: o que entrou (hash do merge), o que foi corrigido em cada PR, o que
ficou e por quê, e o resultado da etapa 2. Decisão que mude o que o dono vê → pergunte a
`migracao-rust-2` e espere.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; rodar instaladores; `push --force`;
mexer direto na `hangar-server-parte1` (só a branch `sync-main-into-parte1`).
