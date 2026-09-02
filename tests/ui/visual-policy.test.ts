import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

interface FontManifest {
  readonly runtimeNetworkRequired: boolean;
  readonly files: ReadonlyArray<{ readonly file: string; readonly sha256: string }>;
}

// UX-SYS-001/002/005/006; NFR-A11Y-002; NFR-A11Y-004.
describe("TASK-009 visual policy", () => {
  const sourceRoot = join(process.cwd(), "src");
  const css = readFileSync(join(sourceRoot, "app.css"), "utf8");

  it("uses exact OLED tokens and contains no prohibited visual treatments", () => {
    const requiredTokens = [
      "--black: #000000", "--surface: #111111", "--surface-raised: #1a1a1a",
      "--border: #222222", "--border-visible: #333333", "--text-disabled: #666666",
      "--text-secondary: #999999", "--text-primary: #e8e8e8", "--text-display: #ffffff",
      "--accent: #d71921", "--error: #d71921", "--success: #4a9e5c",
      "--warning: #d4a843", "--interactive: #5b9bf6",
    ];
    for (const token of requiredTokens) expect(css).toContain(token);
    expect(css).not.toMatch(/(?:linear|radial|conic)-gradient|box-shadow|backdrop-filter|filter:\s*blur/iu);
    expect(css).not.toMatch(/@keyframes\s+(?:bounce|spring)|scroll-snap-type/iu);
    expect(css).toContain("min-height: 44px");
    expect(css).toContain("outline: 2px solid var(--text-display)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).not.toMatch(/url\(["']?https?:/iu);
  });

  it("references every local font and matches the checked-in manifest hashes", () => {
    const manifest = JSON.parse(
      readFileSync(join(sourceRoot, "assets", "fonts", "MANIFEST.json"), "utf8"),
    ) as FontManifest;
    expect(manifest.runtimeNetworkRequired).toBe(false);
    for (const entry of manifest.files) {
      expect(css).toContain(`./assets/fonts/${entry.file}`);
      const bytes = readFileSync(join(sourceRoot, "assets", "fonts", entry.file));
      expect(createHash("sha256").update(bytes).digest("hex").toUpperCase()).toBe(entry.sha256);
    }
  });
});
