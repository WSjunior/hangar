# Páginas na conversa

O agente (Claude ou Codex) publica uma página HTML que aparece **dentro da conversa**, no lugar da
chamada da ferramenta, logo acima do texto final da resposta. É a versão do Hangar das "visual
replies" do T3 Code (`~/Projetos/t3code`): gráfico, tabela, diagrama, mock de tela, comparação
visual — coisa que em texto fica pior.

Plano: [`docs/superpowers/plans/2026-10-07-paginas-na-conversa.md`](../plans/2026-10-07-paginas-na-conversa.md).

## Decidido com o Jefferson

- **Destinos, nesta ordem:** desktop nativo (`desktop-native/`, GPUI) e PWA (vista mobile do
  `frontend/`); depois o app Expo (`mobile/`). A vista desktop do web e o Electron ficam de fora
  (descontinuação; regra "Desktop evolui no Rust" do `CLAUDE.md`).
- **No nativo a página é viva** dentro da mensagem: script, hover e links funcionam.
  - **Linux primeiro**, reaproveitando o Chromium sem janela já pintado pela GPUI por quadros
    (`desktop-native/src/browser/chromium/`).
  - **Windows** só depois de provar na VM DELPHI-02 que a WebView2 fora da tela continua mandando
    quadros por CDP. Até a prova, Windows usa o mesmo cartão estático do macOS.
  - **macOS:** imagem estática da página + botão "abrir no navegador".
- **Servidor:** a rota nasce no Rust (`crates/hangar-server`). Guarda a página como anexo da
  sessão (some quando a sessão é encerrada), injeta as variáveis de tema, embute as imagens locais
  citadas por caminho absoluto e gera a imagem estática.
- **Tool do agente:** `html_render` no MCP `hangar`, para Claude e Codex. O MCP segue em Python e a
  tool só chama a rota do Rust (1:1).
- **PWA:** página viva num iframe isolado, sem o token na URL do documento.

## O que o T3 ensina e o que copiamos

Lido em `apps/server/src/mcp/toolkits/html/tools.ts`, `packages/shared/src/htmlRender.ts`,
`apps/server/src/htmlRender/HtmlRender.ts` e `apps/web/src/components/chat/HtmlRenderFrame.tsx`.

Copiamos:

1. **A redação da tool**: "o leitor já vê a página; a resposta não anuncia, não diz onde está e
   não repete o que ela mostra". A mesma frase volta no resultado da tool, que é o que o modelo
   relê no turno seguinte.
2. **Regras de layout escritas para o agente**: sem fundo próprio em `html`/`body`, largura
   fluida, sem cartão externo nem título de banner, alturas fixas em pixel para gráfico, nada de
   `100vh`.
3. **Variáveis de tema** em `:root`, com padrão escuro e `@media (prefers-color-scheme: light)`,
   sobrescritas pelo app com os valores reais do tema dele; o CSS da própria página continua
   vencendo.
4. **Protocolo de tamanho e link no formato JSON-RPC do MCP Apps** (`ui/notifications/size-changed`,
   `ui/open-link`, `ui/notifications/host-context-changed`): o mesmo script serve PWA, nativo e
   Expo.
5. **Imagem local embutida só se os primeiros bytes forem de imagem** (PNG, JPEG, GIF, WebP, AVIF,
   SVG): um caminho renomeado ou um link simbólico para segredo não entra na página.
6. **Altura medida no servidor em várias larguras**, para o espaço ser reservado antes do
   primeiro quadro; depois, a altura que a própria página informa vence.
7. **Página nunca é substituída**: cada chamada cria um anexo novo. Republicar é chamar de novo.

Não copiamos: a segunda tool `html_preview`. Aqui o rascunho é um parâmetro da mesma tool e da
mesma rota (ver "Conferir antes de publicar"), o que mantém o 1:1 sem uma tool a mais no catálogo.

## Contrato da tool `html_render`

Entrada:

