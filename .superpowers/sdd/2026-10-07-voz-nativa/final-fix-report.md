# Correções da revisão final da voz nativa

Check: `cd desktop-native && nice -n 19 ionice -c3 cargo check --tests -j 4` passou (só avisos antigos, nenhum em voice/). Testes escritos, não rodados (regra do projeto). Nenhuma verificação em uso real (microfone/Codex).

## Importantes
1. audio.rs `output_config` (l.88-96): parte de `default_output_config()`; só se a taxa não é 48 kHz procura faixa com MESMO formato e MESMOS canais que contenha 48 kHz; senão mantém o padrão (o Resampler cobre).
2. voice/mod.rs, handler de `turn/completed`: status diferente de "completed" chama `gate.user_spoke()` e responde "O turno foi interrompido; nada foi enviado." ao envio guardado. Sem teste novo (vive no laço assíncrono; o `SendGate` já tem teste).
3. organizer.rs `SpokenTurns` (item_started / allows / turn_completed, teto de 64) + mod.rs: `item/started` userMessage cujo texto não começa por `[RESULTADO DA SESSÃO` registra o `turnId`; `Send`/`Hold` de turno fora do conjunto respondem "Pedido recusado: só uma fala do usuário pode gerar envio."; `turn/completed` remove o turno. Teste: `only_user_speech_turns_allow_sends`.
4. voice_ui.rs: o fim do turno não fala mais na hora. `voice_turn_finished(state, cx)` marca `reply_pending` e arma `arm_reply_wait` (1,5 s, `REPLY_WAIT`, com `reply_epoch`); fala na próxima `assistant_msg` com a sessão fora de "working" (`voice_message`) ou quando o relógio vence (`voice_reply_wait_over`), o que vier primeiro. `voice_session_opened(cx)` agora também recupera a pendência ao voltar para a sessão que estava na vigia (`watched` = true). Assinaturas ajustadas em app.rs (2 chamadas). Teste: `final_reply_after_intermediate_text_is_a_new_id`.

## Menores
5. `receive_voice_gate`: `codex == None` com chamada viva chama `stop_voice`.
6. `waiting_text` (pergunta de `chat.ask`, senão fim da última resposta, até 600 chars; prefixo "A sessão está esperando sua resposta: "). Teste: `waiting_text_prefers_question_then_reply_tail`. Texto vai ao organizador, não à tela: sem chave i18n.
7. `bar_height` (passos inteiros de px) usada na pílula; `Levels` só notifica quando a altura de uma barra ou o rótulo falando/ouvindo muda. Teste: `bar_height_moves_in_whole_pixels`.
8. `Failed(Organizer)` não abre o painel; a pílula em chamada ganhou o ponto vermelho de erro; `Phase(Live)` e nova chamada limpam o erro.
9. `post(..., String::new(), ...)` no envio por voz (rascunho vazio já é ignorado em `receive_sent`).
10. Teste renomeado para `send_reply_names_session_and_status`.
11. docs/decisoes/harnesses.md: um bullet em "Regras vigentes" (antes de "O /clear e o rodapé do Claude Code").

## Pendências / riscos
- Item 3 depende de o `item/started` do turno de delegação trazer `item.content[].text` e `turnId` e de o `item/tool/call` trazer `turnId`; formato não medido ao vivo. Se a delegação realtime não emitir `userMessage`, todo envio seria recusado (falha fechada): conferir na primeira chamada real.
- Item 4: quando a `assistant_msg` final chega antes do estado, a fala sai 1,5 s depois do fim do turno.

## Correção pontual

- `desktop-native/src/app/voice_ui.rs` (`voice_reply_wait_over`, agora com `cx`): se `chat.state.state == "working"` retorna mantendo `reply_pending` (o fim do turno rearma); se `history_installed` é falso, rearma a espera em vez de falar/limpar.
- `desktop-native/src/voice/mod.rs` + `organizer.rs`: `SpokenTurns` também registra a fala do usuário em `item/completed` (mesmo filtro `[RESULTADO DA SESSÃO`). O cancelamento de envio pendente continua só em `item/started` e no delta de transcrição.
- Check: `cargo check --tests -j 4` em `desktop-native` passou (só warnings antigos de código morto). Sem cargo test.
