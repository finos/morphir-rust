// CI orchestration only. The native CLI owns kit validation and report adjudication.
import { existsSync, mkdirSync, readFileSync, rmSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { installCli, parseReleasePin, releaseTarget } from "./released-cli";

interface KitOptions { readonly root: string; readonly cli: string; readonly kit: string }
type Execute = (args: string[]) => Promise<number>;

export function exitCodeOf(result: { exitCode: number; signalCode: NodeJS.Signals | null }): number {
  if (result.signalCode !== null) throw new Error(`Command terminated by ${result.signalCode}`);
  return result.exitCode;
}

function clearReport(root: string): string {
  const report = path.join(root, ".dev/out/mck/native-report.json");
  mkdirSync(path.dirname(report), { recursive: true });
  rmSync(report, { force: true });
  rmSync(path.join(root, ".dev/out/mck/native-report.html"), { force: true });
  return report;
}

export async function runKit(options: KitOptions, execute: Execute): Promise<void> {
  const { root, cli, kit } = options;
  const report = clearReport(root);
  if (!existsSync(path.join(kit, "mck-kit.lock.json"))) throw new Error(`Expected a managed kit at ${kit}; vendor it with the native CLI first`);
  const requireSuccess = async (args: string[]) => {
    const code = await execute(args);
    if (code !== 0) throw new Error(`${args.join(" ")} exited ${code}`);
  };
  await requireSuccess([cli, "mck", "kit", "status", "--kit", kit]);
  await requireSuccess([cli, "mck", "check", kit]);
  await requireSuccess([cli, "mck", "coverage", "--kit", kit]);
  await requireSuccess([cli, "mck", "schema", "check", "--kit", kit]);
  await requireSuccess(["cargo", "build", "--locked", "-p", "morphir-mck-adapter", "--target-dir", path.join(root, "target")]);
  const adapter = path.join(root, "target/debug", process.platform === "win32" ? "mck-adapter-rust.exe" : "mck-adapter-rust");
  const render = () => requireSuccess([cli, "mck", "report", "render", report, "--format", "html",
    "--output", path.join(root, ".dev/out/mck/native-report.html")]);
  try {
    const code = await execute([cli, "mck", "run", "--kit", kit, "--adapter", adapter, "--report", report]);
    if (code !== 0 && code !== 1) throw new Error(`Native MCK exited ${code}, which is not a case verdict`);
    if (!existsSync(report)) throw new Error(`Native MCK exited ${code} without a fresh report at ${report}`);
    // Exit 1 is not waived here. The shared native checker rejects incomplete
    // sessions, malformed reports and inventory drift before applying the baseline.
    await requireSuccess([cli, "mck", "report", "check", report,
      path.join(root, "crates/morphir-mck-adapter/allowed-failing.json"), "--kit", kit]);
  } catch (error) {
    // Rendering diagnostic evidence cannot replace the original failed verdict.
    if (existsSync(report)) await render().catch((renderError) => console.error(String(renderError)));
    throw error;
  }
  await render();
}

export async function main(root: string): Promise<void> {
  clearReport(root); // Acquisition/configuration failures must remove stale output too.
  const kit = path.resolve(root, process.env.MORPHIR_MCK_KIT ?? "vendor/morphir-mck");
  let cli = process.env.MORPHIR_CLI;
  if (cli) {
    cli = path.resolve(root, cli);
    if (!existsSync(cli)) throw new Error(`MORPHIR_CLI does not exist: ${cli}`);
    console.error(`Using explicit local CLI override: ${cli}`);
  } else {
    const filename = path.join(root, ".config/mck-cli.json");
    if (!existsSync(filename)) throw new Error("Native MCK release pin is not installed. For preparation only, set MORPHIR_CLI and MORPHIR_MCK_KIT to a local CLI and managed kit.");
    const pin = parseReleasePin(JSON.parse(readFileSync(filename, "utf8")));
    const target = releaseTarget();
    cli = await installCli({ root, version: pin.version, target, sha256: pin.sha256[target.triple]! });
  }
  await runKit({ root, cli, kit }, async (args) => {
    console.error(`+ ${args.join(" ")}`);
    const child = Bun.spawn(args, { cwd: root, stdin: "inherit", stdout: "inherit", stderr: "inherit" });
    const exitCode = await child.exited;
    return exitCodeOf({ exitCode, signalCode: child.signalCode });
  });
}

if (import.meta.main) {
  try { await main(path.dirname(path.dirname(fileURLToPath(import.meta.url)))); }
  catch (error) { console.error(String(error)); process.exitCode = 1; }
}
