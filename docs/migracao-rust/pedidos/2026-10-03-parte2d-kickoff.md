# Parte 2D da migração para Rust — planejar e executar

Você vai PLANEJAR e, em seguida, EXECUTAR a etapa 2D: envio de mensagens às sessões **Claude com
terminal** pelo Rust. Ela fecha a parte 2 (sessões do Claude e do Codex, com e sem terminal). O dono
autorizou executar logo depois de planejar, sem parar para aprovação, salvo decisão que mude o
comportamento visto por ele.

## Onde trabalhar

- Pasta `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte2d`, branch
  `hangar-server-parte2d`, criada da `hangar-server-parte1` em `eae31286` (branch do PR #24, já com
  parte 1, 2A, 2B e 2C). Use `git -C <essa pasta>`.
- Em paralelo, a parte 3 (custos e uso) roda noutra máquina, na branch `hangar-server-parte3`, e
  vai usar o contrato interno **versão 8**. A 2D usa a **versão 9** quando mudar o contrato
  (`RUST_SERVER_PROTOCOL` em `backend/app/rust_server.py` e `INTERNAL_PROTOCOL` em
  `crates/hangar-server/src/lib.rs`, juntos). Não toque em custos/uso.

## Leia antes

1. `docs/migracao-rust/README.md` — roteiro, estado e lições.
2. `docs/migracao-rust/parte2-claude-codex/spec.md` e `analise.md`, seção 2D — o desenho aprovado:
   portar a esteira de entrega inteira (socket nativo só recado, plugin com prova de entrega, tmux
   automático só por ausência/resultado seguro, fila/confirmação, opções e teclas), nunca um
   send-keys isolado; INCERTO nunca vira nova digitação.
3. `docs/migracao-rust/parte2b/` e `parte2c/` — como a 2B e a 2C foram planejadas e o que já existe no
   Rust (dono exclusivo da sessão, fila, observação `tmux -C`). Reaproveite; não refaça.
4. `CLAUDE.md` da raiz e as "Regras vigentes" de `docs/decisoes/harnesses.md` e
   `docs/decisoes/windows.md`. A memória `queue-never-retype` resume por que a fila redigitou
   mensagens em loop numa versão antiga: matching cru + `queue-operation`, nunca requeue em
   `working`, confirmação em todo `idle`, dedup integral no front.
5. Código: `backend/app/terminal_input.py`, `tmux.py`, `pqueue.py`, `sse.py`, `api.py` (rotas de
   envio), `rust_server.py`, `internal_api.py`, e `crates/hangar-server`.

## Como trabalhar

1. Escreva `analise.md`, `spec.md` e `plano.md` em `docs/migracao-rust/parte2d/` (plano no formato
   `### Task N:` / `- [ ] **Step N: …**`, testes primeiro, código completo). Pode commitar essa pasta.
2. Execute Task a Task com testes focados de cada Task (autorizados; a suíte inteira só no fim),
   commit por Task com `git add` de caminhos explícitos e mensagem descritiva em inglês, Step marcado
   `[x]` no plano, revisão independente do diff de cada Task antes de seguir.
3. No fim: `cargo test --locked --workspace` em `crates/`,
   `cargo check --locked --target x86_64-pc-windows-gnu -p hangar-server --tests`, pytest dos
   arquivos tocados, revisão final da branch. Verde local → push só da `hangar-server-parte2d` e
   acompanhe o `server.yml` nas três plataformas (`gh run watch`).
4. Ao terminar, ou se travar numa decisão que só o dono toma, mande um resumo curto para a sessão
   `Migracao-Rust` com `hangar-send Migracao-Rust "…"`: commits, testes, link do CI, pendências.

## Regras

- Nunca suba, reinicie ou pare o backend nem o serviço `hangar-backend` (subir outro mata as sessões
  sem terminal); nunca rode instaladores; não mexa na `main` nem na `hangar-server-parte1`.
- Nunca digite em sessão real do usuário nem toque no tmux dele: testes só com `tmux -L <nome>` e
  CLI falsa. Uso real fica para depois, com o dono.
- Log nunca leva texto de conversa nem conteúdo de tela. Identificadores novos em inglês;
  comentários em português, curtos, sobre o porquê. Afirmação técnica com prova (arquivo:linha) ou
  medição.
