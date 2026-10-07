# Parte 4 da migração: planejamento (06/10/2026)

Você planeja a parte 4 da migração para Rust e reporta à coordenação `migracao-rust-3`
(`hangar-send migracao-rust-3 "..."`). Responda ao dono em pt-BR, curto, uma pergunta por vez,
opções letradas (A/B/C, recomendação marcada). Nada de código nesta etapa: o resultado é análise,
desenho e plano aprovados pelo dono.

## Ler antes

- `CLAUDE.md` da raiz (regras do projeto; "Regras vigentes" de `docs/decisoes/harnesses.md` e
  `windows.md` quando o desenho tocar terminal, estado ou psmux).
- `docs/migracao-rust/README.md` inteiro, em especial "Desempenho: erros que já custaram".
- `docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md` (estado, pendências, regras).
- `docs/migracao-rust/lista-estado/` (desenho, plano e provas da lista + estado no Rust): a **Fase C
  (Tasks 19–24)** desse plano ficou guardada para entrar junto da parte 4 (decisão do dono).
- O que já existe no Rust do terminal: `crates/hangar-server/src/terminal_*`, `terminal_control.rs`
  (tmux `-C`), `terminal_state.rs`, a observação terminal (CLAUDE.md, "Observação terminal Rust")
  e os documentos `parte2c/` e `parte2d/`.

## Escopo da parte 4 (roteiro)

"Estado e prévia (tmux em modo controle, cópia da tela por sessão) + terminal real
(`portable-pty`)":
1. Estado ao vivo das sessões com terminal (trabalhando/parada/esperando), prévia, pergunta
   nativa, sugestão, aviso de entrega, problema — saindo do Python (`state.py`, `StateMonitor` em
   `sse.py`, `preview.py`, `askquestion.py`) para o Rust, incluindo a Fase C da lista.
2. Terminal real do app (PTY por WebSocket no rodapé e no celular; hoje `termsock`/Python).
3. Windows: captura avulsa pelo psmux (Task 22), `-C` segue desligado lá.
4. Provas que faltaram na lista: cartão de permissão do Claude com terminal; Codex com cartão
   (não só YOLO).

## Como

- Inventário primeiro: cada coisa que o Python faz hoje nesse caminho (arquivo:linha), quem
  consome, e o que o Rust já tem. Parta de como o Python resolve (as regras em `harnesses.md`
  custaram bugs); nada escrito do zero sem esse ponto de partida.
- Desenho com o dono único (Rust de pé → falha vira erro com código, nunca passa ao Python) e
  com a regra de velocidade (velocidade primeiro; memória só quando não custa tempo; nada de laço
  apertado; não chamar o Python repetido; não republicar o que não mudou).
- Plano no formato `### Task N:` / `- [ ] **Step N: …**` (lido por `planprog.py`), com Tasks
  independentes marcadas para rodar em paralelo, teste que falha sem cada Task e prova de uso
  real com sessões reais em backend isolado (Claude em Haiku, Codex em `gpt-6-luna`, com e sem
  terminal; nunca `claude-200-1` nem `claude-200-3`). Sem fase de "sombra" no backend do dono.
- Medida antes/depois em release por Task que rode a cada tique (`scripts/medir-rust.sh` e
  `scripts/medir-lista-hub.py` como base).
- Pasta dos documentos: `docs/migracao-rust/parte4/` nesta branch (`hangar-server-parte1`); faça
  numa worktree própria de `origin/hangar-server-parte1` (branch `plan/parte4`) e suba a branch;
  quem junta na `hangar-server-parte1` é a coordenação.
- Para explorar o código use subagentes (Explore) e não leia tudo no contexto principal.
- Compilação, se precisar: no máximo 2 `cargo` na máquina (`pgrep -c -x cargo`),
  `CARGO_BUILD_JOBS=4`, `rust-analyzer` desligado na worktree.
- Proibido: backend real, instaladores, mexer em workflow do CI, `push --force`, comentar em PR.

## Entrega

1. Inventário + desenho → mande o resumo ao dono (pergunta por vez) e à coordenação.
2. Plano aprovado pelo dono → avise a coordenação com o caminho e o hash. A execução é aberta
   pela coordenação.
