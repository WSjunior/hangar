# Executar o plano do dono único — Tasks 1 a 6 (sequenciais)

Plano aprovado pelo dono em 04/10/2026 (decisões 1A, 2A, 3A registradas em `desenho.md`):
`docs/migracao-rust/dono-unico/plano.md`, com `inventario.md` e `desenho.md` na mesma pasta.
Pasta deste cwd: branch `feat/rust-single-owner` = `origin/hangar-server-parte1` (`70c626d6`, já com a
`main` e o PR #43) + o plano juntado (`8fdbf38c`).

## Como executar

- Faça as Tasks **1, 2, 3, 4, 5 e 6, nesta ordem**, como o plano descreve. As Tasks 7, 8 e 9 rodam em
  paralelo em outras sessões, a partir do seu commit final da Task 1: **assim que a Task 1 estiver
  commitada, pushada e com CI verde, me avise na hora** (`hangar-send migracao-rust-2 "Task 1 pronta: <hash>"`)
  e siga para a 2 sem esperar.
- Antes de cada Task, reconfira as linhas citadas na base atual (o plano foi conferido em `665fac8e`).
- Por Task: teste que falha sem a mudança, código morto removido no mesmo passo, a regra de
  `CLAUDE.md`/`docs/` que ela contradiz corrigida no mesmo commit, revisão independente por subagente
  (`ecc:python-reviewer`/`ecc:rust-reviewer` + `ecc:silent-failure-hunter`) e CI do `server.yml` verde
  **job por job** nos três sistemas (o resumo da rodada fica verde com job vermelho; workflow fora do
  filtro de caminho → dispare à mão). Marque `- [ ]` → `- [x]` no `plano.md` a cada Step.
- Testes automatizados por Task: decisão do coordenador (registrada no plano).
- Contrato interno: a tabela única 14/15/16/17 do plano; suba `RUST_SERVER_PROTOCOL` e
  `INTERNAL_PROTOCOL` juntos.
- Testes reais (quando o Step pedir): backend de teste isolado (`HOME` temporário, portas livres
  diferentes de 8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio), sessões Claude só Haiku
  (`--model claude-haiku-4-5`), `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`. Pare tudo e
  apague o `HOME` ao fim de cada rodada.
- Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só desta branch.
- Ao fim de cada Task, mande a `migracao-rust-2` uma linha: Task, hash, testes, link do CI.
- Achado que mude o desenho ou o que o dono vê → pergunte a `migracao-rust-2` e espere; o resto,
  decida e registre no plano.
- Se o seu contexto passar de 50%, avise antes de continuar (troca de sessão no meio do plano).

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux padrão
ou em sessão real; rodar instaladores; mexer na `main` ou na `hangar-server-parte1`; `push --force`;
modelo diferente de Haiku nas sessões de teste; log com texto de conversa.
