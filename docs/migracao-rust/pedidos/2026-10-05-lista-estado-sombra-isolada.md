# Sombra da lista com sessões reais no backend isolado (substitui o Step 34)

Decisão do dono (05/10/2026): **sem sombra no backend real.** O Step 34 da Task 15 (2 dias no canal
de testes) vira este teste. Siga as regras de `2026-10-05-lista-estado-task-kickoff.md` (sem push,
`cargo-slot`, sem LSP). Branch `feat/session-list-state-sombra`, a partir da integração.

## Montagem

- Backend isolado no molde de `scripts/prova-dono-unico.py`: `HOME` temporário, portas fora de
  8765/8766/8768 (e das outras sessões), `CP_AUTH_TOKEN` próprio, `tmux -L` próprio,
  `matar_orfaos` trocado por no-op **antes** de subir (sem isso ele mata canos reais),
  `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`; binário Rust em release da sua branch.
- `CP_LIST_SHADOW=1` no ambiente desse backend: cada diferença Rust × Python vai ao diário
  (`rust.list_shadow_diff`, `rust.list_shadow_failed`, `rust.list_shadow_blind`).
- Sessões Claude só em Haiku (`claude-haiku-4-5`); Codex em `gpt-6-luna`. Com e sem terminal
  nos dois.
- Pode virar um roteiro versionado (`scripts/prova-lista-sombra.py`) se ficar repetível.

## Cenários mínimos

nascer; trabalhando → parada; parada em cartão de permissão e em AskUserQuestion; `/clear`; matar a
sessão; fechar e recriar com o mesmo nome; par e grupo; worktree com branch; troca de conta; plano
com barra; restart do backend com sessão viva.

## O que entregar

Para cada cenário: diferenças achadas (sessão + campo, do diário), causa, o que foi corrigido
(commit na sua branch, com teste que falha sem a correção e golden regenerado se a regra estiver no
gerador) e o tempo por atualização da lista (Rust e Python). Diferença aceita só com motivo escrito
em `desenho.md`, seção "Sombra (Task 15)". Ao fim: Step 34 do `plano.md` reescrito com o resultado.
Relatório em `docs/migracao-rust/lista-estado/sombra-isolada.md` e resumo a `lista-org`.

No fim, pare tudo: backend, `tmux -L`, sessões; apague o `HOME` e o `target/`.