| campo | tipo | regra |
|---|---|---|
| `html` | string | 1 a 512 000 caracteres; documento único com `<style>` e `<script>` embutidos |
| `title` | string | 1 a 200 caracteres; nome curto (vira o rótulo do cartão e o `<title>`) |
| `height` | inteiro opcional | 80 a 2000; só para limitar a moldura e deixar o resto rolar dentro dela |
| `draft` | booleano opcional | `true` = não publica; mede, tira o print e devolve diagnóstico |

Saída publicada (texto JSON no `tool_result`, que é o que os apps leem):

```json
{"hangar_page": {"id": "<uuid>", "title": "...", "height": null,
                 "heights": {"360": 412, "728": 380, "1000": 380}},
 "message": "Mostrada ao leitor acima da resposta. Não mencione nem descreva a página; responda só o que ela não diz."}
```

Saída de rascunho:

```json
{"draft": {"id": "<uuid>", "url": "http://.../api/sessions/<s>/pages/<id>?token=...",
           "shot": "/home/.../.hangar/paginas/<chave>/<id>.dark.728.png",
           "heights": {...}, "console": [{"level": "error", "text": "..."}],
           "missing_images": ["/abs/x.png"], "browser": "ok"}}
```

`browser` é `ok`, `ausente` (sem Chromium no servidor: sem print, sem altura, sem console) ou
`falhou` (com o motivo). Rascunho não gera cartão na conversa: o resultado não tem `hangar_page`.

A descrição da tool (texto final no plano, Task 5) carrega as regras de página, de layout e de
tema, e as duas formas de conferir.

## Conferir antes de publicar

- **Com o app desktop aberto** (o hook avisa `[hangar] ... navegador embutido`): `draft: true`,
  depois `browser_open` na `url` devolvida e o `browser` do MCP para ver, passar o mouse e clicar.
  É a única forma de conferir comportamento (hover, script).
- **Sem o app**: `draft: true` e ler o PNG de `shot` com a ferramenta de leitura de imagem, mais o
  `console`. **Decisão: Chromium do servidor, não publicar sem conferir.** Motivo: o servidor
  precisa do Chromium de qualquer jeito para a imagem do macOS e para a altura reservada; o print
  sai de graça, e publicar às cegas deixa o erro de layout para o Jefferson achar no celular. Sem
  Chromium no servidor (`browser: "ausente"`), a tool diz isso no rascunho e publicar continua
  permitido — a página só nasce sem altura reservada e sem imagem estática.

## Servidor (Rust, `crates/hangar-server/src/pages/`)

### Armazenamento

- Pasta `~/.hangar/paginas/<chave>/` com `<id>.html` (já com o tema injetado e as imagens
  embutidas), `<id>.json` (`title`, `height`, `heights`, `created`, `draft`) e os prints
  `<id>.<tema>.<largura>.png` gerados sob demanda. Permissão `0700` na pasta.
- `<chave>` é o `session_key` do `info` interno (o mesmo das filas e marcadores): sobrevive a
  renomear a sessão. `id` é UUID v4 gerado no servidor; nome de arquivo nunca vem do agente.
- Limites: 512 000 caracteres de HTML de entrada; 10 MiB por imagem; 25 MiB por página depois de
  embutir. Estourou → erro com código, nada gravado.
- Rascunho vive 1 hora (varrido junto com a limpeza abaixo).

### Some com a sessão

- Cada pasta guarda, num arquivo `jsonl`, o caminho do transcript da sessão no momento da
  publicação (campo `jsonl` do `info` interno).
- Um mecanismo só, a varredura: a cada rodada do produtor da lista (`list/bridge.rs`, ao lado do
  `prune_gone`), pasta cujo `jsonl` não aparece em nenhuma linha viva há 60 s é apagada. Cobre
  fechar pelo app, sessão morta por fora, `/clear` (transcript novo) e backend reiniciado com
  sessões já mortas. Os 60 s evitam apagar por uma rodada em que a linha sumiu (pane ilegível).
- Sem lista aberta o produtor não roda; a varredura também roda uma vez na subida do servidor e
  a cada publicação, para não depender de alguém olhar a lista.
