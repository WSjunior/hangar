# Parte 4: achados pendentes

Achados médios e baixos das revisões por Task que entraram na `feat/parte4` sem conserto (ritmo
"juntar primeiro, corrigir depois"). Cada item sai daqui quando for corrigido, com o hash.

## Para a Task 5 (troca de dono)

- `PoolCapture` não libera o consumidor sozinho no fim/abort e congela o vínculo: recriar no
  `rebind` (Task 2).
- `rebind` só publica no tique seguinte (até 0,75 s); o desenho pede publicar logo depois do
  `rebind` (Task 2).
- O `Sources` de produção (arquivos da sessão e `FactsStore` ligados ao `Monitor`) é da Task 5
  (Task 2).
- Retrato de fatos que nunca chegou dá `RoundFacts` vazio; o `session.dead` do Python reconfere a
  troca (Task 2).
- Prévia (Task 3): o hub chama `preview::committed_from_frame(&quadro)` a cada linha do
  `FileTail` e zera o `committed` no `rebind`; acordar o `Monitor` no commit para a limpeza ser
  imediata (hoje vem no toque seguinte, até 0,15 s trabalhando e 0,75 s parado);
  `preview_files`/`preview_capture` com `spawn_blocking` e prazo.
- Task 4: os métodos do `Sources` com implementação padrão escondem o que a produção esquecer
  (a Task 5 implementa todos e prova num teste); `deliverable()` sem retorno: falha só o
  implementador registra e a borda não repete (igual ao Python); `suggest` e `asked` são por
  `Monitor`, quem conecta depois depende da reposição do `SideCache`.

## Antes da `hangar-server-parte1`

- Task 10: prova na VM DELPHI-02 (Step 23) com web e nativo; os testes `term::conpty::*` e os
  `cfg(windows)` da Task 7 só rodam no job Windows do CI (conferir pelo log). O nativo responde ao
  `ESC[6n` (`desktop-native/src/term_view.rs:56-58`, `Event::PtyWrite`); o xterm.js também.

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
  `execute`. Compilação Windows da rota privada e do `term.active` só no job Windows do CI.
- Task 7: psmux pode escrever erro no stdout com código diferente de 0 e virar "quadro" (conferir
  na VM, Task 10/13); a linha da lista mostra só `list_capture_failed` e o código fino só vai ao
  log (já era assim); pior caso de 5 s + 5 s quando captura e `has-session` estouram; stderr e
  `io::Error` descartados (só o código); statusline e limite congelam sem `problema` na falha (já
  era assim). Testes `cfg(windows)` ainda não rodaram: saem do job Windows do CI no próximo push.
