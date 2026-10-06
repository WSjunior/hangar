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

## Médios e baixos

- Task 1: envio de fatos que falha não é reenviado sozinho (o Rust recupera pelo retrato a cada
  25 s ou pelo pulo de sequência); lote com o Rust travado atrasa até 2 s por sessão; `_down`
  global; `_seq`/`_sent` sem poda; fim de `transfer_active` não avisa (o Rust usa `session.dead`,
  conferido na hora); corpo da rota `state-service` lido antes do teto; empurrão acima de 64 KiB
  (pergunta com resumo enorme) é recusado pela ponte.
- Task 2: cache do `permission.observe` pode ficar velho se o Python trocar o modo por outro
  caminho até o pane mudar; `None` do pane do agente fica 60 s em cache (igual ao Python).
- Task 8: fila de entrada limitada em quadros (64), não em bytes; `lock().unwrap()`; erro de
  leitura do PTY que não é EIO fecha como fim normal; resize do PTY que falha só aparece em debug;
  `restore_after_crash` na subida sem diário (só `warn`); troca de painel com desmontagem acima de
  10 s fica com dois painéis (vai ao diário).
- Task 7: psmux pode escrever erro no stdout com código diferente de 0 e virar "quadro" (conferir
  na VM, Task 10/13); a linha da lista mostra só `list_capture_failed` e o código fino só vai ao
  log (já era assim); pior caso de 5 s + 5 s quando captura e `has-session` estouram; stderr e
  `io::Error` descartados (só o código); statusline e limite congelam sem `problema` na falha (já
  era assim). Testes `cfg(windows)` ainda não rodaram: saem do job Windows do CI no próximo push.
