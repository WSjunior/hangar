# Parte 4: medições

Release, máquina do dono (i5-13400F, CachyOS), sem backend nem sessão real.

## Task 2: uma rodada do `Monitor` (captura + `reduce`), 06/10/2026

| Medida | Por rodada |
|---|---:|
| CPU do processo Rust (captura pelo `TerminalPool` + análise + `reduce`) | 0,077–0,080 ms |
| CPU do servidor tmux | 0,107–0,113 ms |
| CPU do cliente `-C` vivo | abaixo de 1 tick em 3000 rodadas (< 0,003 ms) |
| **Total** | **0,183–0,193 ms** |
| Parede | 0,182–0,193 ms |
| Só `analyze` + `reduce` (sem captura) | 0,041 ms |
| Pico de memória do processo de teste | 6,7 MB |

Hoje, com o chat aberto, o Python gasta **1,261 ms** de CPU por captura, mais 0,200 ms do Rust
(rota privada) e 0,207 ms do tmux (`harnesses.md`, "Custo da observação terminal"): 1,668 ms por
rodada no total. A rodada do `Monitor` custa 0,19 ms com o tmux incluído, sem HTTP e sem o
Python.

Método: `crates/hangar-server/tests/state_monitor_round.rs` (`--ignored`, à mão), tmux isolado
`-L` com `-f /dev/null`, pane 100×40 com 180 linhas e spinner parado (o mesmo pane sintético da
medida do Python), captura `-S -200` pelo `TerminalPool` (cliente `-C`, sem processo novo por
rodada). 20 rodadas de aquecimento, 4 lotes de 3000 rodadas seguidas, sem espera entre elas. CPU
= delta de `utime+stime` em `/proc/<pid>/stat` (ticks de 10 ms) do processo de teste, do servidor
tmux e do cliente de controle, dividido pelas rodadas. Fatos do Python e arquivos da sessão
ficaram de fora (a Task 2 não liga o `Monitor` a eles); entram na medida da Task 5.
