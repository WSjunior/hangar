# Cópia do gpui-pre-macos 0.3.7

Origem: crate `gpui-pre-macos` 0.3.7 do crates.io, checksum `5a43af845b260b09393e923c4f1e1a67a10fcfc847b4815192bbfd02ed9fe725`, licença Apache-2.0.

Esta cópia entra no build por `[patch.crates-io]` e usa a cópia modificada de `gpui-pre-apple`.

Tecla física: `physical_digit(key_code)` (`src/events.rs`) mapeia os códigos virtuais `kVK_ANSI_1` a `kVK_ANSI_0` para
`KeyDownEvent::physical_digit`; o `KeyDownEvent` do `do_command_by_selector` (`src/window.rs`) passa `None`. Não
compilado aqui; o CI do `native.yml` compila no macOS.
