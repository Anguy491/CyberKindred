export const desktopCases = [
  {
    name: "m2 shell boots inside desktop [FR-RAD-004] [NFR-COMPAT-003]",
    async run(session) {
      const heading = await session.waitForElement("#radio-title");
      const text = await session.elementText(heading);
      assertEqual(text, "今天想听什么状态？", "desktop radio heading");
    },
  },
  {
    name: "m2 shell desktop navigation [FR-SET-001] [NFR-A11Y-001]",
    async run(session) {
      const settings = await session.waitForElement('[aria-label="设置"]');
      await session.click(settings);
      const current = await session.elementAttribute(settings, "aria-current");
      assertEqual(current, "page", "settings navigation state");
      const heading = await session.waitForElement("#settings-title");
      const text = await session.elementText(heading);
      assertEqual(text, "AI & VOICE", "desktop settings heading");
    },
  },
];

export function selectDesktopCases(cases, filter) {
  const normalized = filter.trim().toLocaleLowerCase("en-US");
  if (normalized.length > 100) {
    throw new Error("Desktop case filter must be at most 100 characters.");
  }
  if (normalized.length === 0) return [...cases];
  const selected = cases.filter(({ name }) => name.toLocaleLowerCase("en-US").includes(normalized));
  if (selected.length === 0) {
    throw new Error(`No desktop E2E case matched ${JSON.stringify(filter)}.`);
  }
  return selected;
}

function assertEqual(actual, expected, label) {
  if (actual !== expected) {
    throw new Error(`${label}: expected ${JSON.stringify(expected)}, received ${JSON.stringify(actual)}.`);
  }
}
