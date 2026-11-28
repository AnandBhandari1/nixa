import type { ExtensionAPI, ExtensionContext } from "@oh-my-pi/pi-coding-agent";
import { existsSync, realpathSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const inNixa = (): boolean =>
	Boolean(process.env.NIXA_TERM_ID && process.env.NIXA_ENGINE_SOCKET && process.env.TERM_PROGRAM === "nixa");

const findNixa = (): string | undefined => {
	if (process.env.NIXA_BIN) return process.env.NIXA_BIN;
	try {
		const root = join(dirname(realpathSync(fileURLToPath(import.meta.url))), "..", "..");
		return [join(root, "..", "MacOS", "nixa"), ...["debug", "release"].map((profile) => join(root, "target", "gpui", profile, "nixa"))]
			.find((path) => existsSync(path));
	} catch {
		return undefined;
	}
};

export default (pi: ExtensionAPI) => {
	if (!inNixa()) return;
	const nixa = findNixa();
	if (!nixa) return;
	let queue: Promise<unknown> = Promise.resolve();
	const run = (args: string[]) => {
		queue = queue.then(() => pi.exec(nixa, args, { timeout: 5000 })).catch(() => undefined);
	};
	const report = (state: string, note?: string) => run(["term", "status", state, ...(note ? [note] : [])]);
	let session = "";
	const resume = (ctx: ExtensionContext) => {
		let id = "";
		try {
			id = ctx.sessionManager.getSessionId() ?? "";
		} catch {
			return;
		}
		if (!id || id === session) return;
		session = id;
		run(["hooks", "session", "omp", "--id", id]);
	};
	pi.on("session_start", async (_event, ctx) => {
		resume(ctx);
		report("idle");
	});
	pi.on("session_switch", async (_event, ctx) => resume(ctx));
	pi.on("input", async (_event, ctx) => {
		resume(ctx);
		report("working");
	});
	pi.on("agent_start", async () => report("working"));
	pi.on("tool_approval_requested", async (event) => report("blocked", `approve ${event.toolName}`));
	pi.on("tool_approval_resolved", async () => report("working"));
	pi.on("plan_review_requested", async () => report("blocked", "plan review"));
	pi.on("plan_review_resolved", async () => report("working"));
	pi.on("agent_end", async () => report("done"));
	pi.on("session_shutdown", async () => report("idle"));
};
