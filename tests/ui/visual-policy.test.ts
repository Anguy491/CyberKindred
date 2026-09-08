import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { join } from "node:path";

interface FontManifest {
  readonly runtimeNetworkRequired: boolean;
  readonly files: ReadonlyArray<{ readonly file: string; readonly sha256: string }>;
}

// UX-SYS-001/002/005/006; TEST-A11Y-002; NFR-A11Y-002; NFR-A11Y-004.
describe("TASK-030 visual policy", () => {
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
    expect(css).toContain("min-width: 44px");
    expect(css).toContain("outline: 2px solid var(--text-display)");
    expect(css).toContain("@media (prefers-reduced-motion: reduce)");
    expect(css).toContain("animation-iteration-count: 1 !important");
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

  it("meets the documented text and non-text token contrast thresholds", () => {
    const tokens = readHexTokens(css);
    for (const [foreground, background] of [
      ["--text-primary", "--black"],
      ["--text-secondary", "--black"],
      ["--text-primary", "--surface"],
      ["--text-secondary", "--surface"],
      ["--black", "--text-display"],
      ["--success", "--black"],
      ["--warning", "--black"],
      ["--interactive", "--black"],
    ] as const) {
      expect(contrast(tokens[foreground], tokens[background]), `${foreground} on ${background}`)
        .toBeGreaterThanOrEqual(4.5);
    }
    for (const [foreground, background] of [
      ["--text-disabled", "--black"],
      ["--accent", "--black"],
      ["--text-display", "--surface-raised"],
    ] as const) {
      expect(contrast(tokens[foreground], tokens[background]), `${foreground} on ${background}`)
        .toBeGreaterThanOrEqual(3);
    }
  });

  it("defines 44px targets, responsive reflow, and complete reduced-motion overrides", () => {
    expect(css).toMatch(/button\s*\{[^}]*min-width:\s*44px;[^}]*min-height:\s*44px;/isu);
    expect(css).toMatch(/input\[type="range"\]\s*\{[^}]*min-height:\s*44px;/isu);
    expect(css).toMatch(/\.disabled-link\s*\{[^}]*display:\s*inline-flex;[^}]*min-height:\s*44px;/isu);
    expect(css).toContain("@media (max-width: 900px)");
    expect(css).toContain("@media (max-width: 680px)");
    expect(css).toMatch(/\.weather-candidate-list button,\s*\.schedule-form\s*\{\s*grid-template-columns:\s*1fr;/isu);
    expect(css).toMatch(/@media \(prefers-reduced-motion: reduce\)[\s\S]*transition-duration:\s*0\.01ms !important;[\s\S]*animation-duration:\s*0\.01ms !important;[\s\S]*animation-iteration-count:\s*1 !important;/isu);

    const allCss = ["app.css", "features/library/library.css", "features/radio/radio.css", "features/you/you.css"]
      .map((path) => readFileSync(join(sourceRoot, path), "utf8"))
      .join("\n");
    const declared = new Set([...allCss.matchAll(/(--[a-z-]+):/giu)].map((match) => match[1]));
    const referenced = new Set([...allCss.matchAll(/var\((--[a-z-]+)/giu)].map((match) => match[1]));
    expect([...referenced].filter((token) => !declared.has(token))).toEqual([]);
  });
});

function readHexTokens(css: string): Record<string, string> {
  return Object.fromEntries([...css.matchAll(/(--[a-z-]+):\s*(#[0-9a-f]{6})/giu)]
    .map((match) => [match[1], match[2]]));
}

function contrast(first: string | undefined, second: string | undefined): number {
  if (first === undefined || second === undefined) throw new Error("Missing color token");
  const firstLuminance = luminance(first);
  const secondLuminance = luminance(second);
  return (Math.max(firstLuminance, secondLuminance) + 0.05)
    / (Math.min(firstLuminance, secondLuminance) + 0.05);
}

function luminance(hex: string): number {
  const values = [1, 3, 5].map((start) => Number.parseInt(hex.slice(start, start + 2), 16) / 255)
    .map((value) => value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4);
  return 0.2126 * (values[0] ?? 0) + 0.7152 * (values[1] ?? 0) + 0.0722 * (values[2] ?? 0);
}
