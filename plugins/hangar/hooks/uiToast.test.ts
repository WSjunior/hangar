import { expect, mock, test, tier } from "claude-code/testing";

// Como na sessão do Hangar: o plugin entra por `--plugin-dir` e fica por fora dos outros mods.
tier("prepend");

test("o aviso de outro mod vai ao backend com o nome dele e segue para o terminal", {
  plugins: [{
    name: "outro-mod",
    register(on) {
      on("prompt.submit", ($, e, next) => {
        $.ui.toast("Jenkins: confirme a emissão no navegador", { timeoutMs: 9000 });
        return next(e);
      });
    },
  }],
}, async ($, on) => {
  // O relógio parado segura o long-poll do input.ts: só o POST do aviso sai.
  mock.clock(on, { now: 1_000_000 });
  const env: Record<string, string> = {
    HANGAR_PLUGIN_URL: "http://127.0.0.1:1/api/plugin", HANGAR_PLUGIN_TOKEN: "tok", CP_SESSION_NAME: "sessao-a",
  };
  const posts: { url: string; body: unknown }[] = [];
  const noTerminal: string[] = [];
  on("env.get", ($, e) => ({ value: env[e.name] }));
  on("session.start", ($, e) => ({ cwd: e.cwd }));
  on("session.cwd", () => ({ value: "/tmp" }));
  on("session.model", () => ({ value: "modelo" }));
  on("http.fetch", ($, e) => {
    posts.push({ url: e.url, body: JSON.parse(e.init?.body as string) });
    return { value: { status: 200, ok: true, headers: {}, text: '{"ok":true}' } } as never;
  });
  on("ui.toast", ($, e) => {
    noTerminal.push(e.text);
    return { value: undefined } as never;
  });
  on("prompt.submit", ($, e) => ({ text: e.text }));

  await $.session.start({ cwd: "/tmp", surface: "terminal", isInteractive: true });
  await $.prompt.submit({ text: "oi", wait: false, origin: { kind: "composer" } });

  expect(noTerminal).toEqual(["Jenkins: confirme a emissão no navegador"]);
  const aviso = posts.find((p) => p.url.endsWith("/toast"));
  expect(aviso?.body).toEqual({
    sessao: "sessao-a", token: "tok",
    text: "Jenkins: confirme a emissão no navegador", timeoutMs: 9000, plugin: "outro-mod",
  });
});
