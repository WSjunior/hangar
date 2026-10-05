# Cópia modificada do gpui-pre 0.3.7

Origem: crate `gpui-pre` 0.3.7 do crates.io, licença Apache-2.0 (`LICENSE-APACHE`).
Entra no build por `[patch.crates-io]` em `desktop-native/Cargo.toml`.

`src/scene.rs` e `src/window.rs` foram modificados pelo Hangar para receber regiões de desfoque do conteúdo já pintado. O mecanismo de ordem, repetição de cenas guardadas e a entrada de pintura foram portados de `zeronsh/zui` na revisão `18a89af`, também Apache-2.0. Cada arquivo alterado traz um aviso no cabeçalho.

O renderer usa essas regiões em uma etapa posterior. Sem suporte no renderer, `paint_backdrop_blur` não muda os pixels; quem chama deve manter um preenchimento translúcido como reserva.

`src/scene_damage.rs` (novo), `src/window.rs`, `src/platform.rs` e `src/scene.rs` foram portados do PR #62455 de
zed-industries/zed (`d9c29a3`, Apache-2.0): a janela compara a cena nova com a anterior, pula a apresentação quando
nada mudou e entrega a região mudada ao renderer (`PlatformWindow::draw_with_damage`). Os desfoques entram na
comparação. Ligado por padrão; `GPUI_EXPERIMENTAL_PRESENT_SKIP=0`, `GPUI_EXPERIMENTAL_PARTIAL_RENDER=0` e
`GPUI_EXPERIMENTAL_ORDER_TOLERANT_DAMAGE=0` desligam cada parte. `src/path_builder.rs` reexporta `LineCap`.
`Window::text_input_focused` expõe se o foco está num campo de texto, para teclas como Home/End decidirem se são dele.

`src/window.rs` e `src/element.rs` também guardam, só enquanto houver um receptor instalado por `Window::set_element_map_sink`, os elementos com id do quadro (`ElementRecord`: caminho de ids, bounds lógicos, visível). Views em cache copiam a faixa junto com os hitboxes. Sem receptor nada é registrado. O app usa isso no mapa de elementos das provas (`HANGAR_NATIVE_UI_MAP`).

Na passagem para 0.3.7 as funções `present_skip_enabled`/`partial_render_enabled` foram postas antes da doc e do
`#[inline(always)]` de `with_element_arena`; antes elas ficavam no meio e roubavam o atributo e a doc dessa função.

Navegador embutido (Linux): `PaintSurface` ganhou `texture` (`Arc<dyn Any + Send + Sync>`, um `wgpu::Texture` no
renderer wgpu) e `Window::paint_surface(bounds, texture)` pinta essa textura externa na fase de pintura, sob a máscara
de conteúdo atual. O macOS mantém o `image_buffer` de antes. Os pixels da superfície mudam com a primitiva idêntica,
e a comparação de cenas já trata toda superfície como dano nos dois quadros (`accumulate_surface_damage` em
`scene_damage.rs`), então um quadro com superfície nunca é pulado; o teste `identical_surface_frames_are_still_damaged`
trava isso.

Bandeja: `PlatformWindow::set_hidden` (padrão sem efeito) e `Window::set_hidden(hidden)` escondem a janela sem
destruí-la e a mostram de novo, para a opção "Manter na bandeja ao fechar". O upstream não tem como esconder uma janela;
as implementações ficam nas cópias de gpui-pre-linux e gpui-pre-windows.
