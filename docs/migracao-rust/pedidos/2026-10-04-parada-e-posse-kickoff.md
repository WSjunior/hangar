# Backend não para no SIGTERM e o chat sem terminal cai com "sessão em transferência"

## Defeito 1: o backend não termina no SIGTERM

Nas duas máquinas, todo restart do serviço acaba em SIGKILL:
- esta máquina, 04/10 07:18:43 (`journalctl --user -u hangar-backend`): `State 'stop-sigterm' timed
  out. Killing.` → SIGKILL em `uv` e `python3`;
- notebook, 04/10 13:17:38 e 13:24:11: o mesmo, SIGKILL no `python3`.

SIGKILL pula a parada ordenada (fila, diário, posse do Rust, canos, filhos). Leia
`docs/decisoes/instalacao.md` (Regras vigentes) e a parte do `hangar-server` em
`docs/decisoes/plataforma.md` antes. Ache o que segura a saída (tarefa/thread que não termina,
`Supervisor.stop`, coordenador do runtime, observador do terminal, laço do Rust, SSE abertos,
`TimeoutStopSec` do unit) com evidência (py-spy/faulthandler/log de cada etapa da parada) e corrija
pela causa. O backend deve sair sozinho bem antes do teto do systemd, sem matar canos de sessão sem
terminal (eles sobrevivem ao restart por desenho).

## Defeito 2: SSE de sessão sem terminal fecha com "sessão em transferência"

Log do notebook (backend.log), sessões sem terminal sem nenhuma troca de modo ou conta em curso:
```
12:44:19 claude headless: desligou name=hangar-mobile-deliveries (cano segue vivo)
12:44:19 sse: fechou name=hangar-mobile-deliveries dur=14.4s motivo=erro no pump: RuntimeError: sessão em transferência; aguarde a posse ser confirmada
13:07:27 sse: fechou name=hangar-3 dur=24.6s motivo=erro no pump: RuntimeError: sessão em transferência; aguarde a posse ser confirmada
```
Também às 11:10:42 com a `Projetos` recém-criada. Ache quem levanta essa exceção no caminho do SSE
(`runtime_coordinator`, fases `PreparingRust`/`RecoveringPython`, adoção pelo Rust), por que o
chat aberto derruba o fluxo em vez de esperar a posse, e corrija: o chat não pode cair por causa da
troca de dono interna. Confira também se "desligou … (cano segue vivo)" logo depois de "religou" é
esperado.

## Como provar

Backend de teste isolado desta branch (`HOME` temporário, portas livres diferentes de
8765/8766/8768, `CP_AUTH_TOKEN` próprio, `tmux -L` próprio; a branch tem `bb80cf31`), rodando como
unit do systemd de usuário temporário (ou com SIGTERM e o mesmo teto) para medir a parada. Sessões
Claude só Haiku (`--model claude-haiku-4-5`), `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`.
Casos: parar com sessões com e sem terminal vivas e chat aberto (tempo de saída, nenhum SIGKILL,
canos sem terminal vivos e readotados depois); abrir o chat de sessão sem terminal durante a adoção
pelo Rust e depois de restart (o SSE não cai). Teste que falha sem cada conserto. Anote em
`docs/migracao-rust/parada-e-posse.md`. Pare tudo e apague o `HOME` no fim.

## Entrega

Branch `fix/shutdown-and-ownership` (este cwd, de `origin/hangar-server-parte1`). Commits em inglês,
`git add` de caminhos explícitos, `HANGAR_SEM_PASSO=1`. Revisão independente por subagente antes do
push. Push só desta branch; CI (`server.yml` nos três sistemas). Mande para `migracao-rust-2`: causa
com evidência de cada defeito, conserto, testes, casos reais, link do CI. Não junte na
`hangar-server-parte1`.

## Proibido

Subir/reiniciar/parar o backend real ou o `hangar-backend`; usar 8765/8766/8768; tocar no tmux
padrão ou em sessão real; rodar instaladores; mexer na `main` ou na `hangar-server-parte1`; modelo
diferente de Haiku; log com texto de conversa.
