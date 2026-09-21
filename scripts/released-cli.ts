// Shared acquisition for MCK CI and the released-CLI extension checks.
// No compatibility decisions live here; the native CLI owns those checks.
import { createHash } from "node:crypto";
import { chmodSync, existsSync, mkdirSync, mkdtempSync, readFileSync, renameSync, rmSync, writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { unzipSync } from "fflate";

export interface ReleaseTarget {
  readonly triple: string;
  readonly executable: "morphir" | "morphir.exe";
  readonly format: "tgz" | "zip";
}
interface InstallOptions {
  readonly root: string;
  readonly version: string;
  readonly target?: ReleaseTarget;
  readonly sha256?: string;
  readonly fetchBytes?: (url: string) => Promise<Uint8Array>;
}
interface ReleasePin { readonly version: string; readonly sha256: Readonly<Record<string, string>> }
const digest = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const isDigest = (value: unknown): value is string => typeof value === "string" && /^[a-f0-9]{64}$/.test(value);

function checkVersion(value: unknown): asserts value is string {
  if (typeof value !== "string" || !/^\d+\.\d+\.\d+(?:-[0-9A-Za-z.-]+)?$/.test(value)) {
    throw new Error("CLI version must be an exact release version");
  }
}

export function releaseTarget(os = process.platform as string, arch = process.arch as string): ReleaseTarget {
  const platform = { darwin: "apple-darwin", linux: "unknown-linux-gnu", win32: "pc-windows-msvc" }[os];
  const architecture = { x64: "x86_64", arm64: "aarch64" }[arch];
  if (!platform || !architecture) throw new Error(`No native Morphir release for ${os}/${arch}`);
  return { triple: `${architecture}-${platform}`, executable: os === "win32" ? "morphir.exe" : "morphir", format: os === "win32" ? "zip" : "tgz" };
}

export function parseReleasePin(value: unknown): ReleasePin {
  if (!value || typeof value !== "object") throw new Error("CLI pin must be an object");
  const pin = value as Record<string, unknown>;
  checkVersion(pin.version);
  if (!pin.sha256 || typeof pin.sha256 !== "object") throw new Error("CLI pin requires release archive checksums");
  const hashes = pin.sha256 as Record<string, unknown>;
  const targets = ["linux", "darwin", "win32"].flatMap((os) => ["x64", "arm64"].map((arch) => releaseTarget(os, arch).triple));
  if (Object.keys(hashes).length !== targets.length || targets.some((target) => !isDigest(hashes[target]))) {
    throw new Error("CLI pin requires a lowercase SHA-256 for each of the six release targets");
  }
  return { version: pin.version, sha256: hashes as Record<string, string> };
}

async function download(url: string): Promise<Uint8Array> {
  const response = await fetch(url, { signal: AbortSignal.timeout(600_000) });
  if (!response.ok) throw new Error(`Download failed: ${response.status} ${url}`);
  return new Uint8Array(await response.arrayBuffer());
}

async function executableBytes(bytes: Uint8Array, target: ReleaseTarget): Promise<Uint8Array> {
  // Archive paths never become filesystem destinations, including on Windows.
  if (target.format === "zip") {
    const files = unzipSync(bytes);
    const executable = files[target.executable];
    if (Object.keys(files).length !== 1 || !executable?.length) throw new Error("CLI archive must contain only its executable");
    return executable;
  }
  const files = await new Bun.Archive(bytes).files();
  const executable = files.get(target.executable);
  if (files.size !== 1 || !executable?.size) throw new Error("CLI archive must contain only its executable");
  return new Uint8Array(await executable.arrayBuffer());
}

export async function installCli(options: InstallOptions): Promise<string> {
  checkVersion(options.version);
  if (options.sha256 !== undefined && !isDigest(options.sha256)) throw new Error("Invalid pinned archive SHA-256");
  const target = options.target ?? releaseTarget();
  const fetchBytes = options.fetchBytes ?? download;
  const bin = path.join(options.root, ".dev/bin");
  mkdirSync(bin, { recursive: true });
  const directory = path.join(bin, `morphir-${options.version}-${target.triple}`);
  const cli = path.join(directory, target.executable);
  const verifyCache = () => {
    const receipt = JSON.parse(readFileSync(path.join(directory, "receipt.json"), "utf8"));
    if (!isDigest(receipt.archiveSha256) || !isDigest(receipt.executableSha256)
      || (options.sha256 !== undefined && options.sha256 !== receipt.archiveSha256)
      || digest(readFileSync(cli)) !== receipt.executableSha256) throw new Error(`Invalid CLI cache at ${directory}; remove it and retry`);
    return cli;
  };
  if (existsSync(directory)) return verifyCache();
  const temporary = mkdtempSync(path.join(bin, ".download-"));
  try {
    const asset = `morphir-${options.version}-${target.triple}.${target.format}`;
    const url = `https://github.com/finos/morphir/releases/download/v${options.version}/${asset}`;
    const bytes = await fetchBytes(url);
    const expected = options.sha256 ?? new TextDecoder().decode(await fetchBytes(`${url}.sha256`)).trim().split(/\s+/)[0];
    if (!isDigest(expected) || digest(bytes) !== expected) throw new Error(`Checksum mismatch for ${asset}`);
    const executable = await executableBytes(bytes, target);
    const stagedCli = path.join(temporary, target.executable);
    writeFileSync(stagedCli, executable);
    chmodSync(stagedCli, 0o755);
    writeFileSync(path.join(temporary, "receipt.json"), JSON.stringify({ archiveSha256: expected, executableSha256: digest(executable) }));
    // Another process may have installed the same verified release meanwhile.
    try { renameSync(temporary, directory); }
    catch (error) { if (!existsSync(directory)) throw error; }
    return verifyCache();
  } finally {
    rmSync(temporary, { recursive: true, force: true });
  }
}

if (import.meta.main) {
  try {
    if (process.argv.length !== 4 || process.argv[2] !== "--version") throw new Error("usage: bun scripts/released-cli.ts --version VERSION");
    const root = path.dirname(path.dirname(fileURLToPath(import.meta.url)));
    console.log(await installCli({ root, version: process.argv[3]! }));
  } catch (error) {
    console.error(String(error));
    process.exitCode = 1;
  }
}
