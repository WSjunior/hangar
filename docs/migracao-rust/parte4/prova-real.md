# Parte 4: prova de uso real isolada (Task 12, 06/10/2026)

**Com o Rust de pé, o estado, a prévia, a pergunta nativa, o cartão de permissão e o terminal real
de Claude com terminal funcionam por qualquer porta sem nenhum `StateMonitor`, `PreviewBroker` ou
PTY no Python, com uma captura por rodada mesmo com três chats abertos.** Na reserva e depois de 3
quedas do Rust, o Python volta a servir estado e terminal. Ficaram de fora três defeitos que não são
da parte 4 (Codex e a rota `/select`), descritos no fim.

Como: `scripts/prova-parte4.py` (rodar com `backend/.venv/bin/python`), backend isolado pela classe
`Prova` (HOME, portas, token e `tmux -L` próprios, `matar_orfaos` desligado), `hangar-server` e
`hangar-cano` em release desta árvore (base `feat/parte4` com a Task 6, contrato 32). Claude só
Haiku na conta 02-200 (`~/.claude-jefferson` só como destino da troca de conta); Codex `gpt-6-luna`
numa `CODEX_HOME` temporária. Convite pela porta isolada do convite (o lançador troca a detecção de
rede para `127.0.0.1`), Connect pela porta isolada do Connect com o token do dono. Contadores no
lançador: `StateMonitor` e `PreviewBroker` criados no Python por sessão e provedor, PTY aberto pelo
`termsock`, modo do dono do runtime. Capturas: log do servidor tmux da prova (SIGUSR2) numa janela,
`capture-pane` por pane, separando o cliente de controle (`-C`, o pool do Rust) das avulsas.

## Step 26: Claude Haiku com terminal

| Caso | Resultado | Evidência |
|---|---|---|
| Uma captura por rodada, `ct` parada | ok | 1 chat: 1,40/s pelo `-C`; dono + convidado de convite + Connect: 1,47/s (esperado 1,33/s, rodada de 0,75 s); nenhuma captura avulsa de `ct` com chat aberto; só a lista aberta: 0/s pelo `-C`, 1 avulsa em 15 s |
| Convidado e Connect olhando o chat | ok | os dois recebem `state` pelo canal privado; `StateMonitor`/`PreviewBroker` Python para `ct`: 0/0 |
| Trabalhando/parada, prévia pelo arquivo do hook | ok | chat viu `working` e `idle`; 127 prévias `md`, 0 do pane; captura sem toque rápido (1,5/s); prévia limpa no fim |
| Prévia pelo pane (arquivo do hook escondido nas duas contas) | ok | 17 prévias do pane, 0 `md`; captura rápida de 0,15 s só durante o turno (6,5/s); prévia limpa no fim |
| Pergunta nativa (AskUserQuestion) | ok | lista e chat `awaiting_input`; `ask_question` uma vez; volta a `idle` depois do Esc |
| Cartão de permissão, modo manual, `Bash`, sem chat | ok | lista `awaiting_input` com 2 opções (cartão segurado pelo hook, o pane não o mostra); negar pelo app 200; sai do cartão |
| Cartão de permissão, com chat aberto | ok | lista e chat `awaiting_input`; negar 200; sai do cartão |
| `/clear` com o chat aberto | ok | 200; `reset` no chat; `working` → `idle` depois; chat não caiu |
| Troca de conta (`em_troca`) com o chat aberto | ok | 200 em 0,75 s; nenhum `state` `dead` durante a troca; a sessão roda na conta B e o estado segue |
| Sessão morta com o chat aberto | ok | `state` `dead` em 1,2–2,2 s; sai da lista |
| Fim do Step | ok | modo `rust`; `StateMonitor` 0, `PreviewBroker` 0 para as sessões Claude com terminal |
| Sugestão (`suggest`) | inconclusivo | o Claude não ofereceu sugestão em nenhuma rodada |

