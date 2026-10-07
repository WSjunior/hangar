import { env, exports } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { handle, head, KV_DAILY_MAX, MAIL_REPORT_MAX, MAX_BYTES, TTL_SECONDS, type Deps } from "../src";

function report(over: Record<string, unknown> = {}) {
	return { v: 1, app: "0.20.1.3456", commit: "abc123", os: "linux-x86_64", step: "instalar", code: "sem-systemd",
		outcome: "aberto", agent: null, report: "log limpo", ...over };
}

function post(body: string, headers: Record<string, string> = {}) {
	return new Request("https://hangar.dev.br/api/relatorio", { method: "POST", body,
		headers: { "content-type": "application/json", "x-hangar-stamp": "hangar-native/0.20.1.3456", "cf-connecting-ip": "203.0.113.7", ...headers } });
}

function deps(allow = true, mailFails = false, allowAll = true, today = 1) {
	const mails: { subject: string; text: string }[] = [];
	const pending: Promise<unknown>[] = [];
	const calls = { all: 0 };
	const d: Deps = {
		reports: env.REPORTS,
		allow: async () => allow,
		allowAll: async () => { calls.all++; return allowAll; },
		countToday: async () => today,
		mail: async (subject, text) => { if (mailFails) throw new Error("sem rota de e-mail"); mails.push({ subject, text }); },
		waitUntil: (work) => { pending.push(work); },
	};
	return { d, mails, pending, calls };
}

async function stored() { return (await env.REPORTS.list()).keys; }

beforeEach(async () => { for (const key of await stored()) await env.REPORTS.delete(key.name); });

