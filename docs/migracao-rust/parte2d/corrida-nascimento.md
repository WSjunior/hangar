# Mensagem para sessão que ainda está nascendo

Pedido: `../pedidos/2026-10-04-corrida-nascimento-kickoff.md` (árvore da parte 1). Branch
`fix/terminal-birth-race`, de `ac5c975d`.

## Defeitos e causas

**Com terminal: recusa "vínculo gerenciado indisponível; escrita suspensa".** O pane existe antes
do agente: na sessão de motor, quem roda primeiro é o `hangar-engine`, que o registro não
reconhece como agente. Nessa janela `runtime_terminal.resolve_binding` devolve `None`,
`outside_scope` também não confirma que a sessão está fora do escopo, e
`RuntimeCoordinator.prepare_session` recusa. Antes da 2D o teclado esperava a TUI e a fila
segurava a mensagem.

**Sem terminal: a primeira mensagem sumia (0 entregas em 4 de 4).** O envio volta `deferred`
enquanto o processo sobe; o drain da reserva Python reivindica a entrada (a reivindicação já a marca
entregue); a passagem da posse ao Rust (`adopt` → `LegacyBridge.quiesce`) cancela o drain antes da
escrita. O `recover` só devolve reivindicações do executor terminal, então o Rust adota a fila com a
entrada "entregue" e nunca a manda. Diário da `hl1`: seq 7 `deferred`, 16 `claim` com a entrada,
17 `quiesce`/`recover`, nenhuma fase de escrita do texto.

## Consertos

- `prepare_session` espera o nascimento (`_await_birth`): enquanto o pane da sessão tiver menos de
  15 s, não for de uma vida anterior ao registro anterior de mesmo nome e `outside_scope` for falso,
  relê o vínculo a cada 0,25 s. Achou, segue o caminho normal (registro, posse, Rust). Passou da
  janela, recusa como antes. Vínculo que existia e se perdeu continua suspenso na hora; nome sem
  pane continua "sessão não encontrada".
- O drain do Claude sem terminal registra a entrada reivindicada e marca quando a escrita começa.
  O `quiesce`, depois de cancelar as tarefas, devolve à fila (direto no store, sob a trava da vaga)
  a entrada cuja escrita não começou. Escrita começada fica como está: o diário a trata como incerta.

## Testes reais

Backend desta branch isolado: `HOME` temporário, `hangar-server` debug desta branch na 19765,
Python atrás em porta de loopback, convite e Connect em 19766/19768 (constantes trocadas pelo
lançador), `CP_AUTH_TOKEN` próprio, `tmux -L hangar-birth` por embrulho no `PATH`. O embrulho do
`claude` fixava `--model claude-haiku-4-5` e `CLAUDE_CONFIG_DIR=~/.claude-02-200` só no processo
do agente (o backend ficou sem a conta, para não instalar hooks nela). Para abrir a janela do motor,
o embrulho segura o pane 2 s num `sh -c` anônimo antes do `exec` do `claude`. Entregas contadas no
transcript cru (linha `user` com o marcador único da mensagem); 18+ respostas, todas
`claude-haiku-4-5-20251001`.

| Caso | Sem conserto | Com conserto |
|---|---|---|
| Com terminal, mensagem logo após criar (espera de 2 s) | `400 vínculo gerenciado indisponível; escrita suspensa` | 10 de 10 com 1 entrega; `/input` respondeu em ~2,3 s com `delivered:false`; Rust dono, adiou (`terminal_not_executed`) e a fila entregou |
| Com terminal sem espera (agente reconhecido na hora) | 1 entrega | 1 entrega |
| Sem terminal, mensagem logo após criar (com espera) | 0 de 3 | 5 de 5 com 1 entrega |
| Sem terminal sem espera | 0 de 1 | 2 de 2 com 1 entrega |
| Nome inexistente | — | `404 sessão não encontrada — recado NÃO enfileirado` |

As linhas "com conserto" acima são da primeira versão do segundo conserto. Depois da revisão
independente (devolver só com o drain terminado, registrar também a reivindicação cancelada no
meio, janela de 15 s), uma rodada final: com terminal 3 de 3 com 1 entrega; sem terminal 2 de 3;
nome inexistente 404.

**Resto sem conserto (`fh1`):** a passagem ao Rust cancelou o drain depois de a escrita começar
(fase de escrita aberta no diário) e cancelou junto o leitor do cano, que traria a confirmação. A
escrita ficou incerta e, pela regra da 2D, entrada incerta não se repete: a mensagem ficou marcada
entregue e o `claude` não a recebeu (transcript sem a linha, cano sem registro). Consertar pede
decidir se o `quiesce` espera a escrita em voo terminar antes de cancelar o leitor; ficou para o
coordenador.

O "sessão em transferência" do SSE no log do notebook é a janela da adoção (fase `PreparingRust`);
o cliente reconecta e não perde mensagem.

## Fora deste conserto

- O caminho Codex sem terminal não tem o mesmo registro de reivindicação; não foi medido aqui.
- Reivindicação do drain Python perdida por queda do backend (não por transferência) continua sem
  devolução: o processo novo não sabe se a escrita começou.
