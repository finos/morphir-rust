import { afterEach, expect, test } from "bun:test";
import { mkdirSync, mkdtempSync, readdirSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { createHash } from "node:crypto";
import { zipSync } from "fflate";
import { installCli, parseReleasePin, releaseTarget } from "./released-cli";

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
const sha256 = (bytes: Uint8Array) => createHash("sha256").update(bytes).digest("hex");
const root = () => { const value = mkdtempSync(path.join(tmpdir(), "morphir-cli-test-")); roots.push(value); mkdirSync(path.join(value, ".dev/bin"), { recursive: true }); return value; };
const version = "0.4.0-beta.2";

test("maps all six native release targets, including Windows ARM64", () => {
  for (const [os, triple] of [["linux", "unknown-linux-gnu"], ["darwin", "apple-darwin"], ["win32", "pc-windows-msvc"]]) {
    for (const [arch, prefix] of [["x64", "x86_64"], ["arm64", "aarch64"]]) {
      const target = releaseTarget(os!, arch!);
      expect(target.triple).toBe(`${prefix}-${triple}`);
      expect(target.executable).toBe(os === "win32" ? "morphir.exe" : "morphir");
    }
  }
  expect(() => releaseTarget("freebsd", "x64")).toThrow();
});

test("refuses unsafe versions and incomplete or malformed production pins", () => {
  expect(() => parseReleasePin({ version: "../../bad", sha256: {} })).toThrow();
  expect(() => parseReleasePin({ version, sha256: {} })).toThrow();
  expect(() => parseReleasePin({ version, sha256: { "aarch64-apple-darwin": "not a hash" } })).toThrow();
});

test("accepts a complete immutable release pin without deriving checksums", () => {
  const hashes = Object.fromEntries(["linux", "darwin", "win32"].flatMap((os) =>
    ["x64", "arm64"].map((arch) => [releaseTarget(os, arch).triple, "a".repeat(64)])));
  expect(parseReleasePin({ version, sha256: hashes })).toEqual({ version, sha256: hashes });
});

test("verifies and atomically installs tar and Windows zip releases", async () => {
  for (const os of ["linux", "win32"]) {
    const directory = root();
    const target = releaseTarget(os, "arm64");
    const executable = new TextEncoder().encode("native cli bytes");
    const bytes = os === "win32"
      ? zipSync({ [target.executable]: executable })
      : new Uint8Array(await new Bun.Archive({ morphir: executable }, { compress: "gzip" }).bytes());
    const requested: string[] = [];
    const cli = await installCli({ root: directory, version, target, sha256: sha256(bytes), fetchBytes: async (url) => { requested.push(url); return bytes; } });
    expect(readFileSync(cli)).toEqual(Buffer.from(executable));
    expect(requested).toEqual([`https://github.com/finos/morphir/releases/download/v${version}/morphir-${version}-${target.triple}.${target.format}`]);
    expect(readdirSync(path.join(directory, ".dev/bin")).some((name) => name.includes(".download-"))).toBe(false);
    const cached = await installCli({ root: directory, version, target, sha256: sha256(bytes), fetchBytes: async () => { throw new Error("cache must not fetch"); } });
    expect(cached).toBe(cli);
    await expect(installCli({ root: directory, version, target, sha256: "b".repeat(64) })).rejects.toThrow(/cache/);
    writeFileSync(cli, "corrupt cache");
    await expect(installCli({ root: directory, version, target, sha256: sha256(bytes) })).rejects.toThrow(/cache/);
  }
});

test("checksum mismatch and interrupted fetch never publish a cached CLI", async () => {
  for (const fetchBytes of [async () => new Uint8Array([1]), async () => { throw new Error("interrupted"); }]) {
    const directory = root();
    await expect(installCli({ root: directory, version, sha256: "a".repeat(64), fetchBytes })).rejects.toThrow();
    expect(readdirSync(path.join(directory, ".dev/bin"))).toEqual([]);
  }
});

test("extension helper reads and checks the published per-archive checksum", async () => {
  const directory = root();
  const bytes = new Uint8Array(await new Bun.Archive({ morphir: "cli" }, { compress: "gzip" }).bytes());
  const target = releaseTarget("linux", "x64");
  const asset = `morphir-${version}-${target.triple}.tgz`;
  const cli = await installCli({ root: directory, version, target, fetchBytes: async (url) => url.endsWith(".sha256") ? new TextEncoder().encode(`${sha256(bytes)}  ${asset}\n`) : bytes });
  expect(readFileSync(cli, "utf8")).toBe("cli");
});