describe("POST /api/relatorio", () => {
	it("stores for 90 days and mails 'aberto'", async () => {
		const { d, mails, pending } = deps();
		const before = Math.floor(Date.now() / 1000);
		const res = await handle(post(JSON.stringify(report())), d);
		expect(res.status).toBe(201);
		await Promise.all(pending);
		const keys = await stored();
		expect(keys).toHaveLength(1);
		expect(keys[0].expiration).toBeGreaterThanOrEqual(before + TTL_SECONDS - 5);
		expect(keys[0].expiration).toBeLessThanOrEqual(before + TTL_SECONDS + 60);
		expect(mails[0].subject).toContain("aberto");
		expect(mails[0].subject).toContain("sem-systemd");
		expect(mails[0].text).toContain("log limpo");
		expect(JSON.parse((await env.REPORTS.get(keys[0].name))!).report).toBe("log limpo");
	});

	it("over the daily KV ceiling only mails, marked as not stored", async () => {
		const { d, mails, pending } = deps(true, false, true, KV_DAILY_MAX + 1);
		const res = await handle(post(JSON.stringify(report())), d);
		expect(res.status).toBe(201);
		await Promise.all(pending);
		expect(await stored()).toHaveLength(0);
		expect(mails[0].text).toContain("não guardado no KV");
	});

	it("says 'consertado ali' when the agent fixed it", async () => {
		const { d, mails, pending } = deps();
		await handle(post(JSON.stringify(report({ outcome: "consertado", agent: "claude" }))), d);
		await Promise.all(pending);
		expect(mails[0].subject).toContain("consertado ali");
	});

	it("rejects bodies over 256 KB without storing", async () => {
		const { d } = deps();
		const res = await handle(post(JSON.stringify(report({ report: "x".repeat(MAX_BYTES) }))), d);
		expect(res.status).toBe(413);
		expect(await stored()).toHaveLength(0);
	});

	it("limits by IP before touching the global limit", async () => {
		const { d, mails, calls } = deps(false);
		const res = await handle(post(JSON.stringify(report())), d);
		expect(res.status).toBe(429);
		expect(await stored()).toHaveLength(0);
		expect(mails).toHaveLength(0);
		expect(calls.all).toBe(0);
	});

	it("limits everyone together", async () => {
		const { d, mails, pending } = deps(true, false, false);
		const res = await handle(post(JSON.stringify(report())), d);
		await Promise.all(pending);
		expect(res.status).toBe(429);
		expect(await stored()).toHaveLength(0);
		expect(mails).toHaveLength(0);
	});

	it("charges the global limit only for valid reports", async () => {
		const { d, calls } = deps();
		expect((await handle(post("{"), d)).status).toBe(400);
		expect((await handle(post(JSON.stringify(report({ outcome: "talvez" }))), d)).status).toBe(400);
		expect(calls.all).toBe(0);
		expect((await handle(post(JSON.stringify(report())), d)).status).toBe(201);
		expect(calls.all).toBe(1);
	});

	it("mails the report marked as not stored when KV fails", async () => {
		const { d, mails, pending } = deps();
		d.reports = { put: async () => { throw new Error("kv fora"); } } as unknown as KVNamespace;
		const res = await handle(post(JSON.stringify(report())), d);
		await Promise.all(pending);
		expect(res.status).toBe(503);
		expect(await res.json()).toEqual({ erro: "armazenamento" });
		expect(mails[0].text).toContain("não guardado no KV");
		expect(mails[0].text).not.toContain("chave no KV");
		expect(mails[0].text).toContain("log limpo");
	});

	it("never cuts a UTF-8 character in half", () => {
		// "ç" ocupa 2 bytes: um corte em 3 cairia no meio do segundo.
		expect(head("çççç", 3)).toBe("ç\n…");
		expect(head("ação\nfim", 4)).not.toContain("\ufffd");
	});

	it("mails a short summary and keeps the whole report in KV", async () => {
		const { d, mails, pending } = deps();
		const big = Array.from({ length: 20_000 }, (_, n) => `linha ${n}`).join("\n");
		const res = await handle(post(JSON.stringify(report({ report: big }))), d);
		await Promise.all(pending);
		expect(res.status).toBe(201);
		const { id } = await res.json<{ id: string }>();
		const text = mails[0].text;
		for (const line of [`chave no KV: ${id}`, "resultado: aberto", "código: sem-systemd", "sistema: linux-x86_64", "app: 0.20.1.3456"]) {
			expect(text).toContain(line);
		}
		expect(text).toContain("linha 0\n");
		expect(text).not.toContain("linha 19999");
		expect(new TextEncoder().encode(text).byteLength).toBeLessThanOrEqual(MAIL_REPORT_MAX + 512);
		expect(JSON.parse((await env.REPORTS.get(id))!).report).toBe(big);
	});

	it("puts an unknown code in the body, not in the subject", async () => {
		const { d, mails, pending } = deps();
		await handle(post(JSON.stringify(report({ code: "disco-cheio" }))), d);
		await Promise.all(pending);
		expect(mails[0].subject).toContain("falha não prevista");
		expect(mails[0].subject).not.toContain("disco-cheio");
		expect(mails[0].text).toContain("código: disco-cheio");
	});

	it("accepts the content type in any case", async () => {
		const { d } = deps();
		expect((await handle(post(JSON.stringify(report()), { "content-type": "Application/JSON; charset=UTF-8" }), d)).status).toBe(201);
	});

	it("needs the app stamp, JSON and POST on the right path", async () => {
		const { d } = deps();
		expect((await handle(post(JSON.stringify(report()), { "x-hangar-stamp": "curl/8" }), d)).status).toBe(403);
		expect((await handle(post(JSON.stringify(report()), { "content-type": "text/plain" }), d)).status).toBe(415);
		expect((await handle(new Request("https://hangar.dev.br/api/relatorio"), d)).status).toBe(405);
		expect((await handle(new Request("https://hangar.dev.br/api/outro", { method: "POST" }), d)).status).toBe(404);
	});

	it("rejects broken JSON and fields out of contract", async () => {
		const { d } = deps();
		expect((await handle(post("{"), d)).status).toBe(400);
		expect((await handle(post(JSON.stringify(report({ outcome: "talvez" }))), d)).status).toBe(400);
		expect((await handle(post(JSON.stringify(report({ code: "Disco Cheio" }))), d)).status).toBe(400);
		expect((await handle(post(JSON.stringify(report({ report: "" }))), d)).status).toBe(400);
		expect(await stored()).toHaveLength(0);
	});

	it("keeps the report when the e-mail fails", async () => {
		const { d, pending } = deps(true, true);
		const res = await handle(post(JSON.stringify(report())), d);
		await Promise.all(pending);
		expect(res.status).toBe(201);
		expect(await stored()).toHaveLength(1);
	});

	it("wires the real bindings", async () => {
		const res = await exports.default.fetch(post(JSON.stringify(report())));
		expect(res.status).toBe(201);
	});
});
