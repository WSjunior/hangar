# Crates vendorizados e os ajustes do Hangar

Cada `vendor/<crate>-<versão>` é o crate original do crates.io com os ajustes do Hangar por cima, e entra no build pelo
`[patch.crates-io]` de `desktop-native/Cargo.toml`. O detalhe de cada ajuste está no `MODIFICADO.md` da cópia; o diff
exato contra o original está em `patches/<crate>-<versão>.patch` (aplica com `patch -p0` num diretório que contenha o
original extraído).

## Atualizar para uma versão nova

1. `vendor/rebase.sh bump <crate> <versão-atual> <versão-nova>` para cada crate. O script baixa os dois originais,
   cria a cópia nova e funde cada ajuste por 3 vias. Arquivo que o upstream não tocou entra limpo; onde os dois mexeram
   ficam marcadores `<<<<<<< upstream` / `>>>>>>> ours`.
2. Resolver os conflitos portando a intenção do ajuste para a estrutura nova, não escolhendo um lado. Conferir também os
   arquivos que entraram limpos contra o que o upstream mudou em volta deles.
3. Apontar o `[patch.crates-io]` e a fixação do `gpui-kit` para as versões novas, compilar e adaptar o app.
4. `vendor/rebase.sh export` para regravar os `.patch`, e apagar as cópias antigas.
5. Atualizar a tabela abaixo.

Depois de qualquer mudança à mão num crate vendorizado, rodar `vendor/rebase.sh export`: o `.patch` é o registro do que
fizemos, e só vale se acompanhar a cópia.

## Estado na atualização 0.6.6 → 0.7.0 (gpui-pre 0.3.6 → 0.3.7)

| Ajuste | Onde | Estado | Por quê |
|---|---|---|---|
| Desfoque de fundo (vidro) | gpui-pre `scene.rs`/`window.rs`, gpui-pre-wgpu, -apple, -windows | adaptado no wgpu, mantido nos demais | a 0.3.7 dividiu o renderer wgpu em superfície (`WgpuRenderer`) e dispositivo (`WgpuRendererCore`); buffers, scratch e o alvo copiável foram para o núcleo |
| Redesenho parcial e pular quadro igual | gpui-pre `scene_damage.rs`/`window.rs`/`platform.rs`, gpui-pre-linux, gpui-pre-wgpu | adaptado no wgpu, mantido nos demais | mesma divisão; `render_frame` recebe textura de destino e regiões. O upstream não tem nada equivalente |
| Mapa de elementos das provas, `text_input_focused`, `LineCap` | gpui-pre `window.rs`/`element.rs`/`path_builder.rs` | mantido | upstream não mexeu nesses trechos; duas funções nossas em `window.rs` estavam entre a doc e o `#[inline(always)]` de outra e foram movidas |
| Seleção de texto em view guardada (`retain_cached_view`) | gpui-base `text_selection.rs` | descartado | a 0.7.0 mantém o participante de uma view guardada registrado (`with_rendered_element`, `text_selection.rs:206-245`); o app deixou de chamar o remendo |
| Coluna do marcador de lista, cor do marcador | gpui-base `text/node.rs`/`style.rs` | adaptado | o marcador passou a usar `list_start`, então a lista numerada que começa em outro número (#3204) vale com a coluna ligada |
| Faixa de linguagem do bloco de código | gpui-base `text/node.rs`/`style.rs` | adaptado | o código dentro da faixa usa a estrutura nova do kit (`leaf_key`, `range_backgrounds`, `reveal`) |
| Fonte própria do código inline | gpui-base `text/*`, gpui-component `text/style.rs`/`compat.rs` | mantido | `compat.rs` ficou com o `with_heading` da 0.7.0 mais o repasse da fonte |
| `PopupSurface` (vidro nas listas e menus do kit) | gpui-component `popover.rs`, `menu/popup_menu.rs` | mantido | `dropdown_popup` e `popup_menu.rs` não mudaram na 0.7.0 |
| `PopupMenuAppearance`, `replace_item` | gpui-component `menu/popup_menu.rs`/`mod.rs` | mantido | idem; os consertos de vazamento (#3267, #3279) ficaram em outros arquivos e os acréscimos não guardam entidade |
| Entrada `menu-in` do menu de contexto | gpui-component `menu/context_menu.rs` | adaptado | a 0.7.0 só monta o menu no desenho (`DeferredMenu::build_menu`); a animação foi para lá |
| Fundo pintado e entrada `dialog-in` do diálogo | gpui-component `dialog/dialog.rs` | adaptado | o cartão agora fica no `Positioner::corner`; a subida de 2 px substitui a descida do `slide-down` |

Não compilados aqui: gpui-pre-windows e gpui-pre-apple/-macos (revisados só por leitura).

## Navegador embutido

| Ajuste | Onde | Por quê |
|---|---|---|
| `PaintSurface::texture` e `Window::paint_surface(bounds, texture)` no Linux | gpui-pre `scene.rs`/`window.rs` | a página do WPE WebKit chega como textura GPU externa; o upstream só pinta superfície no macOS (`CVPixelBuffer`) |
| Superfície sempre conta como dano | gpui-pre `scene_damage.rs` (já vinha do PR #62455; teste novo) | a primitiva fica idêntica enquanto os pixels da página mudam; sem isso o quadro seria pulado como igual |
| Dispositivo Vulkan com `VK_EXT_image_drm_format_modifier` e `WgpuContext::shared_device()` | gpui-pre-wgpu `wgpu_context.rs` | importar o DMA-BUF sem cópia exige o modificador DRM e o mesmo dispositivo que o GPUI usa para desenhar |
| `PrimitiveBatch::Surfaces` desenhado como sprite policromático, alpha forçado a 1 | gpui-pre-wgpu `wgpu_renderer.rs`/`shaders.wgsl` | o renderer wgpu ignorava superfícies; em XRGB o byte X não é alpha |
| `CreateTargetForHwnd(hwnd, false)` | gpui-pre-windows `directx_renderer.rs` | com topmost a composição cobre a janela filha do WebView2; o GPUI deixa de desenhar por cima dela |

## Bandeja

| Ajuste | Onde | Por quê |
|---|---|---|
| `PlatformWindow::set_hidden` e `Window::set_hidden` | gpui-pre `platform.rs`/`window.rs`, gpui-pre-linux `wayland/window.rs`/`x11/window.rs`, gpui-pre-windows `window.rs` | o upstream não esconde janela; fechar para a bandeja precisa da janela viva, porque a tela do app não sobrevive a fechar e reabrir |
