# Executar uma das Tasks paralelas (7, 8 ou 9) do plano do dono único

O kick-off diz qual Task é a sua. Plano aprovado pelo dono em 04/10/2026:
`docs/migracao-rust/dono-unico/plano.md` (com `inventario.md` e `desenho.md`, decisões 1A/2A/3A).
Pasta deste cwd: branch `feat/rust-single-owner-taskN`, criada do commit da Task 1 (`9cbe3cd1`,
contrato 14). A sessão `dono-unico-exec` faz as Tasks 2→6 em `feat/rust-single-owner` ao mesmo tempo.

## Regras

- Faça só a sua Task, como o plano descreve. Não toque em `runtime_coordinator.py` (é da sequência
  2→6); se precisar, pare e pergunte a `migracao-rust-2`.
- Reconfira as linhas citadas na base (o plano foi conferido em `665fac8e`).
- Teste que falha sem a mudança, código morto removido no mesmo passo, a regra de `CLAUDE.md`/`docs/`
  que a Task contradiz corrigida no mesmo commit, revisão independente por subagente
  (`ecc:rust-reviewer`/`ecc:python-reviewer` + `ecc:silent-failure-hunter`), CI do `server.yml`
  verde **job por job** nos três sistemas (fora do filtro de caminho → dispare à mão). Marque os
  Steps da sua Task no `plano.md`.
- Contrato interno: use o número que a tabela única do plano dá à sua Task (a 8 leva o 17); suba
  `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos.
- Testes reais quando o Step pedir: backend de teste isolado (`HOME` temporário, portas livres
  diferentes de 8765/8766/8768 e das outras sessões — escolha uma faixa sua, `CP_AUTH_TOKEN`
  próprio, `tmux -L` próprio), sessões Claude só Haiku (`--model claude-haiku-4-5`),
  `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`. Pare tudo e apague o `HOME` no fim.
- Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só da sua branch.
  Não junte: a junção na `feat/rust-single-owner` acontece depois da Task 6, na ordem 7, 8, 9.
- Ao terminar, mande a `migracao-rust-2`: Task, hash, o que mudou, testes, link do CI.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux padrão
ou em sessão real; rodar instaladores; mexer na `main`, na `hangar-server-parte1` ou na
`feat/rust-single-owner`; `push --force`; modelo diferente de Haiku; log com texto de conversa.
