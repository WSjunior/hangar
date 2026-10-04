import { expect, test } from "claude-code/testing";
import { bandBody, type PaneEntry } from "./uiPayload";

const pane = (id: string, size = 10): PaneEntry => ({
  id, title: id, placement: "dock", columns: 72, tree: { type: "Text", children: ["x".repeat(size)] },
});

test("faixa, largura e painéis vão juntos", async () => {
  const body = JSON.parse(`{${bandBody({ type: "Box" }, 87, [pane("review-mr")], 1000)}}`);
  expect(body.above).toEqual({ type: "Box" });
  expect(body.columns).toBe(87);
  expect(body.panes.map((p: PaneEntry) => p.id)).toEqual(["review-mr"]);
});

test("marcador do engine é faixa vazia", async () => {
  expect(JSON.parse(`{${bandBody({ type: "engine", ref: 1 }, 87, [], 1000)}}`).above).toBeNull();
});

test("acima do teto os painéis saem antes da faixa", async () => {
  const body = JSON.parse(`{${bandBody({ type: "Box" }, 87, [pane("grande", 5000)], 300)}}`);
  expect(body.panes).toEqual([]);
  expect(body.above).toEqual({ type: "Box" });
});