Prévia: um pedido curto termina antes de aparecer no pane, e uma resposta igual à anterior não
gera prévia (a regra do Python, `is_committed`, descarta prévia contida na resposta já gravada).
Por isso o caso do pane pede um texto longo e diferente. Os `.hangar-preview` das duas contas da
prova apontam para a mesma pasta real: esconder o arquivo só numa delas deixa o `Monitor` na fonte
arquivo.

## Step 27: cartões fora do YOLO

| Caso | Resultado | Evidência |
|---|---|---|
| Claude sem terminal, modo manual | ok | lista e chat `awaiting_input` com 3 opções; negar 200; sai do cartão |
| Codex sem terminal, "Ask for approval" | **cartão ok, resposta falha** | lista e chat `awaiting_input` com 3 opções; `POST /select` 500 (defeito 1); o cartão fica |
| Codex com terminal, "Ask for approval" | **falha (fora da parte 4)** | menu de aprovação na TUI ("Would you like to run the following command?", 3 opções); lista e chat `working` (defeito 3). A troca de modo pela rota deu 409 (defeito 2) e foi feita pelo teclado |

No "Ask for approval" o sandbox é só leitura e o modelo decide se pede escalada: o pedido precisa
dizer `sandbox_permissions="require_escalated"`, senão o `touch` falha calado e nenhum cartão
aparece.

## Step 28: terminal real

| Caso | Resultado | Evidência |
|---|---|---|
| Dono pela porta do Rust | ok | eco; janela no tamanho do painel; cliente tmux sai ao fechar; tamanho reposto (200×50); `@hangar_term_size` limpo; nenhum PTY no Python; `/api/config` `terminal_panel` `true` |
| Convidado de convite | ok | eco; o dono abrindo pela porta do Rust fecha o convidado com 1000 `outra conexao assumiu`; convidado reabre; revogar fecha com 4410; nenhum PTY no Python |
| Dono pelo Connect | ok | eco; cliente sai; nenhum PTY no Python |
| Duas conexões do dono | ok | a 1ª fecha com 1000 `outra conexao assumiu`; um cliente tmux só; janela no tamanho da 2ª; tamanho reposto |
| 409 com o painel aberto | ok | `POST /select` 409 `erro_terminal_aberto` com o texto; sem painel não é 409 |
| Atalho da sessão e No Hangar | ok | `shortcut-shell` 202 nos dois; eco em `?shortcut=` e em `/api/hangar-terminals/{id}/term` |
| `term-<nome>` | ok | `POST /shell` → `term-b1`; eco |
| Queda de rede (cliente que não responde ping) | ok | desmonta em 40,2 s; tamanho reposto |
| Queda do Rust com o painel aberto | ok | tamanho guardado na opção ao anexar; Rust novo; cliente tmux órfão sai; Rust novo repõe 200×50 e limpa a opção; painel reabre e ecoa |

A janela do tmux fica com uma linha a menos que o painel (a barra de status): painel 100×30 →
janela 100×29.

## Step 29: carga, reserva e quedas

CPU em ms por segundo (`utime+stime` com filhos de `/proc/<pid>/stat`, janelas de 30 s), sessões
Haiku reais, metade com terminal, cada uma aquecida com uma mensagem. "repouso" = só a lista do dono
aberta; "chats" = mais um chat aberto em cada sessão; "ativo" = mais 1 de cada 4 sessões escrevendo
1 a 1200 por extenso, interrompidas depois da janela. Antes da 1ª janela a prova espera o Rust
assentar (10 s) e abre e fecha um chat em cada sessão: o 1º chat dispara o índice de transcripts
do Python (~3 s de CPU, uma vez), que numa rodada sem esse aquecimento pôs a janela "chats" de 1
sessão em 76–101 ms/s.

| Sessões | Cenário | Python + filhos | Rust | tmux |
|---:|---|---:|---:|---:|
| 1 | repouso | 12,0 | 6,3 | 0,0 |
| 1 | chats | 34,7 | 7,3 | 0,3 |
| 1 | ativo (1) | 15,0 | 9,3 | 1,0 |
| 10 | repouso | 27,3 | 16,3 | 0,7 |
| 10 | chats | 36,3 | 18,0 | 2,0 |
| 10 | ativo (3) | 38,3 | 22,3 | 6,0 |
| 20 | repouso | 24,7 | 20,3 | 1,0 |
| 20 | chats | 56,7 | 31,3 | 4,3 |
| 20 | ativo (5) | 62,7 | 40,0 | 10,0 |

