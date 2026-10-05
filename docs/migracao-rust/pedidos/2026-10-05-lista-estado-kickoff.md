# Medir e planejar: lista de sessões + estado no Rust

Pasta: este cwd (branch `plan/session-list-state`, de `origin/hangar-server-parte1` `920473435`).
Leia `docs/migracao-rust/README.md` (regra "O Rust é o único dono do que já migrou"),
`docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md`, o `CLAUDE.md` da raiz (seções do
quadro/canvas/lista, `difusor.py`, `sessionsStore`) e as "Regras vigentes" de
`docs/decisoes/harnesses.md`, `plataforma.md` e `windows.md`.

## Por que esta parte

O dono pediu a parte de maior ganho. A lista de sessões com o estado de cada uma roda o tempo todo
no Python: a cada ~1,5 s percorre todas as sessões (tela do tmux, transcript, sidecar, `git
status`), classifica `working`/`idle`/`awaiting_input` e difunde para todos os clientes pelo
stream agregado (`/api/sessions/events`, `difusor.py`, `registry.py`, `state.py`,
`sessionListModel`). Hoje, com 1 sessão aberta, o Python usa ~5% de um núcleo em repouso e o Rust
~0,5% (medido 20 s). O ganho esperado aparece com muitas sessões (GIL × leitura paralela no Rust).

## Fase 1 — medir (sem mexer em código)

Backend de teste isolado desta branch (`HOME` temporário, portas fora de 8765/8766/8768 e de outras
sessões, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio; sessões Claude só Haiku,
`CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`; nunca `claude-200-1`/`claude-200-3`). Com 1, 10
e 20 sessões (mistura com e sem terminal, algumas trabalhando): CPU do Python e do Rust em repouso e
com sessões ativas, tempo de cada rodada do laço da lista (instrumente por fora, sem commitar: perfil
com `py-spy` ou temporizador), latência até um cliente ver a mudança de estado, quanto é tmux/
transcript/git/classificação. Pare tudo e apague o `HOME` no fim.

## Fase 2 — inventário e plano (sem código)

Em `docs/migracao-rust/lista-estado/`: `medicao.md` (números e metodologia), `inventario.md`
(tudo que monta a lista e o estado hoje, com arquivo:linha: `registry.py`, `state.py`, `difusor.py`,
`sse.py`, `preview.py`, hooks de estado, plugin, o que a 2C/dono único já leva ao Rust, Codex/Pi/Kimi/
omp/orq, Windows), `desenho.md` (o que vai para o Rust, o que fica; Rust como único dono do que
migrar; a lista continua um stream por servidor, nunca um por card; quatro estados nas telas) e
`plano.md` no formato do repositório (`### Task N:` / `- [ ] **Step N: …**`, ver
`docs/agents/issue-tracker.md`), com Tasks pequenas, teste que falha sem cada uma, paralelismo
marcado e prova de uso real. Perguntas ao dono, se houver, fechadas e poucas.

Se a medição mostrar ganho pequeno, diga isso com os números e proponha outra parte em vez de
planejar esta.

## Regras

Compilação: no máximo 2 `cargo` na máquina, `CARGO_BUILD_JOBS=4`, apague o `target/` no fim.
Commits só dos documentos (inglês, `git add` explícito, `HANGAR_SEM_PASSO=1`), push desta branch.
Mande a `migracao-rust-2`: medição resumida, recomendação e o caminho do plano. Proibido:
reiniciar/parar o backend real, usar 8765/8766/8768, tocar no tmux padrão ou em sessão real,
instaladores, `push --force`, texto de conversa em log.
