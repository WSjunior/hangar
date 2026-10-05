# Adiantar o fim do plano do dono único enquanto as Tasks 4–6 rodam

Plano: `docs/migracao-rust/dono-unico/plano.md`. A sessão `dono-unico-exec2` faz as Tasks 4→6 em
`feat/rust-single-owner` (hoje em `7b84fb63`, Tasks 1–3 feitas). As Tasks 7, 8 e 9 estão prontas em
`origin/feat/rust-single-owner-task7` (`b0b2f5c1`), `-task8` (`1bf78bf9`) e `-task9` (`72f34074`),
todas criadas do commit da Task 1. O kick-off diz qual das duas frentes abaixo é a sua.

## Frente A — junção antecipada das Tasks 7, 8 e 9 (branch `feat/rust-single-owner-789`)

1. Na pasta deste cwd (branch criada de `origin/feat/rust-single-owner`), faça **merge** das três na
   ordem 7, 8, 9 (nunca rebase). Combinado entre elas: a 7 tirou os usos do `Fallback` em
   history/events, a 8 tirou os do workspace; aqui o struct `Fallback` inteiro e o teste
   `routes.rs:567-602` saem. Formato do 503 nas duas: `{ok:false,error_code,message,detail:{code,
   params:{motivo},msg}}`. Contrato: a tabela única do plano (a 8 leva o 17); confira que os números
   ficam coerentes com o que a sequência 4→6 vai usar (15, 16).
2. Conflitos com as Tasks 2 e 3 (já na base): preserve os dois lados; a 9 e a 3 mexem nas mesmas
   linhas de `problema.ts`/`SessionProblem.tsx`.
3. `cargo test --locked --workspace`, pytest dos arquivos tocados, `npm run check` na raiz (falhas
   conhecidas: 3 erros de tipo em `mobile/src/features/ditado/useDitado.ts`, 4 testes do
   `semTerminal.test.tsx`, ambos anteriores), CI do `server.yml` job por job. Push da branch.
4. Cada vez que a `feat/rust-single-owner` andar (Tasks 4, 5, 6), faça merge dela na sua branch,
   resolva e rode de novo. Não junte na `feat/rust-single-owner`: me avise e eu junto depois da 6.

## Frente B — preparar a prova da Task 11 (branch `feat/rust-single-owner-prova`)

Escreva o roteiro executável da Task 11 para ela rodar em minutos quando a Task 6 terminar:
`scripts/prova-dono-unico.sh` (ou `.py`) que sobe o backend desta branch isolado como unit
transiente (`systemd-run --user`, `TimeoutStopSec=10`; `HOME` temporário, portas próprias diferentes
de 8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio por embrulho no `PATH`, `claude`
embrulhado com `--model claude-haiku-4-5` e `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`,
`matar_orfaos` desligado), roda os casos dos Steps 49–55 pela API e confere sozinho o que der
(entregas contadas no transcript cru por marcador único, "religou/desligou" no log, `runtime.*` no
diário, SIGKILL no journal), imprime a tabela de casos e no fim para tudo e apaga o `HOME`.
Valide o roteiro contra a base atual (Tasks 1–3): os casos que dependem das Tasks 4–6 podem falhar;
o que importa agora é o roteiro funcionar de ponta a ponta. Commits em inglês, push da branch, me
avise. Não marque os Steps 49–56 (eles são da execução final).

## Regras das duas frentes

Commits em inglês, `git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`; revisão por subagente
antes do push. Proibido: subir/reiniciar/parar o backend real ou o `hangar-backend`; usar
8765/8766/8768; tocar no tmux padrão ou em sessão real; rodar instaladores; mexer na `main`, na
`hangar-server-parte1` ou na `feat/rust-single-owner`; `push --force`; modelo diferente de Haiku;
log com texto de conversa. Resultados e perguntas para `migracao-rust-2`.
