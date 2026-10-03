# Parte 3 — custos e uso: análise (03/10/2026)

Medido nesta máquina (i5-13400F, 16 núcleos, 31 GB, CachyOS) com outras sessões rodando
(ruído de 5–10%). Só leitura: as funções Python rodaram num processo avulso apontando o índice
para uma pasta descartável (`costs_cache._CACHE_DIR` trocado antes de qualquer leitura); o índice
do backend vivo (`~/.claude/.hangar-custos/custos.sqlite3`) e o backend não foram tocados. Branch
`hangar-server-parte3`, base `2c66a347`.

## Conclusão

1. **No dia a dia, custos e uso já não pesam.** A análise inicial mediu a varredura sem índice.
   Hoje o `costs_cache` guarda em SQLite, por arquivo, até onde leu e o estado da leitura, e só
   lê o que cresceu: a atualização de 30 em 30 s custa 0,08–0,14 s, e abrir a tela de custos
   custa 77 ms (`/api/costs`) e 310 ms (`/api/uso`) de ponta a ponta.
2. **O que ainda pesa é a varredura sem índice: 42 s de um núcleo.** Acontece em máquina nova,
   quando alguém sobe a versão de um leitor (`CACHE_VERSAO` do Claude já está em 12) e quando o
   índice corrompe. Enquanto ela roda, as telas mostram "aquecendo". Em Rust ela cai para
   ~3–4 s com 4 núcleos (estimativa sobre protótipo medido; ver abaixo).
3. **Memória:** a varredura sem índice leva o processo de 64 para 163 MB, e ler o uso inteiro
   leva a 211 MB. Dentro do backend esse pico fica: o Python não devolve a memória. O protótipo
   Rust lê tudo com 30–45 MB.
4. **O resto do backend quase não trava.** Durante a varredura sem índice, o laço de eventos
   atrasa no máximo 20 ms (p99 2,7 ms); montar um relatório atrasa até 5–7 ms. O que trava de
   verdade é a fila da própria coleta (`_cache_lock`): "Atualizar dados" espera até 3 s atrás de
   uma varredura e então responde 202.
5. **Motivo para portar agora:** é o módulo mais isolado que sobrou (não fala com tmux nem com
   as CLIs), e a parte 7 exige tirá-lo do Python de qualquer jeito. O ganho de uso é na
   instalação nova, na troca de versão do leitor e na memória do backend, não na tela do dia a
   dia.

## Volume desta máquina

| O quê | Quanto |
|---|---|
| Transcripts do Claude (6 pastas de configuração com `projects/`) | 5,5 GB, 1.190 arquivos, 805 mil linhas |
| Rollouts do Codex (2 contas) | 0,6 GB, 318 arquivos, 147 mil linhas |
| Índice resultante | 33 MB: 1.513 arquivos, 1.898 linhas de custo, 46.655 de uso, 16,6 MB de estado das leituras |
| Maior estado de uma leitura | 192 KB (transcript de 286 MB) |

## Medições do Python

Mediana de 3–4 rodadas onde há mais de uma; "atraso" = quanto uma thread que dorme 1 ms acordou
atrasada (mede quanto o GIL ficou preso).

| Operação | Tempo | CPU | Pico de memória do processo | Atraso máx. / p99 |
|---|---|---|---|---|
| Varredura sem índice (`sincronizar_tudo`) | 41,9 s | 39,8 s | 64 → 163 MB | 20 ms / 2,7 ms |
| Varredura com ~5 min de crescimento | 0,14 s | 0,14 s | 67 MB | 9,5 ms / 2,1 ms |
| Varredura sem mudança | 0,08 s | 0,08 s | — | 3 ms |
| `/api/costs?period=all` (FastAPI, sem relatório pronto) | 77 ms | — | — | 5 ms |
| `/api/costs?period=30d` | 71 ms | — | — | 5 ms |
| `/api/uso?period=all` (primeira do processo / seguintes) | 620 / 310 ms | — | 175 → 211 MB | 5–7 ms |
| Qualquer um com o relatório pronto (mesma versão dos dados) | 3–4 ms | — | — | — |

Detalhe do `/api/uso`: 170 ms lendo 46 mil linhas do SQLite e 140 ms somando; a primeira do
processo paga mais 300 ms varrendo as pastas de skill para saber a origem de cada uma. O
relatório pronto vale até os dados mudarem, e com sessão viva eles mudam a cada atualização de
30 s: na prática quase toda abertura de tela monta de novo.

Tamanho das respostas: `/api/costs` 868 KB (all), 799 KB (30d), 276 KB (7d); `/api/uso` 378 KB.

## Onde vai o tempo da varredura sem índice

`cProfile` da varredura inteira (66 s com o custo do perfil; 42 s sem ele). As proporções valem,
os segundos não.

| Parte | Tempo no perfil | % |
|---|---|---|
| Decodificar JSON (`json.loads`, em C) | 13,7 s | 21% |
| Acumulador de uso do Claude (`uso_claude.Acumulador`: skills, tools, contexto, cargas) | 17,6 s | 27% |
| Leitura de custo do Claude fora do JSON (pré-filtro, datas, dedup por resposta) | ~10 s | 15% |
| Áreas do código (`linhas_de_area`, `fnmatch`, raiz do repositório) | 8,7 s | 13% |
| Codex inteiro (custo + uso) | 8,5 s | 13% |
| SQLite e pickle do estado | ~1 s | 2% |
| Laço de leitura, `dataclasses.replace`, resto | ~6 s | 9% |

