# Repasse do hangar-server ao Python lento em respostas grandes

## O defeito (medido)

Pedido que o Rust ainda não atende é repassado ao Python (`crates/hangar-server/src/proxy.rs`).
Medição do `scripts/medir-rust.sh` no notebook do dono (04/10, `ac5c975d`), média de 5:
`GET /api/costs` **106 ms pelo Rust** (porta 8765) contra **12 ms direto no Python** (porta interna).
Medição anterior no mesmo notebook: 117 ms × 11 ms. Nesta máquina, com resposta pequena, o repasse
custa ~1 ms (5 × 4 ms). Ou seja: ~95 ms a mais só no repasse quando a resposta é grande. Enquanto a
migração não termina, toda resposta grande repassada paga isso.

## O que fazer

1. Reproduza num backend de teste isolado desta branch e meça a resposta grande pelo Rust e direto
   no Python (mesma rota, mesmo corpo). Se `/api/costs` do teste vier pequeno, use qualquer rota de
   leitura do Python que devolva corpo grande (centenas de KB/MB) ou gere dados de teste para isso.
   Meça também por tamanho de resposta (curva), para achar onde o custo aparece.
2. Ache a causa com evidência (não chute). Suspeitas a conferir: Nagle + ACK atrasado no socket
   entre Rust e Python ou Rust e cliente (`TCP_NODELAY`; o valor ~40 ms × 2 é típico), leitura do
   corpo em pedaços pequenos ou acumulada inteira antes de responder, cliente HTTP sem reuso de
   conexão, compressão, conversão de `Transfer-Encoding`/`Content-Length`, cópia do corpo.
3. Corrija pela causa, com teste que falha sem o conserto (no Rust, se possível medindo que o
   repasse de um corpo grande não acrescenta mais que alguns ms sobre o upstream).
4. Prova: a mesma medição antes e depois no backend de teste; e confira que streaming (SSE de rotas
   repassadas), upload e respostas pequenas continuam corretos.
5. `cargo test --locked --workspace` em `crates/`; revisão independente por subagente
   (`ecc:rust-reviewer`) antes do push.

Backend de teste isolado: `HOME` temporário, portas livres diferentes de 8765/8766/8768,
`CP_AUTH_TOKEN` próprio, `tmux -L <nome>` próprio (a branch já tem `bb80cf31`). Pare tudo e apague
o `HOME` no fim. Anote medições e causa em `docs/migracao-rust/repasse-lento.md`.

## Entrega

Branch `fix/proxy-latency` (este cwd, de `origin/hangar-server-parte1`). Commits em inglês, `git add`
de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Push só desta branch e CI do `server.yml` verde nos
três sistemas. Mande para `migracao-rust-2`: causa com evidência, conserto, números antes/depois,
link do CI. Não junte na `hangar-server-parte1`.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768 (medir contra o
backend real só em leitura, sem reiniciar); tocar no tmux padrão ou em sessão real; rodar
instaladores; mexer na `main` ou na `hangar-server-parte1`.
