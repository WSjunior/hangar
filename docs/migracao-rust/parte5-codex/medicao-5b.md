# Parte 5B: medição do Codex sem terminal no Rust

Release, máquina do dono (i5-13400F, CachyOS), backend isolado, Codex de mentira. 08/10/2026.

**Com o Rust dono do Codex sem terminal, a 5B gasta mais CPU, não menos: com 10 sessões
trabalhando, Python + Rust sobem de 62,5–65,5 ms/s (base, Python dono) para 91–92 ms/s, e o total
com canos e `codex` de 88–91 para 115–118.** O Python não sai do caminho: ele mesmo sobe de 53–56
para 74–74,5 ms/s, e o Rust de 9,5 para 17–17,5. Parado, nada muda (~25 ms/s). O pico de memória do
Rust sobe de 25 para 61 MB com 10 sessões (~3,6 MB por sessão); o do Python fica igual.

| Sessões | Modo | Medida | Python sozinho (`CP_RUST_SERVER=0`) | Base `6186ce136` (Python dono, Rust na frente) | 5B (Rust dono) |
|---:|---|---|---:|---:|---:|
| 10 | parado | Python | 22,0 / 21,0 | 22,0 / 22,0 | 19,5 / 22,0 |
| 10 | parado | Rust | — | 3,5 / 3,5 | 3,5 / 3,5 |
| 10 | parado | **Total** | **22,0 / 21,0** | **25,5 / 25,5** | **23,0 / 25,5** |
| 10 | trabalhando | Python | 54,0 / 52,0 | 56,0 / 53,0 | 74,0 / 74,5 |
| 10 | trabalhando | Rust | — | 9,5 / 9,5 | 17,0 / 17,5 |
| 10 | trabalhando | 10 `hangar-cano` | 9,5 / 10,5 | 9,0 / 9,0 | 9,0 / 10,0 |
| 10 | trabalhando | 10 `codex` de mentira | 16,5 / 16,5 | 16,5 / 16,5 | 19,5 / 16,5 |
| 10 | trabalhando | **Total** | **78,0 / 80,0** | **91,0 / 88,0** | **115,0 / 118,0** |
| 5 | parado | Python / Rust | 14,5 / — | 15,5 / 3,5 | 15,5 / 3,5 |
| 5 | trabalhando | Python / Rust | 33,0 / — | 34,5 / 6,5 | 43,0 / 11,5 |
| 5 | trabalhando | **Total** | **48,0** | **54,5** | **69,5** |

CPU em ms por segundo; cada número é a mediana de 3 janelas de uma rodada, e `/` separa as duas
rodadas de 10 sessões (5 sessões: uma rodada). Canos e `codex` parados ficam em 0.

| Pico de RSS (VmHWM) | Python sozinho | Base | 5B |
|---|---:|---:|---:|
| Python, 10 sessões | 161,3 / 161,4 MB | 174,6 / 163,1 MB | 164,0 / 163,7 MB |
| Rust, 10 sessões | — | 25,2 / 24,9 MB | 61,6 / 60,8 MB |
| Rust, 5 sessões | — | 23,2 MB | 40,6 MB |
| 10 `hangar-cano` (soma) | 34,2 / 33,8 MB | 34,4 / 34,5 MB | 34,3 / 34,4 MB |
| 10 `codex` de mentira (soma) | 144,7 / 144,5 MB | 144,4 / 144,5 MB | 144,5 / 144,6 MB |

- Um cano e um `codex` por sessão em todas as rodadas (conferido pela chave `HANGAR_CANO_KEY`
  de cada cano), os mesmos do começo ao fim da janela, e zero canos depois de apagar as sessões.
- A prévia chega igual nos três: 6,65 eventos `preview` por segundo em cada chat.
- O Python trabalhando sobe de uma janela para a seguinte nos três casos (5B: 67 → 74 → 80,5 ms/s
  numa rodada), junto com o texto da resposta em voo. Causa provável, não medida: o canal privado
  `/runtime/events` (`gateway.rs`, `events`) manda ao Python todo evento do motor, sem filtro de
  canal, inclusive cada `preview` com o texto inteiro; o Python decodifica cada um.

