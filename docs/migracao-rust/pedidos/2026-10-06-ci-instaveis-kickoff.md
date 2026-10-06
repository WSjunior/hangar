# Dois testes instáveis no CI da `hangar-server-parte1` (06/10/2026)

Você acha a causa e corrige dois testes que falharam no CI do `ccd06c751` e passaram nas subidas
anteriores com o mesmo código. Reporta à `migracao-rust-3` (`hangar-send migracao-rust-3 "..."`).
Use `superpowers:systematic-debugging`: causa provada antes de corrigir; "repetir passa" não é
conserto, e aumentar prazo só com medida que mostre que o prazo era o problema.

Pasta: `/home/jefferson/pessoal/hangar/.claude/worktrees/ci-flaky`, branch `fix/ci-flaky-parte1`
(de `fd556b121`, a `hangar-server-parte1` local com o diagnóstico do item 1). Leia o `CLAUDE.md` da
raiz e `docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` (regras).

## 1. macOS: `crates/hangar-server/tests/terminal_input_tmux.rs` (os 2 testes juntos)

Run Server `37437129489`, job `112181578830` (log: `gh api
repos/jeffer1312/hangar/actions/jobs/112181578830/logs`). Os dois falham no mesmo instante:
`assert!(new.status.success())` no `tmux new-session` (linha 48) e `identity_failed` na primeira
captura (linha 58). Passaram no macOS em `37391307028` e `37402491496`, mesmo tmux 3.7c, mesmo
teste, mesmo `terminal_input`. Hipótese não provada: o `python3` do CLI falso morre ao abrir e
leva a sessão tmux junto. O `fd556b121` passa a mostrar o stderr do CLI falso e do tmux na falha.

## 2. Linux (pytest): `backend/tests/test_claude_headless_cano.py::test_sigterm_encerra_o_filho_e_limpa_o_socket[cano.py]`

Run CI `37437132895`, job `112181593647`. Depois do SIGTERM no `cano.py` (que dá `terminate()` no
filho e `os._exit(0)`), o filho (pid 10431) seguia vivo e não-zumbi depois de 5 s
(`_vivo` já ignora zumbi). 8095 outros passaram. Olhe se o pid que o cano anuncia é mesmo o processo
que recebe o `terminate()` (wrapper/shell/grupo), e a corrida entre `subir` e o handler.

## Como

- Reproduza na máquina sob carga quando der (Linux); o macOS só no CI.
- Para o item 1, suba a branch uma vez com o diagnóstico e repita **só o job do macOS**
  (`gh run rerun <run> --job <job>`) até falhar e mostrar o motivo; não dispare rodadas inteiras à
  toa (cada push em `crates/` roda Server e Native nos três sistemas).
- Conserto com teste que falhava sem ele quando possível; se a causa estiver no código de produção
  (`terminal_input`, `cano.py`, `hangar-cano`), corrija lá, nos dois lados quando houver par
  Python/Rust.
- Compilação: no máximo 2 `cargo` ao mesmo tempo na máquina (`pgrep -c -x cargo`),
  `CARGO_BUILD_JOBS=4`, `target/` desta pasta apagado ao terminar. `rust-analyzer` já está desligado
  aqui.
- Proibido: backend real, instaladores, `push --force`, mexer em workflow do CI, comentar em PR.
- Ao terminar: CI da branch verde job por job, e mande à `migracao-rust-3` a causa de cada um, o
  commit e os ids dos jobs. Quem junta na `hangar-server-parte1` é a coordenação.
