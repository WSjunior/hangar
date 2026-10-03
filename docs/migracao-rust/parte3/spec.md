# hangar-server, parte 3 — custos e uso em Rust

**Data:** 2026-10-03
**Status:** aguardando aprovação do dono
**Base:** `hangar-server-parte1` em `2c66a347`; contrato interno da parte 2B = versão 4, esta = **5**
**Medições:** [`analise.md`](analise.md)

## O que muda para quem usa

- Instalação nova, versão nova de um leitor ou índice corrompido: as telas Custos e Uso ficam em
  "aquecendo" por ~3–4 s em vez de ~42 s.
- O backend deixa de carregar 100–150 MB a mais depois de varrer o histórico.
- As telas, os números e as chaves não mudam: web, celular, nativo e a soma entre servidores
  continuam iguais.

## Fora do escopo

- **`/api/cotas` e `/api/cotas/sugestao`** continuam no Python: o cache e a espera de 429 são
  compartilhados com a criação de sessão, o `loop.py`, o MCP e a tela de credenciais. Dois
  processos lendo os provedores seriam duas esperas de 429 separadas. Vão na parte 6.
- **`stats.py`** (evento `stats` do chat): vem pela conexão interna da parte 1; vai na parte 4.
- **Custo por papel da orquestração** (`orq_timeline.py`) continua lendo o índice do Python.
- **Download do catálogo de preços** (`pricing.atualizar_em_background`) continua no Python; o
  Rust só lê o que ele grava.
- Convidados: como na parte 1, pedido sem o token do dono é repassado ao Python.

## Como funciona

### 1. Rotas que o `hangar-server` atende sozinho

Só com o token do dono (mesma regra da parte 1). Todas `GET`:

| Rota | Igual ao Python em |
|---|---|
| `/api/costs?period&fresco` | período inválido vira `all`; 202 `{aquecendo, lidos, total}` enquanto a primeira varredura roda |
| `/api/uso?period&conta*&projeto*&modelo*&plugin*&foco&fresco` | filtros repetíveis; `?conta=` vazio = todas |
| `/api/cotacao` | `{usd_brl}`, cache de 1 h |
| `/api/sessions/{name}/cost` | só Codex; 404 com o mesmo `detail` para sessão que não é Codex ou rollout sumido |

Os demais métodos nessas rotas, e todas as outras rotas, seguem repassados.

**Falha do lado Rust vira repasse, nunca 500 novo.** Índice que não abre, varredura que entra em
pânico, escopos que não chegam do Python: o pedido é repassado ao Python e o log registra o código
do motivo (sem caminho de conversa nem texto). O Python atende como hoje: a primeira chamada a ele
dispara a varredura dele e responde 202 enquanto ela alcança.

### 2. De onde vêm as contas: `GET /internal/costs/scopes` (contrato versão 5)

Rota nova no Python, só loopback com o segredo interno, fora do catálogo, como a `info` da parte
1. Devolve o que o `_sincronizar()` hoje calcula antes de ler arquivo:

```json
{
  "claude": [{"root": "/home/u/.claude/projects", "account": "anthropic:<uuid>", "label": "e-mail ou rótulo"}],
  "codex":  [{"home": "/home/u/.codex", "account": "codex:/home/u/.codex", "label": "Codex · default"}],
  "pi":     [{"root": "/home/u/.pi/agent/sessions", "source": "pi"}],
  "kimi":   {"root": "/home/u/.kimi-code/sessions", "index": "/home/u/.kimi-code/session_index.jsonl"},
  "repo":   "/home/u/hangar"
}
```

- Uma fonte de verdade para contas: `list_config_dirs`, `account_info`, `codex_contas` e as raízes
  do Pi/omp/Kimi continuam só no Python. O omp com a mesma raiz do Pi já sai fora da lista (a
  regra de hoje, com o mesmo aviso no log do Python).
