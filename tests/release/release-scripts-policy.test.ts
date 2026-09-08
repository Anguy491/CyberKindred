import { readFileSync } from "node:fs";
import { resolve } from "node:path";

const workspaceRoot = resolve(import.meta.dirname, "../..");

function script(path: string): string {
  return readFileSync(resolve(workspaceRoot, path), "utf8");
}

describe("M7 release provenance policy", () => {
  it("requires an out-of-band manifest digest before installer execution", () => {
    const installer = script("scripts/release/test-installer.ps1");
    expect(installer).toContain("TrustedManifestSha256");
    expect(installer).toContain("Read-ManifestSnapshot $manifestPath $TrustedSha256 $Label $RequireTrusted");
    expect(installer).toContain("manifest differs from the trusted out-of-band digest");
    expect(installer).toContain("installer differs from its trusted manifest immediately before execution");
    expect(installer).toContain("Start-Process -FilePath $stagedPath");
    expect(installer).not.toContain("[string]$DisposableSentinel");
  });

  it("uses a protected VM marker and authenticates the exact uninstaller", () => {
    const installer = script("scripts/release/test-installer.ps1");
    expect(installer).toContain("HKLM:\\SOFTWARE\\CyberKindred\\TestEnvironment");
    expect(installer).toContain("TrustedUninstallerSha256");
    expect(installer).toContain("exact manifest-bound LocalAppData product uninstaller");
  });

  it("rejects dirty development artifacts from release verification", () => {
    const manifest = script("scripts/release/artifact-manifest.mjs");
    const verifier = script("scripts/release/verify-release.ps1");
    expect(manifest).toContain("releaseEligible: !development && sourceStatus.length === 0");
    expect(manifest).toContain("dirty: sourceStatus.length > 0");
    expect(manifest).toContain("statusSha256: sha256(sourceStatus)");
    expect(verifier).toContain("TrustedManifestSha256");
    expect(verifier).toContain("development or dirty artifacts cannot pass release verification");
    expect(verifier).toContain("Read-TrustedManifest $manifestPath $TrustedSha256");
    expect(verifier).toContain("Assert-UnsignedExecutableSnapshot");
  });

  it("builds release candidates from an isolated exact-HEAD snapshot", () => {
    const builder = script("scripts/release/build-beta.ps1");
    expect(builder).toContain("worktree\", \"add\", \"--detach\", $snapshotRoot, $sourceCommit");
    expect(builder).toContain("source checkout changed during the isolated build");
    expect(builder).toContain("Assert-SafeBuildSnapshot $snapshotRoot");
    expect(builder).toContain("-CandidatePath\", $OutputRoot, \"-TrustedManifestSha256\", $trustedManifestHash");
  });

  it("compares both complete artifact and build-input inventories", () => {
    const reproducibility = script("scripts/release/test-reproducibility.ps1");
    expect(reproducibility).toContain("Compare-Object -CaseSensitive -ReferenceObject $first -DifferenceObject $second");
    expect(reproducibility).toContain("Compare-Object -CaseSensitive -ReferenceObject $firstBuildInputs -DifferenceObject $secondBuildInputs");
  });
});