- Mensagem antiga que aponta para página apagada mostra o estado "página expirou" nos apps (404 da
  rota), nunca erro genérico.

### Rotas

| rota | quem | o que faz |
|---|---|---|
| `POST /__hangar_server/pages` (privada, loopback + segredo) | MCP Python | publica ou faz rascunho; corpo `{session, html, title, height?, draft?}` |
| `GET /api/sessions/{name}/pages/{id}` | dono | página isolada (casca com iframe `data:`), para "abrir no navegador" e para o `browser_open` do rascunho |
| `GET /api/sessions/{name}/pages/{id}?raw=1` | dono | HTML cru como `text/plain` + `Content-Security-Policy: sandbox`; apps montam o documento isolado por conta própria |
| `GET /api/sessions/{name}/pages/{id}/shot?theme=dark\|light&width=N` | dono | PNG estático (gera e guarda na primeira vez; `width` arredondada para a largura medida mais próxima) |

- Pedido que não é do dono segue ao Python (`pass`), que recusa como em toda rota do Rust;
  convidado não vê páginas nesta etapa.
- A rota privada nova é contrato interno: `RUST_SERVER_PROTOCOL` (Python) e `INTERNAL_PROTOCOL`
  (Rust) sobem juntos de 36 para 37.
- No modo `python` (Rust fora) não há fallback: a tool responde `erro_paginas_sem_servidor_rust`
  com a frase "páginas precisam do hangar-server de pé". Motivo: regra "feature nova nasce no
  Rust"; duplicar no Python é o que a migração tenta acabar.
- A casca isolada reaproveita o `serve_file` de `workspace_routes.rs` (já tem a casca `data:` com
  `sandbox="allow-scripts allow-popups"` e `Referrer-Policy: no-referrer`), extraída para uma
  função `pub(crate)` que recebe bytes em vez de caminho.

### Tema injetado

Um bloco `<style id="hangar-theme">` e um `<script id="hangar-host">` no começo do `<head>`
(antes de qualquer CSS da página; a busca do `<head>` ignora comentário, `<script>`, `<style>` e
`<template>`). Documento sem `<head>`/`<html>` ganha os dois.

Variáveis (nomes fixos, a página pode usar sem medo):

`--background` (sempre `transparent`), `--foreground`, `--muted-foreground`, `--surface`,
`--border`, `--accent`, `--accent-foreground`, `--danger`, `--warning`, `--success`,
`--code-background`, `--chart-1` … `--chart-4`, `--radius`, `--font-sans`, `--font-mono`.

- O CSS base põe `color-scheme`, `background: var(--background)`, `color`, `font` no `html`,
  `margin: 0` no `body` e esconde a barra de rolagem da página.
- Valores padrão: paleta clássica do Hangar, escura por padrão e clara em
  `prefers-color-scheme: light` (a página baixada continua legível).
- O app sobrescreve com os valores reais do tema dele mandando `ui/notifications/host-context-changed`
  `{theme: "dark"|"light", styles: {variables: {...}}}`. O script reescreve o texto do
  `<style id="hangar-theme">`, de modo que regras `:root` da própria página continuam vencendo.

### Script de ponte (`hangar-host`)

- Tamanho: `ResizeObserver` em `html` e `body` + `DOMContentLoaded` + `load`; manda
  `{"jsonrpc":"2.0","method":"ui/notifications/size-changed","params":{"height":N}}` só quando
  muda. Altura = `scrollHeight` se maior que `clientHeight`, senão `getBoundingClientRect().height`,
  arredondada para cima.
- Link: clique em `<a href="http(s)://...">` é cancelado e vira `ui/open-link {url}`; o app decide.
- Saída: `window.parent.postMessage(msg, "*")` quando há pai (PWA, Expo-web), e
  `window.hangarHost(JSON.stringify(msg))` quando existe essa função (o nativo a instala por
  `Runtime.addBinding`; o Expo, por `window.ReactNativeWebView.postMessage`).
- Entrada: ouve `message` cujo `data.method` é `ui/notifications/host-context-changed`.

### Imagens locais

