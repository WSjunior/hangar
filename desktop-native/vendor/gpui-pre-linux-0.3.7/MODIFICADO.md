# Cópia modificada do gpui-pre-linux 0.3.7

Origem: crate `gpui-pre-linux` 0.3.7 do crates.io, licença Apache-2.0 (`LICENSE-APACHE`).
Entra no build por `[patch.crates-io]` em `desktop-native/Cargo.toml`.

`src/linux/wayland/window.rs` e `src/linux/x11/window.rs` implementam `PlatformWindow::draw_with_damage`, que repassa
ao renderer wgpu a região da cena que mudou desde o último quadro. Portado do PR #62455 de zed-industries/zed
(revisão `d9c29a3`), Apache-2.0. Cada arquivo alterado traz um aviso no cabeçalho.

Bandeja: `PlatformWindow::set_hidden` nos dois backends. No Wayland (`src/linux/wayland/window.rs`) esconder destrói
o toplevel e o `xdg_surface`, tira o buffer da `wl_surface` e cria os dois de novo nela, sem commit; mostrar reaplica
título, `app_id`, tamanhos e modo de decoração e faz o commit inicial, que traz o configure como na criação da janela.
A superfície e o renderer são os mesmos do começo ao fim, e o laço de quadros fica em `Unconfigured` enquanto a janela
está escondida. Só desmapear com buffer nulo não serve: o Hyprland não manda configure no commit seguinte. No X11
(`src/linux/x11/window.rs`) é `UnmapWindow`/`MapWindow`.
