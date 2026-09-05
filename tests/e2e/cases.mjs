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
  {
    name: "m6 settings applies and removes silent Windows behaviors [FR-SET-003] [NFR-COMPAT-001]",
    async run(session) {
      await ensureSilentFallbackOnboarding(session);
      await clickWhenReady(session, '[aria-label="设置"]');
      await clickWhenReady(session, '[data-settings-group="app"]');
      let autostartEnabled = false;
      let trayEnabled = false;
      try {
        await clickWhenEnabled(session, '[data-testid="app-autostart-enable"]');
        autostartEnabled = true;
        await session.waitForElement('[data-testid="app-autostart-disable"]');
        await clickWhenEnabled(session, '[data-testid="app-tray-enable"]');
        trayEnabled = true;
        await session.waitForElement('[data-testid="app-tray-disable"]');
      } finally {
        if (trayEnabled) {
          await clickWhenEnabled(session, '[data-testid="app-tray-disable"]');
          await session.waitForElement('[data-testid="app-tray-enable"]');
        }
        if (autostartEnabled) {
          await clickWhenEnabled(session, '[data-testid="app-autostart-disable"]');
          await session.waitForElement('[data-testid="app-autostart-enable"]');
        }
      }
      const autostart = await session.waitForElement('[data-testid="app-autostart-enable"]');
      const tray = await session.waitForElement('[data-testid="app-tray-enable"]');
      assertEqual(await session.elementText(autostart), "启用登录启动", "autostart restored off");
      assertEqual(await session.elementText(tray), "启用托盘运行", "tray restored off");
    },
  },
  {
    name: "m5 weather stays behind explicit city search [FR-WEA-001] [NFR-PRIV-004]",
    async run(session) {
      await ensureSilentFallbackOnboarding(session);
      await clickWhenReady(session, '[aria-label="设置"]');
      await clickWhenReady(session, '[data-settings-group="context"]');
      await session.waitForElement('[data-testid="weather-city-query"]');
      await session.waitForElement('[data-testid="weather-search"]');
      const heading = await session.waitForElement("#settings-title");
      assertEqual(await session.elementText(heading), "CONTEXT", "weather settings heading");
    },
  },
  {
    name: "m5 schedule persists a notification-only rule [FR-SCH-001] [NFR-COST-001]",
    async run(session) {
      await ensureSilentFallbackOnboarding(session);
      await clickWhenReady(session, '[aria-label="设置"]');
      await clickWhenReady(session, '[data-settings-group="schedule"]');
      await clickWhenReady(session, '[data-testid="schedule-create"]');
      await session.waitForElement('[data-testid="schedule-rule"]');
      const heading = await session.waitForElement("#settings-title");
      assertEqual(await session.elementText(heading), "SCHEDULE", "schedule settings heading");
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

async function clickWhenEnabled(session, selector, timeoutMs = 10_000) {
  const deadline = Date.now() + timeoutMs;
  while (Date.now() < deadline) {
    const element = await session.waitForElement(selector);
    if (await session.elementAttribute(element, "disabled") === null) {
      await session.click(element);
      return;
    }
    await new Promise((resolve) => setTimeout(resolve, 100));
  }
  throw new Error(`Desktop control did not become enabled within ${timeoutMs}ms.`);
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
