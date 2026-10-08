# Medição da 5-0 (escritas do Claude no Rust): roteiro

Sem números. O dono não autorizou subir nenhum backend nesta execução, nem isolado, então esta
página só diz o que medir, antes e depois, e como. Os números saem do uso real do dono e entram
aqui quando existirem.

## Montagem

- Binário em **release** nos dois lados: `main` (antes) e `hangar-server-parte5-claude` (depois).
- Backend isolado pela classe `Prova` de `scripts/prova-dono-unico.py`: `HOME` temporário, portas
  próprias, `tmux -L` próprio e `matar_orfaos` desligado. Nunca o backend de uso (subir um segundo
  derruba as sessões sem terminal vivas).
- Claude só no Haiku, com `CLAUDE_CONFIG_DIR=/home/jefferson/.claude-02-200`.
- 5 sessões Claude: 3 com terminal e 2 sem. Cada uma recebe uma mensagem a cada 10 s por
  `POST /input`, por 5 minutos.

## O que medir

| Medida | Como | Esperado depois |
|---|---|---|
| Chamadas a `/internal/*` por minuto | contador do `diag` do Python durante a carga | caem: o envio não volta ao Python |
| Latência do `POST /input` até a resposta | tempo da chamada até o primeiro `assistant_msg` no `/events`, mediana e p95 | igual ou menor |
| CPU do Python parado | `ps`/`pidstat` do processo Python, 60 s, sessões abertas e sem carga | igual ou menor |

## Conferir junto (uso real, não de bancada)

Mandar com e sem terminal, interromper, responder pergunta por opção e por chat, permissão
segurada no celular, renomear durante um envio e `/clear`.