Caminho absoluto entre aspas (`src="/abs/a.png"`, string JS) ou em `url(/abs/b.webp)` sem aspas vira
`data:<mime>;base64,...`. Confere os primeiros bytes; caminho que não existe ou não é imagem:
erro na publicação, item em `missing_images` no rascunho. URL `http(s)` fica como está.

### Chromium do servidor (`pages/chrome.rs`)

- Binário: `CP_CHROMIUM_BIN`, senão o que a marca `~/.hangar/native/chromium-ok` aponta (Linux,
  instalada pelo `install-chromium.sh`), senão `google-chrome-stable`/`google-chrome`/`chromium`
  no `PATH`, `/Applications/Google Chrome.app/...` (macOS) e o Chrome/Edge padrão do Windows.
- Sobe sem janela com `--remote-debugging-port=0` e perfil temporário próprio, lê a porta no
  `DevToolsActivePort` e fala CDP por WebSocket (`tokio-tungstenite`, já no `Cargo.lock`). Porta
  em vez de pipe porque o pipe por fd 3/4 não existe no Windows. Um processo por publicação,
  morto ao fim; no máximo 2 ao mesmo tempo (semáforo); prazo de 8 s.
- Mede altura nas larguras `360, 728, 1000` no tema escuro (as três que os apps usam: celular,
  coluna de chat padrão, coluna larga). Print do rascunho em 728 escuro; print do macOS sob
  demanda na largura e tema pedidos.
- Bloqueia `file:` e tudo que não for `http(s)`/`data:`/`about:` (`Fetch.enable`), como o
  navegador do nativo.

## Apps

### Reconhecer a página

`tool_use` cujo nome termina em `html_render` e cujo servidor é `hangar` (Claude grava
`mcp__hangar__html_render`; o nome que o Codex grava no rollout é conferido na Task 5 com uma
chamada real) **e** cujo `tool_result` tem JSON com `hangar_page`. Lógica em
`packages/core/src/htmlPage.ts` (PWA e Expo) e espelhada em Rust no nativo. O resto (rascunho,
erro) cai no cartão de ferramenta comum. `agruparConversa` não junta essa chamada a um grupo de
ferramentas (mesma exceção do `Agent`).

### Estados (todos os apps)

| estado | o que aparece |
|---|---|
| carregando | caixa do tamanho reservado (`heights` mais próximo da largura), sem conteúdo, com o título |
| sucesso | a página |
| vazio | não existe: página publicada nunca é vazia; HTML vazio é recusado na rota |
| erro | "Não foi possível carregar «título»" + "tentar de novo" |
| expirou (404) | "Esta página foi apagada quando a sessão foi encerrada" |

Texto de tela em `m.<chave>()` (PWA/Expo) e no catálogo do nativo, `messages/pt.json` e
`messages/en.json` no mesmo commit.

### PWA (`frontend/`, vista mobile)

- `fetch` do `?raw=1` com `Authorization: Bearer` (funciona com qualquer servidor, não depende de
  cookie) e `<iframe sandbox="allow-scripts allow-popups" srcdoc=...>`, com `color-scheme` do
  iframe igual ao do tema (diferente, o navegador pinta fundo opaco atrás da página). Nunca `allow-same-origin`:
  origem opaca, sem acesso ao app, ao `localStorage` nem ao token; o documento não tem URL com
  token. Antes de pôr no `srcdoc`, o app troca o `<style id="hangar-theme">` pelos valores do tema
  atual (lidos das variáveis CSS do app), para não piscar.
- Altura: começa no `heights` reservado; o `size-changed` da página vence; teto 2000 ou o `height`
  do agente. Mudou a altura e a lista estava no fim → rola para o fim (o `MessageList` não tem
  `ResizeObserver`).
- Link: `ui/open-link` só é aceito se `event.source` é este iframe e houve gesto recente
  (`navigator.userActivation.isActive`); abre em nova aba com `noopener`.
- Tema muda → manda `host-context-changed` para cada iframe vivo.
- A lista é janelada (`WINDOW=120`): a página sai do DOM quando a janela anda, e volta recarregada.

