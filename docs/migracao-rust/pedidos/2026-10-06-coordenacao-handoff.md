# Coordenação da migração para Rust — passagem de 06/10/2026 (noite)

De `migracao-rust-3` para a próxima sessão coordenadora. Leia este arquivo, depois
`docs/migracao-rust/README.md` (inclusive "Desempenho: erros que já custaram") e o `CLAUDE.md` da
raiz. O handoff anterior (`2026-10-05-coordenacao-handoff.md`) segue valendo para pendências e
regras; os itens 16–19 foram acrescentados hoje. Responda ao dono (Jefferson) em pt-BR, curto, uma
pergunta por vez, opções letradas; ele lê pelo celular e não quer texto longo.

Pasta de coordenação: `/home/jefferson/pessoal/hangar/.claude/worktrees/hangar-server-parte1`
(branch `hangar-server-parte1`, PR #24 → `main`). A cópia local pode ter commits ainda não subidos
(`git log origin/hangar-server-parte1..HEAD`): sobem juntos no próximo push.

## Próximo passo combinado com o dono (decisão dele)

1. Esperar a sessão `ci-estavel` (branch `fix/ci-timing-tests`) entregar: testes de tempo do
   Windows estabilizados, Windows verde 3× seguidas. Juntar na `hangar-server-parte1` e subir junto
   com os commits locais.
2. Conferir o CI dessa subida **job por job** (CI, Native, Server nos 3 sistemas + publish-check).
3. Com tudo verde, **juntar o PR #24 na `main` com acesso de administrador** (`gh pr merge 24
   --merge --admin`): o dono autorizou explicitamente, sem esperar aprovação e sem período de uso.
4. Daí em diante a migração segue na `main`: branches novas a partir dela e PR para ela; a
   `hangar-server-parte1` é encerrada (canal de testes das máquinas volta para `main`: avisar o
   dono para esvaziar `CP_UPDATE_BRANCH` nas máquinas, ou pedir).
5. A `verificacao-local` não segura a ida para a `main`: se terminar antes, entra na parte1; se
   depois, PR próprio para a `main`.

## Estado (06/10, ~20h)

- Contrato interno **36** (`RUST_SERVER_PROTOCOL`/`INTERNAL_PROTOCOL`). Release
  `server-hangar-server-parte1` com binário dos 3 sistemas no 36 (commit `0a4233d2d`).
- Entraram hoje: lista + estado no Rust (Fases 0–B), parte 4 inteira (estado, prévia, terminal real
  no Rust, inclusive Windows/ConPTY; contrato 35), tela temporária "Migração para Rust" (Sobre →
  Configurações; nativo e web), #84/#85 (`/clear` e painel de agentes), #86 (sessão criada herda
  conta/modo/sem terminal; conta ≥95% vai para a de mais folga), #67, #68, #72, #73, #81, #82, #88,
  #89/#91 na main, #90 (mods do Hangar por `--plugin-dir`, `plugins/hangar` primeiro), #92, #93,
  mod `plugins/github-actions`, conserto do cartão de seletor no nativo, Atualizar que para no
  último commit com binário publicado para o sistema (com aviso, nativo e web) e publicação por
  sistema no `server.yml` (cada sistema publica ao terminar; manifesto `platforms.<plat>.{commit,
  protocol}`).
- Máquinas: PC (esta) no 36 com Rust; notebook no 36; VM `delphi-02` no 35 com binário compilado
  nela em `crates/target/release` (apagar e Atualizar quando a release Windows do 36 estiver lá —
  já está); Waldir (Linux) e Rafael/Viana (Windows) precisam clicar em Atualizar.
- Esta máquina alcança a VM: `ssh delphi-02` (VPN `nmcli connection up wg0`), peer `delphi-02` no
  `backend/peers.json`, senha do RDP em `~/.config/pmedico/delphi-02.pw` (RDP toma a sessão
  gráfica do dono: avisar antes). C: da VM tem ~15 GB livres: compilação Rust lá enche o disco.

## Sessões vivas

| Sessão | O que faz | Branch |
|---|---|---|
| `ci-estavel` | Testes de tempo do Windows (costs, mods, foco, `node` do teste do `/clear`, marcadores da lista — este era defeito real) | `fix/ci-timing-tests` |
| `verificacao-local` | `scripts/verificar-local` (fila única, nice, target fixo), Windows na VM, trava `pre-push` exigindo a verificação, auditoria do CI (tirar `release.yml` do Electron; tabela antes × depois; garantia do dono: nada se perde), avaliar passos paralelos do Actions | `feat/local-verification` |
| `github-actions-mod` | Sessão do dono (mod da faixa do CI); pronto e juntado localmente | `feat/github-actions-mod` |

Diga às sessões que passem a reportar à nova coordenadora (`hangar-send <sessao> "..."`).

## Regras aprendidas hoje (memórias do projeto já gravadas)

- **Juntar primeiro, corrigir depois** nos PRs para a branch da migração (≠ `main`).
- **Push na parte1 é raro**: cada push roda CI e faz o Atualizar de quem está no canal de testes
  puxar; doc/script ficam locais até um push necessário. Commit/mensagem com a palavra de push no
  texto aciona a trava de revisão; repetir o mesmo push passa.
- **Atualizar a máquina do dono** depois de junção relevante, quando o binário Linux publicar
  (`POST /api/atualizacao/iniciar` com o token de `/home/jefferson/hangar/backend/.env`). Só a
  coordenação dispara o Atualizar desta máquina.
- **Sem sombra no backend do dono**; validação = sessões reais (Haiku + `gpt-6-luna`) em backend
  isolado; nunca `claude-200-1`/`claude-200-3` (use `~/.claude-02-200`, `~/.claude-jefferson`).
- **Desempenho primeiro**, memória só quando não custa tempo.
- **Nunca `--no-verify`.** A trava de repo público recusa chamado real (`PM-<número>`) no código.
- CI/instalação: só o pedido literal do dono; mudar o que é publicado exige pergunta a ele.
- Issues do GitHub: o dono indica quais pegar (as do WSjunior atribuídas a ele ficam com ele); ao
  juntar a correção, fechar a issue (`gh issue close N --reason completed`, sem comentário).
- No máximo 2 `cargo` na máquina, `CARGO_BUILD_JOBS=4`, `rust-analyzer` desligado nas worktrees
  filhas, `target/` apagado ao terminar.

## Pendências novas (fora as 1–19 do handoff de 05/10)

- Parte 4: `docs/migracao-rust/parte4/achados-pendentes.md` (Codex `/permissions` 0.159.3 e menu
  de aprovação com terminal → parte 5; Task 13 com o dono; médios por Task).
- Tela de migração: branch vazia no 1º minuto do boot; 503 do Rust contado como "respondido".
- PR #89: três pontos baixos não corrigidos (Esc da lista na web, `selectionchange` no PWA, Esc com
  dois `/x`).
- Defeitos do Codex achados na prova da parte 4 (2 dos 3 ainda abertos: `/permissions` do 0.159.3,
  menu de aprovação com terminal fica `working`).
- `ci-estavel` e `verificacao-local` em curso (acima).
