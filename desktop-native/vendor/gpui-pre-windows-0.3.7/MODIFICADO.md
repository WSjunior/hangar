# Cópia do gpui-pre-windows 0.3.7

Origem: crate `gpui-pre-windows` 0.3.7 do crates.io, checksum `f05592a6f9e3e6bb7a9f2a0d4779271cf747a948db1c07b02448de777ffff8f3`, licença Apache-2.0.

Desfoque de fundo portado de `zeronsh/zui` na revisão `18a89af`. Arquivos alterados:

- `build.rs`: inclui os shaders de desfoque no build Windows de release.
- `src/directx_renderer.rs`: intercala o desfoque pela ordem da cena, registra seus shaders e libera recursos ociosos ou após resize.
- `src/shaders.hlsl`: adiciona as passadas de desfoque e composição.
- `src/directx_renderer/backdrop.rs`: prepara texturas, shaders e buffers DirectX sob demanda, desenha e recompõe a região desfocada.

O desfoque usa o registrador b2 porque b1 pertence a `BatchParams` nesta versão; ao restaurar o estado do DirectX, preserva b1.
`NOTICE` preserva o aviso de atribuição do zui.

Navegador embutido: `src/directx_renderer.rs` cria o alvo do DirectComposition com `CreateTargetForHwnd(hwnd, false)`
(topmost=false). Com `true` a composição fica por cima das janelas filhas e esconde o WebView2 embutido; com `false` a
página aparece e o vidro da janela continua funcionando (provado no protótipo, numa VM Windows; esta cópia não foi compilada). O custo é que o GPUI não desenha
por cima de janelas filhas.

Não conferido em Windows.

Em 0.3.7 `DirectXAtlas::get_texture_view` passou a devolver `Option`; o desfoque não usa o atlas, então nada mudou aqui.

Bandeja: `PlatformWindow::set_hidden` em `src/window.rs`, com `ShowWindowAsync` (`SW_HIDE`/`SW_SHOW`).
