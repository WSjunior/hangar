# Parte 4: execução (06/10/2026)

Você coordena a execução de `docs/migracao-rust/parte4/plano.md` (aprovado pelo dono) e reporta à
`migracao-rust-3`. Responda ao dono em pt-BR, curto, uma pergunta por vez, opções letradas.

## Branches

- Integração: `feat/parte4`, criada de `origin/hangar-server-parte1` (já tem o plano juntado).
- Uma sessão filha por Task (ou por lote do plano), cada uma numa worktree própria em
  `feat/parte4-tN` de `feat/parte4`. As filhas **não** sobem a própria branch: você junta pela
  branch local (as worktrees dividem o repositório). `feat/parte4` sobe no fim de cada lote.
- `hangar-server-parte1` recebe a `feat/parte4` só nos pontos que o plano permite (nada de
  `Monitor` em produção antes da Task 5; Tasks 8 e 9 juntas). Juntar é da coordenação: avise a
  `migracao-rust-3` com o hash e o CI job por job.

## Ritmo (pedido do dono, 06/10)

- **Juntar primeiro, corrigir depois.** Task com testes focados passando e revisão sem achado
  crítico entra na integração; achados médios viram commit seguinte, sem segurar o lote.
- Não espere CI à toa: siga o próximo lote e confira quando terminar.
- Tasks independentes rodam em paralelo (o plano marca quais).

## Regras que já custaram

- Desempenho: seção "Desempenho: erros que já custaram" do `docs/migracao-rust/README.md`
  (velocidade primeiro; nada de laço apertado; não chamar o Python repetido; não republicar o que
  não mudou; fsync fora de trava; cache e canal com teto). Task que roda a cada tique traz medida
  antes/depois em release.
- Dono único: com o Rust de pé, falha em parte migrada vira erro com código no diário, nunca passa
  ao Python.
- Contrato interno: sobe nas Tasks 1, 5, 8 e 9, sempre o próximo número livre na junção (hoje 27),
  `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos.
- Testes reais: backend isolado (`HOME` temporário, portas fora de 8765/8766/8768, `tmux -L`
  próprio, lançador sem `matar_orfaos`), Claude em Haiku e Codex em `gpt-6-luna`; contas
  permitidas: `/home/jefferson/.claude-02-200` e `/home/jefferson/.claude-jefferson`; nunca
  `claude-200-1` nem `claude-200-3`. Sem fase de sombra no backend do dono.
- Compilação: no máximo 2 `cargo` na máquina (`pgrep -c -x cargo`), `CARGO_BUILD_JOBS=4`,
  `target/` por worktree e apagado ao terminar; plugin `rust-analyzer-lsp` desligado em cada
  worktree filha (`.claude/settings.local.json` com
  `{"enabledPlugins": {"rust-analyzer-lsp@claude-plugins-official": false}}`).
- Windows: prova na VM `delphi-02` (skill `vms-windows`) só na Task 10/13; CI do Windows conferido
  pelo log.
- Sessões filhas na conta padrão, sem `--conta`; não podem parar em cartão de permissão (vigia que
  aprova Yes/No). Feche cada filha ao juntar a Task dela, e o vigia quando não houver filha viva.
- Proibido: backend real, instaladores, workflow do CI, `push --force`, comentar em PR.

## Relatórios

Ao fim de cada lote: Tasks juntadas (hash), medidas, achados e o que ficou, à `migracao-rust-3`.
Pergunta que só o dono responde: direto a ele, uma por vez.
