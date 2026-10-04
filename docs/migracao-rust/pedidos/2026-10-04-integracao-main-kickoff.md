# Integração: PRs abertos na main e a branch do PR #24 em dia com a main

O dono quer testar a migração para Rust "como se fosse a main": a branch do PR #24
(`hangar-server-parte1`) com tudo o que estiver na `main`, mais a 2D e o PR #30 quando ficarem
prontos. Você conduz isso em duas etapas.

## Onde trabalhar

Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/integracao-main`, branch local
`integracao-main`, criada de `origin/hangar-server-parte1` em `5ea4e297`. Use `git -C <essa pasta>`.
Repositório `jeffer1312/hangar` (o `gh` desta máquina está logado como `jeffer1312`).

## Etapa 1 — agora: PRs que vão para a main

Revise e junte na `main`, nesta ordem: **#29, #32, #33, #34**, depois **#28** e **#31**. Para cada um:
1. Leia a descrição e o diff inteiro (`gh pr view N`, `gh pr diff N`), e leia o `CLAUDE.md` da raiz
   e as "Regras vigentes" de `docs/decisoes/*.md` que o diff tocar.
2. Revisão de código com subagentes conforme a stack do diff (`ecc:python-reviewer`,
   `ecc:typescript-reviewer`, `ecc:rust-reviewer`, `ecc:silent-failure-hunter`), em paralelo.
3. Problema real encontrado → não junte; comente no PR o que achou (`gh pr comment`) e me avise.
4. Sem problema e com os checks do GitHub verdes (`gh pr checks N`; espere os que estiverem rodando)
   → `gh pr merge N --merge --delete-branch`. Se o PR ficou desatualizado em relação à `main`
   (conflito ou check pedindo atualização), atualize a branch dele só se o conflito for trivial;
   senão comente e me avise.
5. Sobre o #33: a `hangar-server-parte1` já resolveu a mesma falha de outro jeito (`5ea4e297`, o
   observador do Rust deixou de ser read-only). O #33 continua válido como defesa extra no Python e
   não conflita; pode entrar se a revisão aprovar.

Ao terminar a etapa 1, mande o resumo (o que entrou, o que ficou e por quê) para `Migracao-Rust`.

## Etapa 2 — só quando `Migracao-Rust` mandar: a nossa branch em dia

Vai acontecer quando a 2D (`hangar-server-parte2d`, sessão `rust-parte2d-fim`) terminar.
1. `git fetch origin`; na `integracao-main`, junte `origin/hangar-server-parte1` (pode ter andado).
2. Junte a branch da 2D que eu indicar.
3. Junte `origin/main` (**merge, nunca rebase**: a branch do PR #24 é pública e não recebe
   `push --force`). Resolva os conflitos com os PRs #28/#31 preservando os dois lados.
4. Junte o PR #30 (`feat/rust-workspace-io`, base `hangar-server-parte1`). Ele também é parte da
   migração; se o contrato interno mudar, ele fica com o próximo número livre de
   `RUST_SERVER_PROTOCOL`/`INTERNAL_PROTOCOL` (subir os dois juntos).
5. Testes: `cargo test --locked --workspace` em `crates/`, `uv run pytest` inteiro em `backend/`
   (falhas conhecidas desta máquina: os testes do `omp`, que não está instalado, e 3 de
   `test_update_channel` que dependem da ordem — confirme que passam sozinhos), `npm run check` na
   raiz. Corrija o que a junção quebrar.
6. Push como avanço direto: `git push origin integracao-main:hangar-server-parte1`. Acompanhe o
   `server.yml` e o `ci.yml` nos três sistemas (`gh run watch`). Mande o resultado para
   `Migracao-Rust`.

## Regras

- Nunca suba, reinicie ou pare o backend nem o serviço `hangar-backend`; nunca rode instaladores;
  nunca `push --force`, `reset --hard` nem apagar branch que não seja a do PR juntado.
- Commits com mensagem descritiva em inglês, `git add` de caminhos explícitos; o `pre-commit` pede
  passo de atualização — mudança que não altera a máquina usa `HANGAR_SEM_PASSO=1`.
- Decisão que mude escopo ou comportamento visto pelo dono → pergunte a `Migracao-Rust` e espere.
