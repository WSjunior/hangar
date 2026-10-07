/**
 * Relatórios de falha do assistente de instalação (spec "Worker hangar.dev.br"): guarda no KV por 90 dias e avisa por
 * e-mail. O carimbo barra robô genérico, não alguém determinado: o app é código aberto.
 */
export const MAX_BYTES = 256 * 1024;
export const TTL_SECONDS = 90 * 24 * 60 * 60;
const STAMP = /^hangar-native\/[\w.+-]{1,64}$/;
const CODE = /^[a-z0-9-]{1,64}$/;
const SHORT = /^[\w .:+()/·-]{0,200}$/;
const SUBJECTS = { consertado: "consertado ali", aberto: "aberto" } as const;
const FROM = "relatorio@hangar.dev.br";

export type Outcome = keyof typeof SUBJECTS;

export interface Report {
	v: 1; app: string; commit: string; os: string; step: string;
	code: string | null; outcome: Outcome; agent: string | null; report: string;
}

/** O que o handler usa do mundo: os testes trocam limite e e-mail sem tocar na conta. */
export interface Deps {
	reports: KVNamespace;
	allow(ip: string): Promise<boolean>;
	mail(subject: string, text: string): Promise<void>;
	waitUntil(work: Promise<unknown>): void;
}

function json(status: number, body: Record<string, unknown>): Response {
	return new Response(JSON.stringify(body), { status, headers: { "content-type": "application/json" } });
}

/** Lê o corpo parando no limite: corpo enorme nunca fica inteiro na memória. */
async function readCapped(body: ReadableStream<Uint8Array> | null, max: number): Promise<Uint8Array | null> {
	if (!body) return new Uint8Array();
	const reader = body.getReader();
	const chunks: Uint8Array[] = [];
	let total = 0;
	for (;;) {
		const { done, value } = await reader.read();
		if (done) break;
		total += value.byteLength;
		if (total > max) { await reader.cancel(); return null; }
		chunks.push(value);
	}
	const out = new Uint8Array(total);
	let at = 0;
	for (const chunk of chunks) { out.set(chunk, at); at += chunk.byteLength; }
	return out;
}

export function parseReport(value: unknown): Report | null {
	if (typeof value !== "object" || value === null) return null;
	const { v, app, commit, os, step, code, outcome, agent, report } = value as Record<string, unknown>;
	const short = (x: unknown): x is string => typeof x === "string" && SHORT.test(x);
	const codeLike = (x: unknown): x is string | null => x === null || (typeof x === "string" && CODE.test(x));
	if (v !== 1 || !short(app) || app === "" || !short(commit) || !short(os) || !short(step)) return null;
	if (!codeLike(code) || !codeLike(agent)) return null;
	if (outcome !== "consertado" && outcome !== "aberto") return null;
	if (typeof report !== "string" || report === "") return null;
	return { v, app, commit, os, step, code, outcome, agent, report };
}

export function subject(r: Report): string {
	return `[Hangar] ${SUBJECTS[r.outcome]}: ${r.code ?? "falha não prevista"} · ${r.os}`;
}

export async function handle(request: Request, deps: Deps): Promise<Response> {
	const url = new URL(request.url);
	if (url.pathname !== "/api/relatorio") return json(404, { erro: "nao-encontrado" });
	if (request.method !== "POST") return json(405, { erro: "metodo" });
	if (!STAMP.test(request.headers.get("x-hangar-stamp") ?? "")) return json(403, { erro: "carimbo" });
	if (!(request.headers.get("content-type") ?? "").startsWith("application/json")) return json(415, { erro: "tipo" });
	if (Number(request.headers.get("content-length") ?? "0") > MAX_BYTES) return json(413, { erro: "grande" });
	// Limite antes de ler o corpo: quem passou do limite não custa leitura nem escrita.
	if (!(await deps.allow(request.headers.get("cf-connecting-ip") ?? "sem-ip"))) return json(429, { erro: "limite" });
	const bytes = await readCapped(request.body, MAX_BYTES);
	if (bytes === null) return json(413, { erro: "grande" });
	let parsed: unknown;
	try { parsed = JSON.parse(new TextDecoder().decode(bytes)); } catch { return json(400, { erro: "json" }); }
	const report = parseReport(parsed);
	if (!report) return json(400, { erro: "campos" });
	const id = `${new Date().toISOString().slice(0, 10)}/${crypto.randomUUID()}`;
	await deps.reports.put(id, JSON.stringify({ ...report, received: new Date().toISOString(), country: request.cf?.country ?? null }),
		{ expirationTtl: TTL_SECONDS });
	const text = [`id: ${id}`, `app: ${report.app} · ${report.commit}`, `sistema: ${report.os}`, `etapa: ${report.step}`,
		`código: ${report.code ?? "-"}`, `agente: ${report.agent ?? "-"}`, "", report.report].join("\n");
	// Já guardado: e-mail que falha vai ao log, não desfaz o envio.
	deps.waitUntil(deps.mail(subject(report), text).catch((e: unknown) =>
		console.error(JSON.stringify({ message: "email falhou", id, error: e instanceof Error ? e.message : String(e) }))));
	console.log(JSON.stringify({ message: "relatorio", id, outcome: report.outcome, code: report.code }));
	return json(201, { id });
}

/** `REPORT_TO` é segredo (`wrangler secret put`), fora do `wrangler.jsonc` e por isso fora do `Env` gerado. */
type Bindings = Env & { REPORT_TO?: string };

export default {
	async fetch(request, env: Bindings, ctx): Promise<Response> {
		return handle(request, {
			reports: env.REPORTS,
			allow: async (ip) => (await env.LIMITER.limit({ key: ip })).success,
			mail: async (subject, text) => {
				if (!env.REPORT_TO) throw new Error("REPORT_TO ausente");
				await env.MAILER.send({ from: FROM, to: env.REPORT_TO, subject, text });
			},
			waitUntil: (work) => ctx.waitUntil(work),
		});
	},
} satisfies ExportedHandler<Bindings>;
