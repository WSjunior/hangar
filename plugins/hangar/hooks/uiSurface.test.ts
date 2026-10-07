import { expect, mock, test, tier } from "claude-code/testing";
import type { Engine } from "claude-code/testing";
import { registerOutroMod, URL_MOD } from "./uiSurfaceMod";

// Como na sessão do Hangar: o plugin entra por `--plugin-dir` e fica por fora dos outros mods.
tier("prepend");

// O painel do outro mod como o Hangar o recebe pela superfície remota (formato da medição, Task 5).
const montar = ($: Engine, surface: "desktop" | "terminal" = "desktop") => $.ui.mount({
  plugin: "outro-mod", surface, component: "Pane", requestId: "painel",
  props: { title: "painel", isFocused: false, bodyColumns: 58, placement: "dock", scroll: { offset: 0, bodyRows: 40 }, view: {} } as never,
});

test("sem terminal: a URL do clique do app vai ao Hangar; cópia e aviso seguem pelo canal da superfície", {
  plugins: [{ name: "outro-mod", register: registerOutroMod }],
}, async ($, on) => {
  // O relógio parado segura qualquer espera: só os POSTs do clique saem.
  mock.clock(on, { now: 1_000_000 });
  const env: Record<string, string> = {
    HANGAR_PLUGIN_URL: "http://127.0.0.1:1/api/plugin", HANGAR_PLUGIN_TOKEN: "tok", CP_SESSION_NAME: "sessao-a",
  };
  const posts: { url: string; body: Record<string, unknown> }[] = [];
  const rodados: (readonly string[])[] = [];
  const copias: string[] = [];
  const avisos: string[] = [];
  on("env.get", ($, e) => ({ value: env[e.name] }));
  on("session.start", ($, e) => ({ cwd: e.cwd }));
  on("session.cwd", () => ({ value: "/tmp" }));
  on("session.model", () => ({ value: "modelo" }));
  on("http.fetch", ($, e) => {
    posts.push({ url: e.url, body: JSON.parse(e.init?.body as string) });
    const text = e.url.endsWith("/press-start") ? '{"fromApp":true,"attempt":"t-1"}' : '{"ok":true}';
    return { value: { status: 200, ok: true, headers: {}, text } } as never;
  });
  on("process.run", ($, e) => {
    rodados.push(e.argv);
    return { value: { exitCode: 0, stdout: "", stderr: "" } } as never;
  });
  on("ui.copy", ($, e) => {
    copias.push(e.text);
    return { value: { isCopied: true } } as never;
  });
  on("ui.toast", ($, e) => {
    avisos.push(e.text);
    return { value: undefined } as never;
  });

  await $.session.start({ cwd: "/tmp", surface: "terminal", isInteractive: false });
  const ui = await montar($);
  await ui.press({ key: "abrir" });
  await ui.press({ key: "copiar" });
  await ui.press({ key: "avisar" });
  await ui.unmount();

  // A URL não roda no servidor: vai ao `opened`, que a devolve ao aparelho de quem clicou.
  expect(rodados).toEqual([]);
  expect(posts.filter((p) => p.url.endsWith("/opened")).map((p) => p.body)).toEqual([{ sessao: "sessao-a", token: "tok", attempt: "t-1", url: URL_MOD }]);
  expect(posts.filter((p) => p.url.endsWith("/press-start")).map((p) => p.body)).toEqual(["abrir", "copiar", "avisar"].map((element) => (
    { sessao: "sessao-a", token: "tok", requestId: "painel", plugin: "outro-mod", element })));
  // Cópia e aviso seguem ao engine (que os manda ao Hangar pelo canal da superfície), uma vez cada.
  expect(copias).toEqual(["texto do mod"]);
  expect(avisos).toEqual(["aviso do mod"]);
  // Nada pela ponte do terminal (`/toast`, `/copied`, `/pressed`, `/state`, `/ui`): no `-p` ela é nula.
  expect(posts.filter((p) => !p.url.endsWith("/press-start") && !p.url.endsWith("/opened")).map((p) => p.url)).toEqual([]);
});

test("sem as variáveis da ponte o `claude -p` abre a URL onde roda, sem POST", {
  plugins: [{ name: "outro-mod", register: registerOutroMod }],
}, async ($, on) => {
  mock.clock(on, { now: 1_000_000 });
  const posts: string[] = [];
  const rodados: (readonly string[])[] = [];
  on("env.get", () => ({ value: undefined }));
  on("session.start", ($, e) => ({ cwd: e.cwd }));
  on("session.cwd", () => ({ value: "/tmp" }));
  on("session.model", () => ({ value: "modelo" }));
  on("http.fetch", ($, e) => {
    posts.push(e.url);
    return { value: { status: 200, ok: true, headers: {}, text: "{}" } } as never;
  });
  on("process.run", ($, e) => {
    rodados.push(e.argv);
    return { value: { exitCode: 0, stdout: "", stderr: "" } } as never;
  });

  await $.session.start({ cwd: "/tmp", surface: "terminal", isInteractive: false });
  const ui = await montar($);
  await ui.press({ key: "abrir" });
  await ui.unmount();

  expect(rodados).toEqual([["xdg-open", URL_MOD]]);
  expect(posts).toEqual([]);
});

test("no terminal a cópia do clique do app vai ao Hangar pela ponte, sem passar pelo engine", {
  plugins: [{ name: "outro-mod", register: registerOutroMod }],
}, async ($, on) => {
  // O relógio parado segura o long-poll do input.ts e o envio da faixa: só os POSTs do clique saem.
  mock.clock(on, { now: 1_000_000 });
  const env: Record<string, string> = {
    HANGAR_PLUGIN_URL: "http://127.0.0.1:1/api/plugin", HANGAR_PLUGIN_TOKEN: "tok", CP_SESSION_NAME: "sessao-a",
  };
  const posts: { url: string; body: Record<string, unknown> }[] = [];
  const copias: string[] = [];
  on("env.get", ($, e) => ({ value: env[e.name] }));
  on("session.start", ($, e) => ({ cwd: e.cwd }));
  on("session.cwd", () => ({ value: "/tmp" }));
  on("session.model", () => ({ value: "modelo" }));
  on("http.fetch", ($, e) => {
    posts.push({ url: e.url, body: JSON.parse(e.init?.body as string) });
    const text = e.url.endsWith("/press-start") ? '{"fromApp":true,"attempt":"t-1"}' : '{"ok":true}';
    return { value: { status: 200, ok: true, headers: {}, text } } as never;
  });
  on("ui.copy", ($, e) => {
    copias.push(e.text);
    return { value: { isCopied: true } } as never;
  });

  await $.session.start({ cwd: "/tmp", surface: "terminal", isInteractive: true });
  const ui = await montar($, "terminal");
  await ui.press({ key: "copiar" });
  await ui.unmount();

  // O aparelho de quem clicou recebe a cópia pela ponte do terminal, uma vez; o engine não copia no servidor.
  expect(posts.filter((p) => p.url.endsWith("/copied")).map((p) => p.body)).toEqual([
    { sessao: "sessao-a", token: "tok", attempt: "t-1", text: "texto do mod" }]);
  expect(copias).toEqual([]);
});
