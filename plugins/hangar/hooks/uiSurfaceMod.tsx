import type { On } from "claude-code";

// Mod de apoio do uiSurface.test.ts: um painel com um botão para cada caminho que o plugin do Hangar
// trata no clique do app pela superfície `desktop` (abrir URL, copiar, avisar).
export const URL_MOD = "https://example.com/hangar";

// O kit de testes roda o `register` de um plugin inline no ambiente dele, sem o escopo deste
// módulo: a URL é repetida aqui dentro, e o teste, que compara com `URL_MOD`, acusa se divergirem.
export function registerOutroMod(on: On): void {
  const url = "https://example.com/hangar";
  on("ui.render", { component: "Pane", requestId: "painel" }, ($, e) => {
    const { Box, Button } = $.ui.resolve(e);
    return (
      <Box flexDirection="column">
        <Button key="abrir" label="Abrir" onPress={() => void $.process.run(["xdg-open", url])} />
        <Button key="copiar" label="Copiar" onPress={(press) => void $.ui.copy({ text: "texto do mod", surface: press.surface })} />
        <Button key="avisar" label="Avisar" onPress={() => $.ui.toast("aviso do mod")} />
      </Box>
    );
  });
}
