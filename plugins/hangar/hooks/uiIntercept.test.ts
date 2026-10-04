import { expect, test } from "claude-code/testing";
import { openerUrl } from "./uiIntercept";

test("abridores do sistema com URL http(s)", async () => {
  const url = "https://gitlab.exemplo/mr/1";
  for (const argv of [
    ["xdg-open", url], ["/usr/bin/xdg-open", url], ["open", url], ["wslview", url], ["gio", "open", url],
    ["sensible-browser", url], ["explorer.exe", url], ["cmd", "/c", "start", "", url],
    ["rundll32", "url.dll,FileProtocolHandler", url], ["powershell", "-NoProfile", "Start-Process", url],
  ]) expect(openerUrl(argv)).toBe(url);
});

test("o resto passa", async () => {
  expect(openerUrl(["python3", "script.py", "https://x.exemplo"])).toBeNull();
  expect(openerUrl(["xdg-open", "/home/arquivo.txt"])).toBeNull();
  expect(openerUrl(["xdg-open", "javascript:alert(1)"])).toBeNull();
  expect(openerUrl(["gio", "list", "https://x.exemplo"])).toBeNull();
});
