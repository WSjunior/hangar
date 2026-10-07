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
/** A tabela de códigos do app (`codes.rs`); fora dela o assunto diz "falha não prevista" e o código vai no corpo. */
const KNOWN_CODES = new Set(["sem-internet", "sem-winget", "sem-sudo", "senha-cancelada", "politica-travada", "checkout-sujo",
	"sem-systemd", "pacotes-desatualizados", "tailscale-https", "tailscale-login", "agente-nao-instalou", "sem-agente",
	"versao-diferente", "modo-desenvolvedor", "roda-do-mouse"]);
/** O e-mail leva só o começo do relatório: o inteiro fica no KV, e e-mail grande é o que um abuso multiplicaria. */
export const MAIL_REPORT_MAX = 8 * 1024;

export type Outcome = keyof typeof SUBJECTS;

export interface Report {
	v: 1; app: string; commit: string; os: string; step: string;
	code: string | null; outcome: Outcome; agent: string | null; report: string;
}

/** O que o handler usa do mundo: os testes trocam limite e e-mail sem tocar na conta. */
export interface Deps {
	reports: KVNamespace;
	allow(ip: string): Promise<boolean>;
	/** Teto de todos juntos: muitos IPs (ou um que troca) não esgotam as escritas do KV nem a caixa de e-mail. */
	allowAll(): Promise<boolean>;
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
	const code = r.code !== null && KNOWN_CODES.has(r.code) ? r.code : "falha não prevista";
	return `[Hangar] ${SUBJECTS[r.outcome]}: ${code} · ${r.os}`;
}

/** As primeiras linhas do texto em até `max` bytes. */
export function head(text: string, max: number): string {
	const bytes = new TextEncoder().encode(text);
	if (bytes.byteLength <= max) return text;
	// Recua até o começo de um caractere: corte no meio vira U+FFFD no e-mail.
	let end = max;
	while (end > 0 && (bytes[end] & 0xc0) === 0x80) end--;
	const cut = new TextDecoder().decode(bytes.slice(0, end));
	const line = cut.lastIndexOf("\n");
	return `${line > 0 ? cut.slice(0, line) : cut}\n…`;
}

export async function handle(request: Request, deps: Deps): Promise<Response> {
	const url = new URL(request.url);
	if (url.pathname !== "/api/relatorio") return json(404, { erro: "nao-encontrado" });
	if (request.method !== "POST") return json(405, { erro: "metodo" });
	if (!STAMP.test(request.headers.get("x-hangar-stamp") ?? "")) return json(403, { erro: "carimbo" });
	if (!(request.headers.get("content-type") ?? "").toLowerCase().startsWith("application/json")) return json(415, { erro: "tipo" });
	if (Number(request.headers.get("content-length") ?? "0") > MAX_BYTES) return json(413, { erro: "grande" });
	// Limite do IP antes de ler o corpo: quem passou dele não custa leitura nem escrita.
	if (!(await deps.allow(request.headers.get("cf-connecting-ip") ?? "sem-ip"))) return json(429, { erro: "limite" });
	const bytes = await readCapped(request.body, MAX_BYTES);
	if (bytes === null) return json(413, { erro: "grande" });
	let parsed: unknown;
	try { parsed = JSON.parse(new TextDecoder().decode(bytes)); } catch { return json(400, { erro: "json" }); }
	const report = parseReport(parsed);
	if (!report) return json(400, { erro: "campos" });
	// O teto de todos só conta relatório válido: lixo com o carimbo público não tira a vez de quem precisa.
	if (!(await deps.allowAll())) return json(429, { erro: "limite" });
	const id = `${new Date().toISOString().slice(0, 10)}/${crypto.randomUUID()}`;
	let stored = true;
	try {
		await deps.reports.put(id, JSON.stringify({ ...report, received: new Date().toISOString(), country: request.cf?.country ?? null }),
			{ expirationTtl: TTL_SECONDS });
	} catch (e: unknown) {
		// O e-mail sai mesmo assim, marcado: vira o único rastro do relatório.
		stored = false;
		console.error(JSON.stringify({ message: "kv falhou", id, error: e instanceof Error ? e.name : typeof e }));
	}
	const text = [stored ? `chave no KV: ${id}` : "não guardado no KV", `resultado: ${report.outcome}`, `código: ${report.code ?? "-"}`, `sistema: ${report.os}`,
		`app: ${report.app} · ${report.commit}`, `etapa: ${report.step}`, `agente: ${report.agent ?? "-"}`, "",
		head(report.report, MAIL_REPORT_MAX)].join("\n");
	// E-mail que falha vai ao log, não desfaz o envio. Só nome e código do erro: a mensagem pode levar o
	// destinatário.
	deps.waitUntil(deps.mail(subject(report), text).catch((e: unknown) => {
		const code = typeof e === "object" && e !== null && "code" in e ? String(e.code) : null;
		console.error(JSON.stringify({ message: "email falhou", id, error: e instanceof Error ? e.name : typeof e, code }));
	}));
	console.log(JSON.stringify({ message: "relatorio", id, stored, outcome: report.outcome, code: report.code }));
	return stored ? json(201, { id }) : json(503, { erro: "armazenamento" });
}

/** `REPORT_TO` é segredo (`wrangler secret put`), fora do `wrangler.jsonc` e por isso fora do `Env` gerado. */
type Bindings = Env & { REPORT_TO?: string };

export default {
	async fetch(request, env: Bindings, ctx): Promise<Response> {
		return handle(request, {
			reports: env.REPORTS,
			allow: async (ip) => (await env.LIMITER.limit({ key: ip })).success,
			allowAll: async () => (await env.GLOBAL_LIMITER.limit({ key: "global" })).success,
			mail: async (subject, text) => {
				if (!env.REPORT_TO) throw new Error("REPORT_TO ausente");
				await env.MAILER.send({ from: FROM, to: env.REPORT_TO, subject, text });
			},
			waitUntil: (work) => ctx.waitUntil(work),
		});
	},
} satisfies ExportedHandler<Bindings>;
