import { describe, expect, it } from "bun:test";
import { classifyRepoProbe } from "../src/backup/index.ts";

/**
 * The messages below are the shapes restic 0.17.3 prints (cmd/restic/global.go), which is
 * the version the image pins. The point of the classifier: `restic init` may only follow
 * restic's explicit "no repository here", never a transient failure.
 */
const S3 = "s3:https://s3.example.com/bucket/prefix";

describe("classifyRepoProbe", () => {
  it("a readable repository is ready", () => {
    expect(classifyRepoProbe({ exitCode: 0, stderr: "" })).toBe("ready");
  });

  it("restic's 'repository does not exist' (exit 10) is missing: the only case that may init", () => {
    const stderr = `Fatal: repository does not exist: unable to open config file: Stat: The specified key does not exist.\nIs there a repository at the following location?\n${S3}\n`;
    expect(classifyRepoProbe({ exitCode: 10, stderr })).toBe("missing");
  });

  it("recognizes a missing repository by message too, should the exit code differ", () => {
    expect(
      classifyRepoProbe({ exitCode: 1, stderr: `Fatal: repository does not exist: ${S3}` }),
    ).toBe("missing");
    expect(classifyRepoProbe({ exitCode: 10, stderr: "" })).toBe("missing");
  });

  it("a wrong password is its own verdict (exit 12, or the message)", () => {
    expect(classifyRepoProbe({ exitCode: 12, stderr: "" })).toBe("wrong-password");
    expect(
      classifyRepoProbe({ exitCode: 1, stderr: "Fatal: wrong password or no key found" }),
    ).toBe("wrong-password");
  });

  it("'no key found' (a repository with unreadable keys) is never initialized over", () => {
    expect(classifyRepoProbe({ exitCode: 1, stderr: "Fatal: No key found in repo" })).toBe(
      "wrong-password",
    );
  });

  it("a transient failure is NOT a missing repository, even though restic asks 'is there a repository?'", () => {
    // restic prints this exact text for ANY failure to stat the config file.
    const network = `Fatal: unable to open config file: Stat: Get "https://s3.example.com/bucket/prefix/config": dial tcp: i/o timeout\nIs there a repository at the following location?\n${S3}\n`;
    expect(classifyRepoProbe({ exitCode: 1, stderr: network })).toBe("unreachable");
  });

  it("credential, bucket and proxy errors are unreachable", () => {
    for (const stderr of [
      `Fatal: unable to open repository at ${S3}: client.BucketExists: Access Denied.`,
      "Fatal: unable to open repository at s3:...: The AWS Access Key Id you provided does not exist in our records.",
      'Get "https://s3.example.com/": proxyconnect tcp: dial tcp 127.0.0.1:7890: connect: connection refused',
      "",
    ]) {
      expect(classifyRepoProbe({ exitCode: 1, stderr })).toBe("unreachable");
    }
  });

  it("does not depend on letter case", () => {
    expect(classifyRepoProbe({ exitCode: 1, stderr: "FATAL: Repository Does Not Exist" })).toBe(
      "missing",
    );
    expect(
      classifyRepoProbe({ exitCode: 1, stderr: "FATAL: WRONG PASSWORD OR NO KEY FOUND" }),
    ).toBe("wrong-password");
  });

  it("only 'missing' may lead to init", () => {
    const mayInit = (r: ReturnType<typeof classifyRepoProbe>) => r === "missing";
    const others = [
      { exitCode: 0, stderr: "" },
      { exitCode: 12, stderr: "" },
      { exitCode: 1, stderr: "timeout" },
      { exitCode: 130, stderr: "interrupted" },
    ];
    for (const o of others) expect(mayInit(classifyRepoProbe(o))).toBe(false);
  });
});
