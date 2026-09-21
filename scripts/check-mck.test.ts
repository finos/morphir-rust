import { afterEach, expect, test } from "bun:test";
import { existsSync, mkdirSync, mkdtempSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import path from "node:path";
import { exitCodeOf, runKit } from "./check-mck";

const roots: string[] = [];
afterEach(() => { for (const root of roots.splice(0)) rmSync(root, { recursive: true, force: true }); });
function fixture() {
  const root = mkdtempSync(path.join(tmpdir(), "mck-task-test-")); roots.push(root);
  const kit = path.join(root, "managed kit"); mkdirSync(kit);
  writeFileSync(path.join(kit, "mck-kit.lock.json"), "{}");
  const report = path.join(root, ".dev/out/mck/native-report.json"); mkdirSync(path.dirname(report), { recursive: true }); writeFileSync(report, "stale");
  return { root, kit, cli: "native cli.exe", report };
}

test("removes stale reports before building and uses one explicit kit for every native gate", async () => {
  const options = fixture(); const calls: string[][] = [];
  await runKit(options, async (args) => {
    if (!calls.length) expect(existsSync(options.report)).toBe(false);
    calls.push(args);
    if (args.includes("run")) { writeFileSync(options.report, "fresh"); return 1; }
    return 0;
  });
  for (const args of calls.filter((args) => args[1] === "mck" && args[3] !== "render")) {
    if (args[2] === "check") expect(args[3]).toBe(options.kit);
    else expect(args[args.indexOf("--kit") + 1]).toBe(options.kit);
  }
  const check = calls.find((args) => args[2] === "report");
  expect(check?.slice(1, 5)).toEqual(["mck", "report", "check", options.report]);
  expect(check).toContain(path.join(options.root, "crates/morphir-mck-adapter/allowed-failing.json"));
  expect(calls.some((args) => args.includes("--test"))).toBe(false);
  const build = calls.find((args) => args[0] === "cargo");
  expect(build?.slice(-2)).toEqual(["--target-dir", path.join(options.root, "target")]);
  expect(calls.find((args) => args[3] === "render") ?? []).toContain(path.join(options.root, ".dev/out/mck/native-report.html"));
});

test("run exit one only succeeds when native report adjudication succeeds", async () => {
  const options = fixture();
  await expect(runKit(options, async (args) => {
    if (args.includes("run")) { writeFileSync(options.report, "failed session"); return 1; }
    return args[2] === "report" && args[3] === "check" ? 1 : 0;
  })).rejects.toThrow(/report/);
});

test("refuses missing fresh reports and non-verdict driver exits", async () => {
  for (const status of [0, 1, 2, 137]) {
    const options = fixture();
    await expect(runKit(options, async (args) => args.includes("run") ? status : 0)).rejects.toThrow();
    expect(existsSync(options.report)).toBe(false);
  }
});

test("requires a managed kit and never builds an adapter for an unmanaged source", async () => {
  const options = fixture(); rmSync(path.join(options.kit, "mck-kit.lock.json"));
  await expect(runKit(options, async () => { throw new Error("unexpected command"); })).rejects.toThrow(/managed kit/);
});

test("a signal after a fresh report cannot masquerade as an allowed exit one", async () => {
  const options = fixture(); const calls: string[][] = [];
  await expect(runKit(options, async (args) => {
    calls.push(args);
    if (args.includes("run")) {
      writeFileSync(options.report, "fresh report before signal");
      return exitCodeOf({ exitCode: 1, signalCode: "SIGTERM" });
    }
    return 0;
  })).rejects.toThrow(/SIGTERM/);
  expect(calls.some((args) => args[2] === "report" && args[3] === "check")).toBe(false);
  expect(calls.some((args) => args[3] === "render")).toBe(true);
});
