# App nativo seguindo o canal de testes (release por branch)

## Problema

O canal de testes (`CP_UPDATE_BRANCH=<branch>` no `backend/.env`, tela "Canal de testes") troca o
backend Python, o binário Rust (release `server-<branch>`, `.github/workflows/server.yml`) e a tela
web (compilada na máquina), mas **não o app nativo** (`desktop-native/`): ele vem só da release fixa
`native-latest`, gerada a cada push na `main` (`.github/workflows/native.yml`), e o "Atualizar" do
app lê `native-latest.json` (`desktop-native/src/update.rs`). Resultado: o dono, na branch
`hangar-server-parte1`, não vê no nativo o que entrou nela (ex.: PR #50, pele Terminal).
O `workflow_dispatch` atual do `native.yml` reescreveria a `native-latest` de todos: não usar.

## O que fazer

1. Leia `native.yml`, `server.yml` (como ele publica `server-<branch>` e como o download escolhe a
   release pela branch do checkout), `desktop-native/src/update.rs`, `scripts/install-native.sh` /
   `.ps1`, `backend/app/atualizar.py`, `backend/app/rust_release.py` e as "Regras vigentes" de
   `docs/decisoes/instalacao.md` e `windows.md`.
2. `native.yml` passa a compilar também em push na `hangar-server-parte1` (e em qualquer branch de
   canal que o `server.yml` já cubra, mesmo critério), publicando numa release própria
   `native-<branch>` com o mesmo manifesto; a `native-latest` continua só da `main`.
3. O app nativo segue o canal: quando o backend ao qual ele está ligado está num canal de testes
   (exposto pela API que a tela "Canal de testes" já usa), o "Atualizar" do nativo lê a release da
   branch; sem canal, `native-latest` como hoje. Desligar o canal volta para a `main` no próximo
   Atualizar. Se fizer sentido, o Atualizar do backend no canal também instala o nativo da branch
   (como já faz com o binário Rust) — escolha o caminho mais simples que cubra os dois e explique.
4. Linux, Windows e macOS; release ausente para a branch → mantém o nativo atual e avisa (nunca
   instala o da `main` calado nem quebra).
5. Testes: os do `update.rs` e dos scripts/`atualizar.py` tocados; `cargo test --locked` em
   `desktop-native/`; workflow validado (rode o `native.yml` na branch e confira que publica
   `native-<branch>` sem tocar a `native-latest`). Prova: neste PC, o nativo atualiza para a versão da
   `hangar-server-parte1` e mostra a pele Terminal.

## Entrega

Branch `feat/native-test-channel` (este cwd, de `origin/hangar-server-parte1`). Commits em inglês,
`git add` explícito, `HANGAR_SEM_PASSO=1` (se mudar o que a atualização faz na máquina, siga a regra
dos passos de `docs/atualizacoes/`). Revisão por subagente (`ecc:rust-reviewer`,
`ecc:python-reviewer`, `ecc:silent-failure-hunter`). Compilação: no máximo 2 `cargo` na máquina,
`CARGO_BUILD_JOBS=4`, apague o `target/` ao terminar. Push só desta branch; avise `migracao-rust-2`
com o que mudou, testes e CI; eu junto. Proibido: reiniciar/parar o backend real, rodar
instaladores sem ser na prova combinada, `push --force`, reescrever a `native-latest`.