- `repo` é a raiz do checkout: a origem de skill `@repo` depende dela.
- O Rust pede os escopos a cada varredura (como o Python relê hoje) e guarda a última resposta
  boa; sem resposta e sem nenhuma guardada, os pedidos são repassados.
- **Versão:** `RUST_SERVER_PROTOCOL = 5` (Python) e `INTERNAL_PROTOCOL = 5` (Rust), no mesmo
  commit. **Depende da 2B (versão 4) entrar antes**; se a 2B mudar de número, esta vira o seguinte.

A posse de cada rollout do Codex é calculada no Rust com a regra de
`codex_contas.account_for_rollout`: caminho canônico dentro de `<home>/sessions` ou
`<home>/archived_sessions` de exatamente uma conta; ambíguo ou de ninguém fica de fora.

### 3. Índice próprio do Rust

- Arquivo `custos-rust.sqlite3` na mesma pasta do índice do Python
  (`~/.claude/.hangar-custos/`; no Windows `%LOCALAPPDATA%\hangar\custos\`, fora do OneDrive).
  Um não escreve no do outro. O do Python continua servindo a orquestração e a reserva.
- Mesmo desenho do `costs_cache`: tabela `files` (caminho, escopo, versão, dev/ino, tamanho,
  mtime, offset, últimos 64 bytes, estado da leitura) e tabelas `custo` e `uso` com as mesmas
  colunas. Diferença: o estado da leitura é `serde_json` comprimido com `flate2`, não pickle.
- Mesmas regras de retomada: lê só o que cresceu; tamanho menor, outro inode, versão diferente
  ou os 64 bytes antes do offset mudados → relê do zero; linha sem `\n` no fim entra no
  resultado mas não no estado salvo; arquivo que sumiu sai; falha de leitura de um arquivo vira
  log e mantém as linhas anteriores; esquema diferente apaga e refaz.
- `rusqlite` com SQLite embutido (`bundled`), WAL, `synchronous=NORMAL`, gravação em lotes de 1 s.
  Dependência nova; o build compila o SQLite em C nas três plataformas (o `zigbuild` já tem C).

### 4. A varredura

- **Leitores portados, mesmas regras de acumulação:** Claude (`DobraClaude` + `uso_claude` +
  áreas de `uso_areas`), Codex (`RespostasCodex` + `uso_codex`), Pi/omp e Kimi (somas simples,
  ~100 linhas no Python). Pi/omp/Kimi entram para o total não mudar em máquina que tenha esse
  histórico; são só custo, sem uso.
- Decodificação tipada (serde com campos emprestados); `Value` inteiro só onde o Python lê
  conteúdo de tamanho variável. Tamanho de texto em **caracteres**, como o `len()` do Python.
- Arquivos lidos em paralelo num pool `rayon` de `min(4, núcleos)` threads, fora das threads do
  tokio; gravação no SQLite numa thread só, em lote.
- Primeira varredura 30 s depois de subir (o mesmo atraso do Python, para não disputar disco no
  boot). Até ela terminar, as rotas respondem 202 com o progresso (arquivos lidos/total).
- Depois disso, igual ao Python: o pedido lê o índice como está; se a última varredura tem mais
  de 30 s, outra roda atrás; `fresco=1` espera até 3 s por uma varredura e responde 202 se não
  deu. Uma varredura por vez.
- Mapa de áreas (`~/.hangar/uso-areas.json`) lido uma vez por processo, como hoje. Mudou o mapa →
  refaz só as linhas de área a partir dos alvos guardados, sem reler transcript.

### 5. Relatórios e preço

- `costs.montar` e `uso_report.montar` portados com as mesmas ordens de soma e de saída.
  Relatório pronto guardado por (versão dos dados, geração do preço, rótulos, origens de skill,
  mapa de áreas, período, dia, filtros), até 8, como `costs_cache.relatorio`.
- **Preço:** lê `~/.claude/.hangar-pricing/models.dev.json` e `overrides.json`; sem eles, o
  `backend/app/pricing_data.json` embutido no binário. Relê quando o mtime de um deles muda (o
  Python sobe a geração no mesmo momento). Apelidos, prefixos, `IGNORADOS`, contexto longo do
  Codex, modo rápido e cache de 1 h iguais ao `pricing.py`.
- **Cotação:** o Rust busca a própria (`economia.awesomeapi.com.br`, 3 s de prazo, 1 h de cache,
  falha conta como tentativa, vencida devolve a última e busca atrás). Só a primeira do processo
  espera. É uma consulta por hora a mais na máquina; dividir a do Python exigiria outra rota
  interna sem ganho.
- **Origens de skill:** mesma varredura de pastas e a mesma conferência de mtimes de 30 em 30 s.

### 6. O Python com o Rust de pé

- O `rust_server` marca "custos no Rust" quando a saúde responde versão 5. Com a marca, o
  aquecimento de boot do Python (`agendar_aquecimento(30)`) não varre nada.
- `_take_over` (o Python assume a porta) limpa a marca e agenda o aquecimento na hora. O índice do
  Python retoma de onde parou: só o que cresceu desde a última varredura dele.
- Sem binário, `CP_RUST_SERVER=0` ou protocolo diferente: tudo como hoje.

## Como provar

- **Paridade dos leitores (golden):** transcripts sintéticos em
  `backend/tests/fixtures/contract/costs/` cobrindo: blocos da mesma resposta, compactação,
  subagente (Claude e Codex), fork e contador legado do Codex, `token_usage_record`, skill por
  barra, ferramenta, hook, leitura de `SKILL.md` e arquivo de apoio, agente pedido/sozinho, MCP,
  imagem PNG, hook com e sem conteúdo, área por caminho, por comando e por skill, linha truncada,
  linha que não é objeto, modelo ignorado, modo rápido, cache de 1 h. Um gerador Python grava as
  linhas de `custo` e `uso` de cada arquivo; o teste Rust compara (inteiros exatos, números
  fracionários com erro relativo de 1e-9, ordem igual). Nunca conversa real em fixture.
- **Retomada:** cada fixture lida inteira = lida em dois pedaços com o estado salvo no meio.
- **Paridade dos relatórios:** com catálogo de preço fixo, `now` fixo e cotação nula, o gerador
  grava `/api/costs` (all, 7d) e `/api/uso` (all, com filtro de conta, de projeto e com `foco`) a
  partir das linhas golden; o Rust monta os mesmos relatórios e compara.
- **Dados reais, só leitura:** um exemplo do crate (`cargo run --example custos`) varre esta
  máquina num índice descartável e imprime os relatórios; um script compara com o Python avulso
  (o mesmo processo de `analise.md`). Diferença aceita: zero em inteiros e chaves.
- **Reserva:** teste do `rust_server` em que a saúde v5 desliga o aquecimento e o `_take_over`
  religa; teste de rota em que índice ilegível e escopos ausentes viram repasse.
- **Uso real com o dono, no fim:** abrir Custos e Uso no web, no celular (card) e no nativo;
  "Atualizar dados"; filtro e clique num item do Uso; custo de sessão Codex no painel; apagar o
  `custos-rust.sqlite3` e ver o "aquecendo" durar segundos; `CP_RUST_SERVER=0`.
- Testes automatizados rodam quando o dono pedir, como na parte 1.

## Pronto quando

- As quatro rotas saem do `hangar-server` com os golden passando e os três clientes sem mudança.
- A varredura sem índice desta máquina leva menos de 5 s e o pico de memória do `hangar-server`
  durante ela fica abaixo de 100 MB (medidos e anotados em `docs/decisoes/plataforma.md`).
- Os totais desta máquina batem com o Python avulso (inteiros e chaves iguais).
- Sem binário, com versão diferente ou com falha do lado Rust, as telas funcionam como hoje.
