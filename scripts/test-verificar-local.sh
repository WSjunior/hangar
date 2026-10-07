#!/usr/bin/env bash
# Confere qual passo cada mudança dispara no scripts/verificar-local (a parte com mais ramos dele).
# Usage: ./scripts/test-verificar-local.sh
set -uo pipefail

V="$(dirname "$0")/verificar-local"
falhou=0
caso() {   # $1 = arquivos (um por linha), $2 = linux esperado, $3 = windows esperado
    local saida lin win
    saida="$(printf '%s\n' "$1" | "$V" --classificar)"
    lin="$(sed -n 's/^linux: //p' <<< "$saida")"
    win="$(sed -n 's/^windows: //p' <<< "$saida")"
    if [[ "$lin" != "$2" || "$win" != "$3" ]]; then
        printf 'FALHOU %s\n  linux:   "%s" (esperado "%s")\n  windows: "%s" (esperado "%s")\n' \
            "$(tr '\n' ' ' <<< "$1")" "$lin" "$2" "$win" "$3"
        falhou=1
    fi
}

caso "crates/hangar-server/src/main.rs" "rust backend" "rust"
caso "crates/hangar-api/src/lib.rs" "rust backend nativo" "rust"
caso "backend/app/registry.py" "backend" "runtime"
caso "backend/app/runtime_terminal.py" "rust backend" "rust"
caso "frontend/src/App.svelte" "backend front" ""
caso "packages/core/src/api.ts" "backend front mobile" ""
caso "mobile/src/App.tsx" "backend mobile" ""
caso "desktop-native/src/main.rs" "nativo" ""
caso "install.ps1" "backend" "runtime"
caso "scripts/shell/claude.fish" "backend shell" ""
caso "scripts/hooks/pre-push" "backend shell" ""
caso "scripts/configure-statusline.cjs" "backend statusline" "statusline"
caso ".github/workflows/server.yml" "" ""
caso "docs/migracao-rust/pedidos/x.md" "" ""
caso "docs/decisoes/instalacao.md" "backend" ""

(( falhou )) && exit 1
echo "ok: classificação do verificar-local"
