# Tela "Migração para Rust" + script de medida que acha o backend (06/10/2026)

Você faz este trabalho e reporta à `migracao-rust-3` (`hangar-send migracao-rust-3 "..."`).
Responda ao dono em pt-BR, curto, uma pergunta por vez, opções letradas. Leia o `CLAUDE.md` da
raiz (regras de frontend, i18n, nativo × web), `docs/migracao-rust/README.md` e
`docs/migracao-rust/pedidos/2026-10-05-coordenacao-handoff.md`.

Pasta: worktree própria de `origin/hangar-server-parte1`, branch `feat/migration-status`.

## O que o dono pediu

Uma tela **temporária** (sai quando a migração terminar, na parte 7) para saber, sem terminal,
se o Rust está mesmo respondendo e o que ainda está no Python. Pode ser no "Sobre" ou numa tela
própria ligada a partir dele. Tem que mostrar:

1. **Quem está atendendo a porta 8765 agora:** Rust (`hangar-server`) ou Python, e o modo do
   processo (`pending`/`rust`/`python`). Se for Python, **o motivo** (sem binário,
   `CP_RUST_SERVER=0`, `--reload`, protocolo diferente, 3 quedas em 60 s, endereço privado
   inválido…), com o código que o Supervisor já registra.
2. **Versões:** contrato do Rust (`INTERNAL_PROTOCOL`) e do Python (`RUST_SERVER_PROTOCOL`),
   versão do binário, branch/commit do checkout e canal de testes (`CP_UPDATE_BRANCH`).
3. **Consumo:** memória (RSS) e CPU de cada processo — Python do backend e Rust (e o
   `hangar-cano` das sessões sem terminal, somados à parte).
4. **Quem atende cada parte:** tabela por área (conversa/histórico e eventos, lista de sessões,
   estado das sessões, terminal real, Git/arquivos, worktrees, custos/uso, envio Claude com e sem
   terminal, Codex, Pi/Kimi/omp/orq, contas, convidados, pareamento, MCP, push, atualização,
   uploads, ditado, cotas…) com **Rust / Python**. Não pode ser lista escrita à mão que envelhece:
   derive do que o código faz de fato — a tabela de rotas que o Rust atende sozinho × repassa,
   mais o modo do processo (com o Rust fora, tudo é Python). E, para provar "quem respondeu",
   contadores do Rust por área nos últimos minutos: atendidas pelo Rust × repassadas ao Python.

## Onde

- Backend: um endpoint só de leitura (dono, não convidado) que junta tudo. O Rust atende a parte
  dele e pede ao Python o que só o Python sabe (modo, motivo, RSS do Python) por rota `/internal`
  já existente ou nova — se mexer no contrato interno, próximo número livre, os dois lados juntos.
  Com o Python sozinho na porta, o mesmo endpoint responde pelo Python dizendo isso.
- Telas: **app nativo** (`desktop-native/`, onde o dono usa no PC) e **web/PWA** (celular). O app
  Expo (`mobile/`) fica de fora por ser temporária — diga isso no relatório. Textos por `m.<chave>()`
  com `pt.json` e `en.json` juntos; quatro estados (carregando, vazio, erro, sucesso).
- Atualiza sozinho enquanto aberta (poucos segundos), sem SSE próprio por item.

## Script de medida

`scripts/medir-rust.sh` acha o backend por `pgrep -f "python3 -m app.main"`: falha onde o venv
chama o executável de `python` (o dono viu "backend do Hangar não está rodando" numa máquina Linux
recém-passada para o canal de testes com o Rust de pé). Faça o script (e o `.ps1`) tirar porta do
Python, modo e token do jeito mais robusto — de preferência do endpoint novo — e dizer com clareza
qual das coisas faltou (backend fora, Rust fora com o Python na porta, binário ausente, etc.).

## Regras

- Desempenho: o endpoint é barato (nada de varrer a máquina por pedido; RSS lido do `/proc` ou da
  API do sistema dos pids conhecidos).
- Teste real: backend isolado (`HOME` temporário, portas fora de 8765/8766/8768, `tmux -L`
  próprio); nos dois modos (Rust de pé e `CP_RUST_SERVER=0`). Tela conferida no nativo e na web
  (PWA na largura de celular), com print citado por caminho absoluto.
- Compilação: no máximo 2 `cargo` na máquina, `CARGO_BUILD_JOBS=4`, `target/` apagado ao terminar,
  `rust-analyzer` desligado na worktree.
- Não suba a branch a cada commit: suba pronta, avise a `migracao-rust-3` com hash e CI job por
  job; quem junta na `hangar-server-parte1` é a coordenação.
- Proibido: backend real, instaladores, workflow do CI, `push --force`, comentar em PR.
