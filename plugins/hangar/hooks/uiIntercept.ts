// Reconhece a chamada que um mod faz para abrir uma URL no navegador do sistema. Clique vindo do
// app abre no aparelho de quem clicou, não na máquina do terminal.

const OPENERS = new Set(["xdg-open", "open", "wslview", "sensible-browser", "x-www-browser", "explorer"]);
const isHttp = (a: string) => /^https?:\/\//i.test(a);

function base(cmd: string): string {
  return (cmd.split(/[\\/]/).pop() ?? "").toLowerCase().replace(/\.exe$/, "");
}

/** A URL http(s) que o `argv` manda abrir, ou null quando é outra coisa. */
export function openerUrl(argv: readonly string[]): string | null {
  const url = argv.find(isHttp) ?? null;
  if (!url) return null;
  const cmd = base(argv[0] ?? "");
  const rest = argv.slice(1).map((a) => a.toLowerCase());
  if (OPENERS.has(cmd)) return url;
  if (cmd === "gio" && rest[0] === "open") return url;
  if (cmd === "cmd" && rest.includes("start")) return url;
  if (cmd === "rundll32" && rest.some((a) => a.startsWith("url.dll,"))) return url;
  if ((cmd === "powershell" || cmd === "pwsh") && rest.includes("start-process")) return url;
  return null;
}
