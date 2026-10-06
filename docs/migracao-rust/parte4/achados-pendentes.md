# Parte 4: achados pendentes

Achados médios e baixos das revisões por Task que entraram na `feat/parte4` sem conserto (ritmo
"juntar primeiro, corrigir depois"). Cada item sai daqui quando for corrigido, com o hash.

## Antes da `hangar-server-parte1`

- Task 10: prova na VM DELPHI-02 (Step 23) com web e nativo. Os testes `term::conpty::*` e os
  `cfg(windows)` da Task 7 passaram no job Windows do CI (run 37467170394). O nativo responde ao
  `ESC[6n` (`desktop-native/src/term_view.rs:56-58`, `Event::PtyWrite`); o xterm.js também.
- Task 5: no Windows a prévia pelo pane captura a 0,15 s com um processo psmux por toque
  (~25–50 ms cada) enquanto a sessão trabalha sem arquivo do hook; medir na VM e, se pesar,
  limitar o ritmo rápido no Windows. O Monitor no Windows (psmux) e o convidado de convite de
  verdade não foram conferidos (só o Connect).

## Médios e baixos

- Task 1: envio de fatos que falha não é reenviado sozinho (o Rust recupera pelo retrato a cada
  25 s ou pelo pulo de sequência); lote com o Rust travado atrasa até 2 s por sessão; `_down`
  global; `_seq`/`_sent` sem poda; fim de `transfer_active` não avisa (o Rust usa `session.dead`,
  conferido na hora); corpo da rota `state-service` lido antes do teto; empurrão acima de 64 KiB
  (pergunta com resumo enorme) é recusado pela ponte.
- Task 2: cache do `permission.observe` pode ficar velho se o Python trocar o modo por outro
  caminho até o pane mudar; `None` do pane do agente fica 60 s em cache (igual ao Python).
- Task 3: a captura rápida traz a análise completa do pool, refeita no caminho com corte; clone
  do texto do hook por toque; `norm()` sem os separadores `\x1c-\x1f`; a época zera a prévia sem
  publicar vazio (o reset do hub cobre); largura por `unicode-width` pode divergir do Python em
  caracteres raros.
- Task 4: com `awaiting_input` e sem `ask_question` emitido, o `Monitor` relê o arquivo da
  pergunta a cada rodada (o Python só relê na mudança de estado) — desvio aceito: o hook pode
  gravar depois de o menu aparecer; problema do runtime perde para o dos fatos e da observação
  (igual ao Python); o celular não mostra o detalhe dos códigos novos; casamento degradado vai
  ao log em `info` (igual ao Python); nativo não compilado nesta Task (só o `messages`).
- Task 5: pane do agente cai em `=nome:` quando a descoberta falha (igual ao Python, só log);
  `RuntimeView` não limpa quando o ator fecha sem evento; prévia do hook ilegível cai no pane só
  com log; linha acima de 1 MiB no canal privado derruba o stream do convidado (reconecta);
  `Drop` sem runtime não solta o consumidor do pool.
- Task 8: fila de entrada limitada em quadros (64), não em bytes; `lock().unwrap()`; erro de
  leitura do PTY que não é EIO fecha como fim normal; resize do PTY que falha só aparece em debug;
  `restore_after_crash` na subida sem diário (só `warn`); troca de painel com desmontagem acima de
  10 s fica com dois painéis (vai ao diário).
- Task 10: `Drop` implícito do `Pty` (future cancelada, pânico) solta o mestre antes de matar o
  filho no Windows; no ConPTY a saída não dá EOF quando o `tmux attach` sai
  (microsoft/terminal#4564), e `exit` deixa o painel mudo até o cliente fechar (igual ao Python);
  o caminho `forget` vaza a vaga do teto de painéis; `fail()` no Windows usa `child.wait()` sem
  prazo se o `TerminateProcess` falhar; erro do `try_wait` vira `client_not_reaped` sem log;
  `dropping_writer_writes_nothing` não separa "nada escrito" de "conhost morreu no EOF".
- Task 9: rota privada do terminal responde 400/404 sem linha de log própria no Rust (o Python
  registra o status); quadro do cliente sem teto próprio no Python (Rust e uvicorn limitam); em
  `pending` o `/api/config` publica a capacidade do Python; braço morto `TermActive` no
  `execute`.
- Task 7: psmux pode escrever erro no stdout com código diferente de 0 e virar "quadro" (conferir
  na VM, Task 10/13); a linha da lista mostra só `list_capture_failed` e o código fino só vai ao
  log (já era assim); pior caso de 5 s + 5 s quando captura e `has-session` estouram; stderr e
  `io::Error` descartados (só o código); statusline e limite congelam sem `problema` na falha (já
  era assim).
