# Planejar: o Rust como único dono do que já migrou

## A decisão (do dono, 04/10/2026)

Fica só a reserva do processo inteiro: se o `hangar-server` não sobe, é incompatível (protocolo) ou
cai (3 quedas em 60 s), o Python assume a porta 8765 e atende tudo — isso continua. Com o Rust de
pé, o que já foi migrado é só dele: falha numa operação vira erro com código e motivo (diário e
`hangar-server.log`), nunca a passagem daquela sessão ou operação ao Python. O código Python dessas
partes fica só de referência e sai na parte 7. Leia "Como a migração funciona" em
`docs/migracao-rust/README.md`.

Motivo: a passagem entre os dois donos com o Rust vivo foi a origem da maioria dos defeitos de
04/10 — 1ª mensagem sumindo na sessão nova (adoção pelo Rust depois de o Python subir o cano),
"sessão em transferência" derrubando o chat, entrega marcada sem chegar no `quiesce`, reserva
circular do Git no PR #30 (`docs/migracao-rust/parte2d/corrida-nascimento.md`,
`docs/migracao-rust/parada-e-posse.md`, `docs/migracao-rust/git-arquivos/revisao-real.md`).

## O que entregar agora: análise + plano, sem mexer em código

Escreva em `docs/migracao-rust/dono-unico/`:

1. **Inventário** de todo mecanismo de passagem com o Rust vivo, com arquivo:linha: na 2B/2D
   (`runtime_coordinator.py` — fases `PreparingRust`/`RecoveringPython`, `_hand_to_python`,
   `failure_reason`, 3+1, `Fallback` por sessão; `runtime_adapter.py` — `LegacyBridge`, `quiesce`,
   adoção, `owner_state_stream`; `runtime_policy.py`, `runtime_queue.py`, `runtime_terminal.py`; o
   lado Rust em `crates/hangar-server/src/runtime/`), no PR #30 (`workspace_bridge.py` `delegate`,
   `take_over`, `x-hangar-workspace-fallback`, contagem de `workspace_unavailable` por sessão;
   `workspace_routes.rs`), na 2C (observador do terminal), e o caminho de nascimento (quem sobe o
   cano da sessão nova hoje e quem adota). Para cada um: o que faz, se some, fica ou muda.
2. **O que fica** e por quê: a reserva do processo inteiro (Supervisor, `lifespan="off"`, 3 quedas
   em 60 s) e o que ela precisa para retomar as sessões quando o Rust cai de verdade; a separação
   por plataforma (Windows com a 2C desligada, Python observa o terminal ali) — isso não é passagem
   com o Rust vivo; Pi/Kimi/omp/orq, que ainda não migraram.
3. **Desenho do novo**: sessão nova nasce já no Rust quando ele está de pé (sem adoção depois);
   falha de operação → erro visível + registro, e a sessão continua no Rust; o que o usuário vê em
   cada falha (mensagem na tela, estado do cartão); como fica o restart do backend com sessões sem
   terminal vivas; contrato interno (próximo número livre: 14).
4. **Plano** no formato do repositório (`### Task N:` e `- [ ] **Step N: …**`, ver
   `docs/agents/issue-tracker.md`), com Tasks pequenas, cada uma com teste que falha sem ela e o
   código morto removido no mesmo passo, e a prova de uso real (backend isolado, sessões Haiku,
   `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`): criar sessão e mandar na hora, fila com o
   Claude ocupado, `/clear`, restart com fila, queda do Rust (o Python assume tudo sem duplicar),
   falha forçada de operação (erro visível, sessão segue no Rust).
5. Riscos e o que não dá para fazer sem decisão do dono — liste como perguntas fechadas, poucas.

Use subagentes de exploração para o inventário (é amplo). Não suba backend nem rode testes reais
nesta fase. Commits só dos documentos (inglês, `git add` explícito, `HANGAR_SEM_PASSO=1`) na branch
`plan/rust-single-owner` deste cwd; push dela. Mande a `migracao-rust-2` o caminho do plano, o
resumo do que some/fica e as perguntas. A execução começa só depois da aprovação do dono.

## Proibido

Mexer em código; subir/reiniciar/parar backend; tocar em sessão real; mexer na `main` ou na
`hangar-server-parte1`.
