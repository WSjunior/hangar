# Executar uma Task do plano da lista + estado (Fases 0–B)

O recado diz qual Task é a sua. Plano aprovado pelo dono em 05/10/2026 (opção A):
`docs/migracao-rust/lista-estado/plano.md`, com `desenho.md`, `inventario.md` e `medicao.md`.
Sua pasta e branch (`feat/session-list-state-tN`) saem da integração `feat/session-list-state`.
Quem coordena e junta é a sessão `lista-org`.

## Regras

- Leia antes `CLAUDE.md` da raiz, `docs/migracao-rust/README.md` ("O Rust é o único dono do que
  já migrou") e, no plano, as "Restrições globais", o "Foco da revisão" e a sua Task.
- Faça só a sua Task, como o plano descreve. Reconfira as linhas citadas na base antes de codar
  (o plano foi conferido em `f98798166`). Arquivo de outra Task: não toque; precisou, pergunte a
  `lista-org`.
- Teste que falha sem a mudança (visto falhar), código morto removido no mesmo passo, a regra de
  `CLAUDE.md`/`docs/` que a Task contradiz corrigida no mesmo commit, revisão independente por
  subagente da stack (`ecc:rust-reviewer`/`ecc:python-reviewer`) + `ecc:silent-failure-hunter`.
  Marque `- [x]` nos Steps da sua Task no `plano.md`.
- Testes: só os dos arquivos tocados (comandos nas "Restrições globais").
- **Cargo: sempre por `~/.cache/hangar-lista/cargo-slot <args>`** (fila de no máximo 2 `cargo`
  na máquina, `CARGO_BUILD_JOBS=4`), nunca `cargo` direto. Cada worktree usa o próprio `target/`;
  apague-o ao terminar. Não use a ferramenta LSP nem suba rust-analyzer: o `cargo check` dele fura
  a fila e já esgotou a memória da máquina.
- Contrato interno: só a Task 14 sobe (23), `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos.
- Testes reais, se o Step pedir: backend isolado (`HOME` temporário, portas livres fora de
  8765/8766/8768 e das outras sessões, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio, lançador com
  `matar_orfaos` desligado), sessões Claude só Haiku (`--model claude-haiku-4-5`),
  `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`. Pare tudo e apague o `HOME` no fim.
- Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. **Sem push** (pedido
  do dono: cada push em `crates/` dispara CI nos três sistemas e cria release). Só commit local;
  `lista-org` junta pela branch local, e o CI roda quando a integração sobe no fim do lote. Para
  trazer a integração: `git merge feat/session-list-state` (local).
- Ao terminar, mande a `lista-org` (`hangar-send lista-org "…"`): Task, hash, o que mudou, testes
  rodados e resultado, revisão (o que apontou e o que foi feito).
- Travou numa decisão que não está no plano: pergunte a `lista-org`, não ao dono.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux padrão
ou em sessão real; rodar instaladores; mexer na `main`, na `hangar-server-parte1` ou na
`feat/session-list-state`; `git push` de qualquer branch; modelo diferente de Haiku nos testes reais; log com
texto de conversa.
