# Prova da troca: a lista do dono servida pelo Rust, com sessões reais

Fecha a Fase B (Tasks 16–18) antes de a integração ir à `hangar-server-parte1`. Siga
`2026-10-05-lista-estado-task-kickoff.md` (sem push, `cargo-slot`, sem LSP). Branch
`feat/session-list-state-prova`, a partir da integração.

## Montagem

Backend isolado como em `scripts/prova-lista-sombra.py` (que é a base): `HOME` temporário, portas
fora de 8765/8766/8768, `tmux -L` próprio, `matar_orfaos` e poda desligados, binários release desta
branch, `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`, segunda conta só `~/.claude-jefferson`
(nunca `claude-200-1`/`-3`). Claude em Haiku, Codex em `gpt-6-luna`, com e sem terminal. **Sem**
`CP_LIST_SHADOW`: agora quem serve a lista do dono é o `ListHub`.

## O que provar

1. **Os 12 cenários da sombra** (nascer, trabalhando → parada, permissão, `AskUserQuestion`,
   `/clear`, matar, recriar com o mesmo nome, par e grupo, worktree com branch, plano com pino,
   troca de conta, restart do backend com sessão viva), agora lendo `GET /api/sessions` e o SSE
   `/api/sessions/events` do dono. Para cada um, o estado e os campos que o cenário mexe batem com o
   esperado (escrito no roteiro antes de rodar), e as sessões sem terminal mostram o estado do
   runtime, sem `list_runtime_absent`.
2. **Zero descoberta Python no modo `rust`:** o contador `registry.PYTHON_DISCOVERY` (Task 16) fica
   em 0 durante tudo; diga como foi lido.
3. **Reserva:** com `CP_RUST_SERVER=0` a lista volta pelo Python igual (mesmos cenários curtos:
   nascer, parada, matar); com o Rust derrubado 3 vezes em 60 s o Python assume e a lista segue.
4. **Tempo e custo, em release:** marcador → lista (SSE), `GET` (mediana/máx), CPU do Python e do
   Rust parados e com 1 lista aberta, pico de RSS do Rust. Use `scripts/medir-lista-hub.py` quando
   couber.
5. **Convidado:** a lista do convidado continua pelo Python e não mostra sessão escondida do dono.

Defeito achado: corrija na sua branch com teste que falha sem a correção. Relatório em
`docs/migracao-rust/lista-estado/prova-troca.md` (cenário, esperado, visto, tempo) e resumo a
`lista-org`. No fim, pare tudo e apague `HOME` e `target/`.