## Achado: a primeira conversa no Rust entra em laço

**Sem remendo, a conversa nova de Codex sem terminal no Rust quase nunca abre: o motor repete
`initialize` → `thread/start` ~8 vezes por segundo, sem aviso no log nem `problema` na sessão.**
Em 10 tentativas seguidas da mesma sessão nenhuma abriu; com o remendo abaixo, 5 de 10 sessões
(e 6 de 10, 2 de 5) bateram no caso. No Python sozinho e na base, nenhuma.

O que se vê: o motor publica a thread nova e, em paralelo, pede `session.patch_meta {thread_id,
rollout_path}` ao Python. O Python compara com a vista salva, que ainda não tem a thread, e responde
`stale` (`runtime_policy.py`, `session.patch_meta`). O sidecar fica com `thread_id: null`; quando a
vista chega com a conversa, o coordenador vê conversa diferente do sidecar e religa o motor
(`runtime_coordinator.py`, `_rebind`), que abre outra `thread/start` com o sidecar ainda vazio. O
`actor.rs` já salva a vista antes da política para `service_tier` e `permission_mode`; `thread_id`
não está nessa lista. Não conferido com Codex real.

Remendo só desta medida, no lançador do backend isolado: `runtime_policy.run` repete o
`patch_meta` com `thread_id` a cada 0,2 s enquanto ele voltar `stale` (até 10 vezes). Ele roda uma vez
por sessão, na criação, fora das janelas medidas.

## Como

`scripts/medir-codex-sem-terminal.py`, backend isolado pela classe `Prova` de
`prova-dono-unico.py`: HOME, portas, token e `tmux -L` próprios, unit transiente do systemd,
`matar_orfaos` trocado por no-op antes do lifespan, portas de convite e Connect trocadas,
`CP_RUST_NO_ORPHAN_SWEEP=1` para a varredura de canos do `hangar-server`. Binários `hangar-server` e
`hangar-cano` em release. "Python sozinho" = esta branch com `CP_RUST_SERVER=0`; "base" = `git
archive` de `6186ce136` (a junção da parte 5-0, antes da 5B), com o próprio backend, binários release
compilados dele e o roteiro copiado para dentro; "5B" = esta branch com o Rust de pé.

Sem Codex real e sem credencial: o `codex` do PATH isolado é um Python que responde
`--version` (0.161.0, a conferida) e fala JSON-RPC por stdio (`initialize`, `thread/start`,
`thread/resume`, `turn/start`, `turn/interrupt`; o resto recebe `{}`). Com turno aberto ele manda
um `item/agentMessage/delta` de 8 caracteres a cada 50 ms até o arquivo `parar` aparecer, e grava
no rollout a fala do usuário e a resposta final. Sessões criadas uma de cada vez por
`POST /api/sessions` (`provider: codex`, `headless: true`); o chat de cada uma
(`/api/sessions/<nome>/events` do dono) abre depois que o rollout existe. Trabalhando = um
`/input` por sessão e todas em `working` pela lista.

CPU = `utime+stime+cutime+cstime` de `/proc/<pid>/stat`, por processo e somada por grupo (Python,
Rust, canos, `codex`), em 3 janelas de 20 s depois de 8 s assentando. Pico de RSS = `VmHWM` de
`/proc/<pid>/status` no fim da rodada trabalhando. Antes e depois de cada rodada, a lista de
`hangar-cano` fora do HOME isolado (os canos reais do dono, pids 723830 e 2284740) foi conferida e
ficou idêntica em todas as rodadas.

Não medido aqui: Codex real (o custo do próprio `codex` e o ritmo de deltas reais), latência de
entrega, aprovações e perguntas, e Windows. A causa do aumento do Python é hipótese; medir pede um
perfil do Python (sem `py-spy`/`perf` na máquina, e `ptrace_scope=1`).
