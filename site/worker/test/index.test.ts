import { env, exports } from "cloudflare:workers";
import { beforeEach, describe, expect, it } from "vitest";
import { handle, MAX_BYTES, TTL_SECONDS, type Deps } from "../src";

function report(over: Record<string, unknown> = {}) {
	return { v: 1, app: "0.20.1.3456", commit: "abc123", os: "linux-x86_64", step: "instalar", code: "sem-systemd",
		outcome: "aberto", agent: null, report: "log limpo", ...over };
}

function post(body: string, headers: Record<string, string> = {}) {
	return new Request("https://hangar.dev.br/api/relatorio", { method: "POST", body,
		headers: { "content-type": "application/json", "x-hangar-stamp": "hangar-native/0.20.1.3456", "cf-connecting-ip": "203.0.113.7", ...headers } });
}

function deps(allow = true, mailFails = false) {
	const mails: { subject: string; text: string }[] = [];
	const pending: Promise<unknown>[] = [];
	const d: Deps = {
		reports: env.REPORTS,
		allow: async () => allow,
		mail: async (subject, text) => { if (mailFails) throw new Error("sem rota de e-mail"); mails.push({ subject, text }); },
		waitUntil: (work) => { pending.push(work); },
	};
	return { d, mails, pending };
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

	it("limits by IP", async () => {
		const { d, mails } = deps(false);
		const res = await handle(post(JSON.stringify(report())), d);
		expect(res.status).toBe(429);
		expect(await stored()).toHaveLength(0);
		expect(mails).toHaveLength(0);
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