Quase 80% é transformação em Python sobre o JSON já decodificado. O JSON em si é C; por isso o
ganho do Rust vem de sumir com a camada Python, não de decodificar mais rápido.

## Protótipo Rust (medido)

`scratchpad`, fora do repositório. Lê os mesmos arquivos com o mesmo pré-filtro de bytes da
`DobraClaude` e decodifica cada linha num struct tipado (serde, campos emprestados), somando o
`usage`. Não tem o acumulador de uso, as áreas nem o SQLite: é o piso. Contou as mesmas 805 mil
linhas e 717 mil decodificações do Python.

| Variante | 1 núcleo | 4 núcleos | 8 núcleos | Pico de memória |
|---|---|---|---|---|
| Linha inteira em `serde_json::Value` | 8,1 s | 3,4 s | 3,9 s | 30 MB (lendo por linha) |
| Struct tipado, conteúdo como `Value` | 6,7 s | 2,1 s | — | 29–45 MB |
| Struct tipado, conteúdo ignorado | 5,7 s | 1,6 s | — | 30–43 MB |

Oito núcleos não ganham de quatro: o disco e a memória viram o limite. Com o acumulador, as
áreas e a gravação, a estimativa é 9–13 s em um núcleo e **3–4 s em quatro**, contra 42 s. O
`Value` inteiro é quase tão lento quanto o `json.loads` do Python; a porta tem de ser tipada.

## O que trava o resto do backend hoje

- **GIL:** pouco. A varredura solta o GIL a cada 5 ms (intervalo padrão); o pior atraso medido
  foi 20 ms.
- **Fila da coleta:** `costs_sources._cache_lock` serializa toda varredura. Com `fresco=1`, o
  pedido espera até 3 s e responde 202 se não conseguiu.
- **Threads:** as rotas são `def` e rodam no pool do anyio (limite 200, o mesmo do chat). Cada
  aparelho que abre a tela ocupa uma thread por 80–310 ms; o 202 ocupa menos.
- **Memória retida:** +100 MB depois da varredura sem índice, +40 MB depois de um `/api/uso`.

## Rotas e quem chama

| Rota | Quem chama | Frequência |
|---|---|---|
| `GET /api/costs?period&fresco` | Web: tela Custos (todos os servidores, soma no cliente) e card da tela inicial; celular: card da tela inicial; nativo: card e tela Custos (servidor ativo) | Ao abrir e ao trocar período; cache de 60 s–5 min no cliente; "Atualizar" manda `fresco=1` |
| `GET /api/uso?period&conta*&projeto*&modelo*&plugin*&foco&fresco` | Web: tela Uso e seção "Por área" da tela Custos; nativo: Uso, clique num item (`foco`) e áreas | Ao abrir e a cada filtro |
| `GET /api/cotacao` | Web (painel lateral, orquestração, ajustes); nativo (aparelho) | No máximo 1×/h |
| `GET /api/sessions/{name}/cost` | Web e nativo, só sessão Codex com o painel aberto | A cada 30 s |
| `GET /api/cotas`, `/api/cotas/sugestao` | Faixa de cotas (web, poll de 60 s), criar sessão (web, celular, nativo), `hangar-send --conta auto` | 60 s enquanto há tela |

202 `{aquecendo, lidos, total}`: web, celular e nativo tentam de novo a cada 3 s, até 100 vezes,
mostrando o progresso. Exceção: o clique num item do Uso no nativo (`stats.rs:308`) não tenta de
novo e mostra "servidor não respondeu". Nenhum backend chama o `/api/costs` de outro: a soma
entre servidores é do cliente web, pelas chaves (`anthropic:<uuid>`, `codex:<home>`, dia,
modelo canônico). As chaves têm de sair iguais.

## O que não entra e por quê

- **Cotas (`cotas.py`, `/api/cotas*`):** é rede (OAuth da Anthropic, Kimi, Codex, opencode,
  commandcode), não CPU. O mesmo cache em memória e em disco serve, dentro do Python, a criação
  de sessão (`conta_com_cota`), o `loop.py`, o MCP e a tela de credenciais. Atender a rota no
  Rust faria dois processos lerem os mesmos provedores com duas esperas de 429 separadas, que é
  exatamente o que a regra "Cota tem cache em disco e respeita 429" proíbe. Fica para a parte 6,
  junto com contas.
- **`stats.py`:** não tem rota. É o evento `stats` do chat, que já vem do Python pela conexão
  interna da parte 1. Vai com estado e prévia (parte 4).
- **Custo por papel da orquestração (`orq_timeline.py`):** lê o índice Python arquivo a arquivo
  (`custos_do_transcript`, `custos_do_rollout`). Continua no Python, com o índice dele.

## Metodologia

- Scripts em `scratchpad/m/` (fora do repositório): `medir.py` (varredura e relatórios com
  `resource.getrusage` e a thread de 1 ms), `perfil.py` (cProfile), `serial.py` (rotas FastAPI
  com `TestClient` e `response_model` reais, cotação fixa, relatório pronto derrubado com
  `costs_cache.mudou()` antes de cada pedido). Protótipo em `scratchpad/proto/`.
- Pastas lidas: as que `config.list_config_dirs()` e `codex_contas.list_accounts()` devolvem
  nesta máquina; Pi tem uma sessão vazia; Kimi tem 2 pastas de sessão.
- A varredura "com crescimento" rodou ~5 min depois da sem índice, com 3 sessões escrevendo.
