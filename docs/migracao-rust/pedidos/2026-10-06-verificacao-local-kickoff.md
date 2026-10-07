# Verificação local antes do push + auditoria do CI (06/10/2026)

Você faz este trabalho e reporta à `migracao-rust-3` (`hangar-send migracao-rust-3 "..."`).
Responda ao dono em pt-BR, curto, uma pergunta por vez, opções letradas. Leia o `CLAUDE.md` da
raiz, "Regras vigentes" de `docs/decisoes/instalacao.md` e `windows.md`, e a história do revert
`cd1e36ca2` (a "aceleração" que tirou o Windows da release e foi desfeita).

Pasta: worktree própria de `origin/hangar-server-parte1`, branch `feat/local-verification`.

## O problema

Cada tarefa leva ~4 h, ~3 delas esperando CI: sobe, cai, corrige, sobe de novo. Causa medida: a
trava `pre-push` só roda os testes ligados aos arquivos quando consegue perguntar no `/dev/tty`;
numa sessão de agente ela imprime "testes … PULADOS" e o primeiro lugar onde os testes rodam de
verdade é o CI (Windows: 35–40 min por rodada).

## Garantia exigida pelo dono (não negociável)

**Nada do que existe hoje se perde.** Todos os binários continuam subindo (server: linux,
windows, macos; nativo: os três), o `dist-latest` do front (PWA) continua sendo publicado, o
Python continua testado. É otimização, não corte. Entregue uma tabela antes × depois de tudo que
cada workflow publica e testa, conferida contra uma rodada real.

## O que fazer

1. **Auditoria do CI** (`.github/workflows/*.yml`): por job, o que faz, o que publica, tempo médio
   nas últimas rodadas (`gh run view --json jobs`), e se ainda é necessário. O dono pediu tirar o
   que for de Electron: `release.yml` empacota o shell Electron em tag `v*` (última rodada em
   25/08). Remova-o, conferindo antes que nada mais depende dele (instaladores, docs, `atualizar`);
   o desktop é o nativo (`desktop-native/`). Qualquer outro corte proposto: me diga e eu pergunto
   ao dono; não corte por conta.
2. **`scripts/verificar-local`**: olha o que a branch mudou contra a base e roda o que o CI rodaria
   para aquilo (Rust do `server.yml` se `crates/`; `cargo test` do nativo se `desktop-native/`;
   pytest; vitest e `npm run check` se front/core/mobile; os scripts de teste dos wrappers se
   `scripts/shell`). Sem travar a máquina: fila única na máquina (`flock` num arquivo fixo, uma
   verificação por vez entre todas as sessões), `nice`/`ionice`, `CARGO_BUILD_JOBS=4`, e um
   `CARGO_TARGET_DIR` fixo por máquina para compilar só o que mudou (cuidado com a regra antiga:
   target compartilhado entre checkouts rodou binário velho — prove que o teste roda o binário
   desta árvore). Grava o resultado por commit (sha da árvore) num lugar que a trava lê.
3. **Windows na VM `delphi-02`** (`ssh delphi-02`, VPN `wg0`): o mesmo comando por SSH, numa pasta
   fixa de trabalho na VM (não o checkout do Hangar instalado lá, `C:\Users\Administrator\hangar`),
   com target persistente; fila também lá. Rodar quando a mudança toca `crates/`, `backend/` ou
   scripts `.ps1`. Diga o que a VM NÃO reproduz (relógio de 1 ms contra 15 ms do runner).
4. **Trava `pre-push`**: em vez de pular quando não há tty, exige que o `verificar-local` tenha
   passado neste commit (Linux e, quando cabível, Windows na VM); sem isso recusa com a frase de
   como rodar. Atalho de emergência documentado e explícito, nunca automático.
5. Documente a regra no `CLAUDE.md` (seção de Dev commands) e em `docs/decisoes/`.

## Regras

- Medir: tempo de uma verificação local completa e incremental (Linux e VM), carga da máquina
  durante (`uptime`), antes de propor como padrão.
- Proibido: backend real, instaladores rodados de verdade, `--no-verify`, `push --force`, cortar
  job/artefato do CI sem o dono.
- Suba a branch pronta, CI job por job, e me avise; quem junta é a coordenação.
