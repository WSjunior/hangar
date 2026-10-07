# Organizar: PRs abertos e a branch da migração em dia com a main

Você coordena este trabalho no lugar da `migracao-rust-2` (o dono foi para outra tarefa). Responda
ao dono em pt-BR, curto, uma pergunta por vez, opções letradas. Leia também
`docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` (estado e regras da migração).
Repositório `jeffer1312/hangar`, `gh` logado como `jeffer1312`.

## Contas

Tudo na **conta padrão** (`~/.claude`, "Felizardo e Batista"): crie as sessões de trabalho com
`hangar-send --new <nome> <cwd>` **sem** `--conta` (nascem na conta de quem chama) e confira o
campo de conta na resposta. Se a conta padrão bater o limite de uso, troque a conta da sessão
mantendo a conversa (`POST /api/sessions/<nome>/conta`), nunca feche e abra outra.

## O que fazer

PRs abertos agora (confira com `gh pr list --state open`; podem ter chegado outros):

| PR | Base | O que é |
|---|---|---|
| #51 | `main` | Bandeja no app nativo: manter o Hangar aberto ao fechar a janela (+1328) |
| #52 | `main` | Nativo: alinhar a linha do rótulo da faixa de mods com o Raster (+99) |
| #49 | `hangar-server-parte1` | Git da sessão segue a worktree onde o agente trabalha (+434) |
| #50 | `hangar-server-parte1` | Pele Terminal: ferramentas como no Claude Code (+3617) |
| #53 | `hangar-server-parte1` | Clique e painel de mod pelo app (+133) |

Além disso a `main` já tem 4 PRs juntados que a branch da migração não tem (#45, #46, #47, #48;
`git rev-list --count origin/hangar-server-parte1..origin/main` = 16 commits).

1. **Uma sessão por PR** (worktree própria na branch do PR; os do WSjunior aceitam push do
   mantenedor: `git push https://github.com/WSjunior/hangar.git <local>:<branch>`), que:
   - lê descrição e diff inteiro, o `CLAUDE.md` da raiz e as "Regras vigentes" de
     `docs/decisoes/*.md` que o diff tocar;
   - revisa com subagentes da stack em paralelo (`ecc:rust-reviewer` para `desktop-native`/`crates`,
     `ecc:typescript-reviewer`, `ecc:python-reviewer`, sempre `ecc:silent-failure-hunter`);
   - corrige problema real na branch do PR, um commit por problema com teste que falha sem ele;
     nunca `push --force`; **não comenta no PR**;
   - nos PRs para a `hangar-server-parte1`: confere a regra do dono único (com o Rust de pé, parte
     migrada não passa ao Python por falha) e faz merge de `origin/hangar-server-parte1` na branch
     do PR (nunca rebase) antes de terminar; contrato interno: se mudar, use o próximo número livre
     (hoje 20 → 21, 22…) e suba `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos;
   - roda os testes dos arquivos tocados (`cargo test --locked` em `desktop-native/` quando tocar o
     nativo — o CI do PR não compila o nativo; `cargo test --locked --workspace` em `crates/`;
     pytest; `npm run check` na raiz; falhas conhecidas: 3 erros em
     `mobile/src/features/ditado/useDitado.ts`, 4 testes de `semTerminal.test.tsx`);
   - confere os checks **job por job** e te avisa. Quem junta é você:
     `gh pr merge N --merge --delete-branch`.
2. **Depois que os PRs da `main` entrarem**, uma sessão traz a `main` para a migração: branch
   `sync-main-into-parte1` de `origin/hangar-server-parte1`, **merge** de `origin/main`, conflitos
   resolvidos preservando os dois lados. Onde a `main` mudou Python de algo que na migração já é do
   Rust (Git/arquivos/worktrees: `git_ops.py`, `filetree.py`, `fs.py`, `worktrees.py`; custos:
   `costs*.py`; histórico/eventos; runtime das sessões Claude), leve a mudança ao Rust com teste de
   paridade, sem deixar a parte migrada só no Python. `cargo test --locked --workspace`, pytest dos
   tocados, `npm run check`, CI do `server.yml` job por job; push da branch; você junta na
   `hangar-server-parte1` (merge, nunca rebase; push liberado).
3. Junte os PRs da `hangar-server-parte1` depois da sincronização, cada um com a branch em dia.
4. Ao terminar, atualize `docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` com o que
   entrou (hashes) e as pendências novas, e avise o dono com um resumo curto: o que entrou, o que
   foi corrigido em cada PR, o que ficou e por quê.

## Regras

- Sessões filhas não podem parar em cartão de permissão: mantenha um vigia que lê
  `GET /api/sessions` (token `CP_AUTH_TOKEN` de `/home/jefferson/hangar/backend/.env`, porta 8765) e
  responde `POST /api/sessions/<nome>/select {"option":1}` quando o cartão é Yes/No.
- Não espere CI à toa: siga com o próximo e confira quando terminar.
- Feche cada sessão de trabalho ao juntar o PR dela (`hangar-send --close`).
- Proibido: subir/reiniciar/parar o backend real ou o `hangar-backend`; rodar instaladores;
  `push --force`; comentar em PR; apagar branch que não seja a do PR juntado.
- Decisão que mude o que o dono vê (comportamento novo, remoção) → pergunte a ele e espere.
