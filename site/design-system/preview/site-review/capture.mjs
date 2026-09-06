// Run after building and starting the production preview. See README.md.
import { chromium } from '@playwright/test';
import { readFile, mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

const baseURL = process.argv[2] || 'http://127.0.0.1:4321/waft/';
const output = new URL('./', import.meta.url);
await mkdir(output, { recursive: true });
const browser = await chromium.launch();
const metrics = [];
try {
  for (const [device, width, height] of [['desktop', 1440, 1000], ['mobile', 390, 844]]) {
    for (const theme of ['light', 'dark']) {
      const page = await browser.newPage({
        viewport: { width, height }, deviceScaleFactor: 1, colorScheme: theme,
      });
      await page.addInitScript(theme => localStorage.setItem('waft-theme', theme), theme);
      for (const variant of ['current', 'headline', 'index']) {
        await page.goto(baseURL);
        await page.evaluate(() => document.fonts.ready);
        if (variant !== 'current') {
          await page.addStyleTag({ content: await readFile(new URL(`proposal-${variant}.css`, output), 'utf8') });
        }
        const measured = await page.evaluate(() => ({
          height: document.documentElement.scrollHeight,
          width: document.documentElement.scrollWidth,
          headlineSize: getComputedStyle(document.querySelector('h1')).fontSize,
        }));
        if (measured.width > width) throw new Error(`${variant} overflows ${device}`);
        await page.screenshot({ path: fileURLToPath(new URL(`${variant}-${device}-${theme}.png`, output)), fullPage: true });
        metrics.push({ variant, device, theme, ...measured });
      }
      await page.close();
    }
  }
  await writeFile(new URL('measurements.json', output), JSON.stringify(metrics, null, 2) + '\n');
} finally {
  await browser.close();
}
