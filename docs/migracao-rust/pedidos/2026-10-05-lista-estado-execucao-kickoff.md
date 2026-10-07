# Executar o plano da lista + estado — Fases 0 a B (Tasks 1 a 18)

Plano aprovado pelo dono em 05/10/2026, opção **A**: Fases 0–B agora (Tasks 1–18); Fase C (19–22)
fica para junto da parte 4. Plano: `docs/migracao-rust/lista-estado/plano.md` (com `desenho.md`,
`inventario.md`, `medicao.md`). Branch de integração: `feat/session-list-state` (este cwd), criada do
plano + `origin/hangar-server-parte1` (`24cef8075`).

Você **coordena**: abre sessões de trabalho para as Tasks conforme a seção "Lotes" do plano
(paralelo onde o plano marca; uma worktree e uma branch por sessão, a partir da integração), confere
cada entrega e junta na `feat/session-list-state` na ordem dos números. Use `hangar-send --new
<nome> <cwd>` **sem** `--conta` (conta padrão) e confira a conta na resposta.

## Regras

- Leia antes `docs/migracao-rust/README.md` ("O Rust é o único dono do que já migrou"),
  `docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` e o `CLAUDE.md` da raiz.
- Cada Task: teste que falha sem ela, revisão independente por subagente da stack +
  `ecc:silent-failure-hunter`, regra de `CLAUDE.md`/`docs/` que a Task contradiz corrigida no mesmo
  commit, Steps marcados `- [x]` no `plano.md` (o planprog lê daí).
- **Compilação: no máximo 2 `cargo` ao mesmo tempo na máquina, `CARGO_BUILD_JOBS=4`**; apague o
  `target/` de cada worktree ao terminar. Disco e carga já derrubaram o backend real hoje.
- CI do GitHub: conferir job por job; o GitHub está com falta de máquinas hoje (jobs cancelados com
  "not acquired by Runner"); repita os cancelados, não espere à toa.
- Contrato interno: o plano usa 23 (Task 14); suba `RUST_SERVER_PROTOCOL` e `INTERNAL_PROTOCOL` juntos.
- Task 15 (sombra) é a única que vai ao canal de testes nesta fase: avise `migracao-rust-2` antes de
  juntar na `hangar-server-parte1`, porque o dono testa por ali. As Tasks 16–18 trocam o dono da
  lista; só juntam na `hangar-server-parte1` depois de 2 dias de sombra sem divergência, com o dono
  avisado.
- Testes reais: backend isolado (`HOME` temporário, portas fora de 8765/8766/8768, `tmux -L` próprio,
  sessões só Haiku, `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`; nunca `claude-200-1`/`-3`).
- Sessões filhas não podem parar em cartão de permissão: vigia que responde `select` opção 1.
- Proibido: reiniciar/parar o backend real, portas 8765/8766/8768, tmux padrão, sessão real,
  instaladores, `push --force`, mexer na `main`.
- Reporte a `migracao-rust-2` ao fim de cada fase (hashes, testes, CI) e quando travar numa decisão
  do dono; feche as sessões de trabalho ao juntar.