### Nativo Linux (página viva)

- Cada cartão de página visível tem um `Engine` próprio (um alvo CDP com janela própria no mesmo
  Chromium compartilhado do painel), num **contexto de navegador separado**
  (`Target.createBrowserContext`): sem cookies nem armazenamento do painel do navegador.
- Carrega com `Page.navigate about:blank` + `Page.setDocumentContent` (sem limite de tamanho de
  URL `data:`), depois `Runtime.addBinding hangarHost`.
- Pinta pelo mesmo `canvas` + `paint_surface` do painel; o tamanho do alvo segue a largura da
  coluna e a altura informada. Altura mudou → `remeasure_items` da lista.
- Fundo transparente de verdade: `Emulation.setDefaultBackgroundColorOverride` com alfa 0 e
  screencast em **PNG** (o JPEG do painel não tem alfa e pintaria um retângulo opaco sobre o vidro
  da conversa). PNG pesa mais na decodificação, mas a página da conversa é pequena e quase sempre
  parada.
- Lista virtualizada: cartão fora da tela para o screencast (`hide`). No máximo 4 páginas vivas;
  além disso a mais antiga fecha o alvo e o cartão mostra o último quadro como imagem; ao voltar,
  recarrega.
- Mouse e hover vão para a página; a roda vai para a conversa, exceto quando a página rola por
  dentro (altura limitada pelo agente ou acima de 2000). Teclado só depois de clicar no cartão;
  Esc devolve o foco à conversa.
- Link (`ui/open-link`) abre no navegador do sistema. Navegação do próprio documento depois da
  carga é bloqueada.
- Tema: `host-context-changed` por `Runtime.evaluate`, com os valores de `theme::colors()`;
  repete quando o tema do app muda.
- Sem Chromium na máquina: cartão estático (como o macOS).

### Nativo macOS (e Windows até a prova)

Cartão com o PNG de `/shot` na largura da coluna e no tema atual, o título e o botão
"abrir no navegador" (`cx.open_url` na casca isolada). Sem Chromium no servidor (404 do `/shot`
com código `erro_pagina_sem_imagem`): só título e botão.

### Nativo Windows (depois da prova)

A prova (Task 9, na DELPHI-02) responde: a WebView2 estacionada fora da tela (x=-3000, a regra
de `docs/decisoes/frontend.md` para manter a composição) manda `Page.screencastFrame` contínuo
por `CallDevToolsProtocolMethod`, e aceita `Input.dispatchMouseEvent`? Sim → a Task 10 liga o
mesmo cartão vivo do Linux sobre a WebView2 fora da tela. Não → Windows fica no cartão estático e
o resultado vai para `docs/decisoes/frontend.md`.

### Expo (`mobile/`, depois)

`react-native-webview` (já instalado, 13.16.1) com `source={{ html }}` vindo do `?raw=1` com o
cabeçalho de token, `originWhitelist={['about:*']}`, `onShouldStartLoadWithRequest` bloqueando
navegação (links por `ui/open-link` → `Linking.openURL`), altura por `onMessage`.

## Fora desta etapa

- Convidado (porta 8766) e par externo vendo páginas.
- Vista desktop do web e Electron.
- Editar/substituir página publicada.
- Pi, omp e Kimi (o MCP `hangar` é de Claude e Codex).

## Critérios de pronto

1. Claude e Codex publicam com `html_render`; a página aparece no lugar da chamada, no nativo
   Linux viva (hover e script funcionando) e no PWA viva.
2. Rascunho sem o app devolve print legível e console; com o app, `browser_open` abre a página.
3. Tema claro/escuro do app aparece na página sem fundo próprio; trocar o tema troca a página.
4. Imagem local por caminho absoluto aparece; arquivo que não é imagem é recusado.
5. Fechar a sessão apaga a pasta; mensagem antiga mostra "expirou".
6. macOS mostra a imagem estática e abre no navegador.
7. Windows: prova feita e registrada; vivo só se a prova passar.
8. Nenhum token na URL do documento da página; página não lê nada do app.
