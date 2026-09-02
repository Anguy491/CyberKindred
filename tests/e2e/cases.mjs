export const desktopCases = [
  {
    name: "m2 onboarding reaches silent radio fallback [FR-ONB-001] [FR-ONB-007]",
    async run(session) {
      await ensureSilentFallbackOnboarding(session);
      const heading = await session.waitForElement("#radio-title");
      const text = await session.elementText(heading);
      assertEqual(text, "今天想听什么状态？", "desktop radio heading");
      await session.waitForElement('[data-testid="radio-start"]');
    },
  },
  {
    name: "m2 shell desktop navigation [FR-SET-001] [NFR-A11Y-001]",
    async run(session) {
      await ensureSilentFallbackOnboarding(session);
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

async function ensureSilentFallbackOnboarding(session) {
  try {
    await session.findElement("#radio-title");
    return;
  } catch {
    // A filtered desktop case starts with a fresh dedicated app-data root.
  }
  await clickWhenReady(session, '[data-testid="onboarding-next-welcome"]');
  await session.waitForElement('[data-testid="onboarding-step-music_source"]');
  await clickWhenReady(session, "#onboarding-source-apple");
  await clickWhenReady(session, '[data-testid="onboarding-next-source"]');
  await session.waitForElement('[data-testid="onboarding-step-openai_key"]');
  await clickWhenReady(session, '[data-testid="onboarding-local-only"]');
  await session.waitForElement('[data-testid="onboarding-step-voice"]');
  await clickWhenReady(session, '[data-testid="onboarding-next-text-only"]');
  await session.waitForElement('[data-testid="onboarding-step-profile"]');
  await clickWhenReady(session, '[data-testid="onboarding-next-profile"]');
  await session.waitForElement('[data-testid="onboarding-step-city_schedule"]');
  await clickWhenReady(session, '[data-testid="onboarding-next-city"]');
  await session.waitForElement('[data-testid="onboarding-step-privacy"]');
  await clickWhenReady(session, "#onboarding-confirm-sound");
  await clickWhenReady(session, "#onboarding-confirm-retention");
  await clickWhenReady(session, '[data-testid="onboarding-complete"]');
  await session.waitForElement("#radio-title");
}

async function clickWhenReady(session, selector) {
  const element = await session.waitForElement(selector);
  await session.click(element);
}

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
