import { expect, test } from "claude-code/testing";
import { focusElement, holding, HOLD_MS } from "./uiFocus";

test("a key armada só entra em movimento que já ia para um elemento de mod", async () => {
  const alvo = { attempt: "t-1", rewrite: "sec-anx" };
  expect(focusElement({ plugin: "pm-mock", element: "sec-desc" }, alvo)).toBe("sec-anx");
  // Parada do motor (título de aba, `✕`): sem plugin nem elemento, segue como veio ((r)).
  expect(focusElement({}, alvo)).toBeUndefined();
  expect(focusElement({ plugin: "pm-mock", element: "sec-desc" }, { attempt: "t-1", rewrite: null })).toBe("sec-desc");
  expect(focusElement({ plugin: "pm-mock", element: "sec-desc" }, null)).toBe("sec-desc");
});

test("a guarda do envio vale até o prazo", async () => {
  expect(holding(1000, null)).toBe(false);
  expect(holding(1000, 1000 + HOLD_MS)).toBe(true);
  expect(holding(1000 + HOLD_MS, 1000 + HOLD_MS)).toBe(false);
});
