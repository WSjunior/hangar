import type { On } from "claude-code";

// Mod de apoio do uiHooks.test.ts, no molde do pm-mock: três painéis que o mod fecha de uma vez. O painel
// do meio demora a desenhar uma vez (o relógio de um build andando), então o desenho dele pode cruzar o
// fechamento. Os botões ficam num painel desenhado na superfície `desktop`, que o plugin do Hangar não
// espelha: só os painéis do terminal entram na conta.
export function registerCascata(on: On): void {
  let atrasou = false;
  on("ui.render", { component: "Pane" }, async ($, e) => {
    const { Box, Button, Text } = $.ui.resolve(e);
    if (e.requestId === "controle") {
      return (
        <Box flexDirection="column">
          <Button key="abrir" label="Abrir" onPress={async () => {
            for (const id of ["pm-a", "pm-b", "pm-c", "fora"]) await $.ui.open({ id, title: id });
          }} />
          <Button key="reabrir" label="Reabrir" onPress={async () => {
            await $.ui.open({ id: "pm-c", title: "pm-c" });
            await $.ui.open({ id: "pm-a", title: "pm-a" });
          }} />
          <Button key="fechar-c" label="Fechar C" onPress={() => $.ui.close({ id: "pm-c" })} />
          <Button key="fechar-tres" label="Fechar os três" onPress={async () => {
            await Promise.all(["pm-a", "pm-b", "pm-c"].map((id) => $.ui.close({ id })));
          }} />
        </Box>
      );
    }
    if (e.requestId === "pm-b" && !atrasou) {
      atrasou = true;
      await $.clock.sleep(1000);
    }
    return (
      <Box>
        <Text>{e.requestId}</Text>
      </Box>
    );
  });
}