- Em todos os pontos, nenhum `StateMonitor`/`PreviewBroker` Python para as sessões com terminal, e
  todos os chats receberam `state`.
- No ponto de 20, uma das 5 sessões ativas terminou antes do fim da janela (4 ainda trabalhavam):
  a linha "ativo (5)" mede um pouco menos que 5 turnos inteiros. A prova marca isso como falha da
  conferência, não do produto.
- A janela "chats" de 1 sessão (34,7) ficou acima da "ativo" (15,0) mesmo com o aquecimento;
  o resto dela não foi atribuído (mesma ordem de grandeza da oscilação vista entre rodadas).
- O Python com 20 chats (56,7 ms/s) inclui os 10 chats das sessões sem terminal e a conexão interna
  de cada sessão com terminal (estatísticas, fila, faixa). Na medida da Task 5 (`medicao.md`, panes
  falsos, 20 chats com terminal) o Python ficou em 29 ms/s.
- Comparação com `../lista-estado/medicao.md` (antes da parte 4, sem chats): 20 sessões com 1
  cliente da lista, Python ≈ 80 ms/s e Rust 8 ms/s; aqui, no mesmo cenário (repouso de 20), Python
  24,7 e Rust 20,3, com a lista, o estado e a captura no Rust. As duas medidas são de dias e rodadas
  diferentes, com outras sessões reais na máquina: a comparação é de ordem de grandeza.
- Uma rodada anterior desta Task leu o pid do tmux antes de existir sessão (o servidor só nasce
  com ela) e mostrou a coluna tmux em 0; os números acima são da rodada corrigida.

| Caso | Resultado | Evidência |
|---|---|---|
| Reserva `CP_RUST_SERVER=0` | ok | modo `python`; chat `working` → `idle`; `StateMonitor` Python para `ct` criado; eco no terminal pelo PTY do Python; cliente sai |
| Rust derrubado 3 vezes em 60 s | ok | 3 kills em 2 s; o Python assumiu a porta |
| Depois das 3 quedas | ok | modo `python`; chat `working` → `idle`; `StateMonitor` Python para `ct` criado; eco pelo PTY do Python; cliente sai |

## Defeitos achados fora da parte 4 (não corrigidos)

1. **`POST /select` no Codex sem terminal responde 500, e o cartão de aprovação não se responde pelo
   app.** `api.py:5818` chama `route_sync` antes do ramo do sem terminal (`:5848`);
   `prepare_session(name, 'claude')` levanta "vínculo gerenciado indisponível" para a sessão Codex
   gerenciada (`runtime_coordinator.py:390`). Veio do dono único (03/10). O mesmo 500 aparece no
   `/select` de uma sessão tmux sem agente. Conserto provável: o ramo do sem terminal antes do
   `route_sync`.
2. **O picker `/permissions` do Codex 0.159.3 não é reconhecido.** O rodapé mudou para "enter select
   · esc back" e a lista chega depois de "Loading permission profiles…"; `codex_permissions.py`
   espera "Press enter to confirm or esc to go back", então `GET/POST /codex-permissions` dão 409
   `erro_permissao_picker` ("abriu pela metade").
3. **Codex com terminal com o menu de aprovação na TUI fica `working` na lista e no chat.** O pedido
   de aprovação vai ao cliente da TUI no app-server; o backend só lê `menu_codex` na abertura (sem
   thread, `registry.py:1877`), e o `codex_menu` do Rust não é usado. Já era assim antes da parte 4;
   o estado do Codex é da parte 5.

## Não conferido aqui

- Telas web, PWA, Expo e nativo, dois servidores (Task 13 com o dono).
- Windows (VM, Task 13).
- Sugestão (`suggest`): o Claude não ofereceu nenhuma nas rodadas.
- Convidado com login próprio (`/api/guests`) no terminal: só o convidado de convite.
