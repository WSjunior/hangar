# Parte 4: medição

## Terminal real (Task 8, 06/10/2026)

**O PTY do dono no Rust tira o Python do caminho dos bytes e corta a CPU por MB pela metade;
vazão e eco não mudam, porque quem limita é o servidor tmux.**

Como: `scripts/medir-terminal.py`, backend isolado (classe `Prova`: HOME, portas, token e
`tmux -L` próprios), `hangar-server` em release, nenhum outro cliente. Uma sessão `bash` de
200×50 no tmux da prova, painel aberto pelo WebSocket do dono. Vazão = `cat` de um arquivo de
50 MB (linhas de 100 caracteres), do Enter até o marcador do fim chegar ao cliente. Eco = 50
teclas, do envio ao primeiro quadro binário com a tecla de volta. CPU = `utime+stime` de
`/proc/<pid>/stat` durante o `cat`, dividida pelos 50 MB (o cliente `tmux attach` de cada painel
fica fora da conta, nos dois lados). Memória = RSS e threads antes e depois de abrir 10 painéis,
dividido por 10. "Antes" = o código da base `82017cb7d` (o Rust repassa o WebSocket ao Python, que
abre o PTY); "reserva" = a mesma base com `CP_RUST_SERVER=0`; "depois" = o código desta Task.
Amostras separadas por `/`; a última de cada coluna é do script com o eco filtrado por quadro
binário.

| Medida | Antes (Python atrás do Rust) | Reserva (Python sozinho) | Depois (Rust) |
|---|---:|---:|---:|
| Vazão, MB/s | 61,3 / 55,2 / 59,6 | 61,4 / 61,7 | 60,6 / 61,1 / 60,0 |
| Eco, mediana ms | 0,56 / 0,62 / 0,56 | 0,51 / 0,59 | 0,55 / 0,52 / 0,58 |
| Eco, p95 ms | 0,79 / 0,85 / 0,82 | 0,75 / 3,55 | 0,93 / 0,75 / 0,86 |
| CPU por MB, Python | 7,8 / 12,6 / 7,6 ms | 6,8 / 6,4 ms | 0,0 / 0,0 / 0,4 ms |
| CPU por MB, Rust | 3,0 / 3,4 / 3,0 ms (repasse) | — | 4,6 / 5,0 / 5,2 ms |
| CPU por MB, Python + Rust | 10,8 / 16,0 / 10,6 ms | 6,8 / 6,4 ms | 4,6 / 5,0 / 5,6 ms |
| CPU por MB, servidor tmux | 16,0 / 17,8 / 16,4 ms | 16,2 / 16,0 ms | 16,2 / 16,4 / 16,6 ms |
| Por painel, Rust | ~0 MB, 0 thread | — | ~0,25 MB, +2 threads |

- O tmux gasta ~16 ms por MB em qualquer cenário (ele redesenha a tela): é ele que segura a vazão
  em ~60 MB/s, então trocar o dono do PTY não mexe nela nem no eco.
- Python + Rust por MB cai de 10,6–16,0 ms para 4,6–5,6 ms; contra o Python sozinho (6,4–6,8 ms)
  o Rust gasta ~20% menos e livra o laço de eventos do Python dos bytes do terminal.
- Cada painel no Rust custa 2 threads (leitor e escritor do PTY, teto de 64 painéis) e ~0,25 MB.
  A memória do Python por painel não se mede assim: o RSS dele oscila mais que o painel (de −1,09
  a +0,05 MB nas amostras).

Não medido aqui: celular e nativo de verdade (Task 12), Windows (Task 10), Origin (a medida
conecta sem `Origin`).
