import AxeBuilder from "@axe-core/playwright";
import { expect, test } from "@playwright/test";

type DocsPage = {
  title: string;
  description: string;
  slug: string;
  href: string;
};

const origin = "http://127.0.0.1:4321";
const projectTitle = "waft";
const landingHeadline = "Copy ignored files.";
const projectDescription =
  "waft copies ignored, untracked files selected by .worktreeinclude between Git worktrees.";
const themeStorageKey = "waft-theme";
const repositoryUrl = "https://github.com/plx/waft";
const basePath: string = "/waft";
const normalizedBasePath = basePath === "/" ? "" : basePath;
// prettier-ignore
const docsPages: DocsPage[] = [
    {
      "title": "Usage",
      "description": "Install waft and run its copy, list, info, and validate commands.",
      "slug": "usage",
      "href": "usage/"
    },
    {
      "title": ".worktreeinclude",
      "description": "Define file selection with .gitignore syntax.",
      "slug": "worktreeinclude",
      "href": "worktreeinclude/"
    },
    {
      "title": "Safety",
      "description": "Understand planning, conflicts, symlink handling, and hook trust boundaries.",
      "slug": "safety",
      "href": "safety/"
    },
    {
      "title": "Profiles",
      "description": "Choose claude, git, or wt file-selection behavior.",
      "slug": "profiles",
      "href": "profiles/"
    },
    {
      "title": "Configuration",
      "description": "Set file-selection and copy policy through config, environment, or CLI options.",
      "slug": "configuration",
      "href": "configuration/"
    },
    {
      "title": "Architecture",
      "description": "How waft resolves policy, selects files, and publishes copies.",
      "slug": "architecture",
      "href": "architecture/"
    }
  ];
const pagesToCheck = ["/", ...docsPages.map((page) => page.href)];
const pagesToAudit = pagesToCheck;

function sitePath(path = "/"): string {
  const cleanPath = path.startsWith("/") ? path : `/${path}`;
  return `${normalizedBasePath}${cleanPath}`;
}

function isSkippableHref(href: string): boolean {
  return (
    href === "" ||
    href.startsWith("mailto:") ||
    href.startsWith("tel:") ||
    href.startsWith("javascript:")
  );
}

test.describe("rendered site", () => {
  test("exposes core document and landmark properties", async ({ page }) => {
    await page.goto(sitePath("/"));

    expect(await page.title()).toContain(projectTitle);
    await expect(page.locator('meta[name="description"]')).toHaveAttribute(
      "content",
      projectDescription,
    );
    await expect(page.getByRole("main")).toBeVisible();
    await expect(
      page.getByRole("navigation", { name: /primary/i }),
    ).toBeVisible();
    await expect(
      page.getByRole("heading", {
        level: 1,
        name: landingHeadline,
        exact: true,
      }),
    ).toBeVisible();
    await expect(page.locator("svg.waft-mark").first()).toBeVisible();
    await expect(
      page.getByRole("img", { name: "waft mark", exact: true }).first(),
    ).toBeVisible();
    await expect(page.locator(".skip-link")).toHaveAttribute("href", "#main");
  });

  test("exposes accessible theme choices and persists a manual selection", async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: "dark" });
    await page.goto(sitePath("/"));

    const themeGroup = page.getByRole("radiogroup", {
      name: "Color theme",
      exact: true,
    });
    const lightChoice = themeGroup.locator('button[data-theme-choice="light"]');
    const darkChoice = themeGroup.locator('button[data-theme-choice="dark"]');
    const autoChoice = themeGroup.locator('button[data-theme-choice="auto"]');

    await expect(themeGroup).toBeVisible();
    await expect(lightChoice).toHaveAccessibleName("Light");
    await expect(darkChoice).toHaveAccessibleName("Dark");
    await expect(autoChoice).toHaveAccessibleName("System");
    await expect(lightChoice).toHaveAttribute("role", "radio");
    await expect(darkChoice).toHaveAttribute("role", "radio");
    await expect(autoChoice).toHaveAttribute("role", "radio");
    await expect(autoChoice).toHaveAttribute("aria-checked", "true");
    await expect(autoChoice).toHaveAttribute("tabindex", "0");
    await expect(lightChoice).toHaveAttribute("tabindex", "-1");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await autoChoice.press("ArrowLeft");

    await expect(darkChoice).toHaveAttribute("aria-checked", "true");
    await expect(darkChoice).toBeFocused();

    await lightChoice.click();

    await expect(lightChoice).toHaveAttribute("aria-checked", "true");
    await expect(darkChoice).toHaveAttribute("aria-checked", "false");
    await expect(autoChoice).toHaveAttribute("aria-checked", "false");
    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    expect(
      await page.evaluate((key) => localStorage.getItem(key), themeStorageKey),
    ).toBe("light");

    await page.reload();

    await expect(page.locator("html")).toHaveAttribute("data-theme", "light");
    await expect(
      page
        .getByRole("radiogroup", {
          name: "Color theme",
          exact: true,
        })
        .locator('button[data-theme-choice="light"]'),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("preserves Starlight search on documentation pages", async ({
    page,
  }) => {
    await page.goto(sitePath(docsPages[0]?.href));

    const search = page.getByRole("button", { name: "Search" });
    await expect(search).toBeVisible();
    await expect(search).toBeEnabled();
    await expect(page.getByRole("link", { name: "Edit page" })).toHaveAttribute(
      "href",
      `${repositoryUrl}/edit/main/site/src/content/docs/usage.mdx`,
    );
  });

  test("opens the documentation sidebar from the mobile menu", async ({
    page,
  }) => {
    await page.goto(sitePath(docsPages[0]?.href));

    const menu = page.getByRole("button", { name: "Menu", exact: true });
    const sidebar = page.locator("#starlight__sidebar");
    const compactNavigation = (page.viewportSize()?.width ?? 0) < 800;

    if (!compactNavigation) {
      await expect(menu).toBeHidden();
      await expect(sidebar).toBeVisible();
      return;
    }

    await expect(menu).toBeVisible();
    await expect(menu).toHaveAttribute("aria-controls", "starlight__sidebar");
    await expect(sidebar).toBeHidden();

    await menu.click();

    await expect(page.locator("body")).toHaveAttribute(
      "data-mobile-menu-expanded",
      "",
    );
    await expect(sidebar).toBeVisible();
    await expect(
      sidebar.getByRole("link", { name: "Usage", exact: true }),
    ).toBeVisible();

    await page.keyboard.press("Escape");

    await expect(page.locator("body")).not.toHaveAttribute(
      "data-mobile-menu-expanded",
      "",
    );
    await expect(sidebar).toBeHidden();
    await expect(menu).toBeFocused();
  });

  test("carries the selected theme from the landing page to docs", async ({
    page,
  }) => {
    await page.emulateMedia({ colorScheme: "light" });
    await page.goto(sitePath("/"));

    const darkChoice = page
      .getByRole("radiogroup", {
        name: "Color theme",
        exact: true,
      })
      .locator('button[data-theme-choice="dark"]');
    await darkChoice.click();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");

    await page.goto(sitePath(docsPages[0]?.href));

    await expect(page.getByRole("main")).toBeVisible();
    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    expect(
      await page.evaluate((key) => localStorage.getItem(key), themeStorageKey),
    ).toBe("dark");

    await page.goto(sitePath("/"));

    await expect(page.locator("html")).toHaveAttribute("data-theme", "dark");
    await expect(
      page
        .getByRole("radiogroup", {
          name: "Color theme",
          exact: true,
        })
        .locator('button[data-theme-choice="dark"]'),
    ).toHaveAttribute("aria-checked", "true");
  });

  test("keeps focus-ring contrast above 3:1 in both themes", async ({
    page,
  }) => {
    for (const theme of ["light", "dark"] as const) {
      await page.goto(sitePath("/"));
      await page.evaluate(
        ({ key, value }) => localStorage.setItem(key, value),
        { key: themeStorageKey, value: theme },
      );
      await page.reload();

      const ratio = await page.evaluate(() => {
        const styles = getComputedStyle(document.documentElement);
        const parseHex = (value: string): number[] => {
          const hex = value.trim().replace("#", "");
          return [0, 2, 4].map((offset) =>
            Number.parseInt(hex.slice(offset, offset + 2), 16),
          );
        };
        const luminance = (rgb: number[]): number => {
          const linear = rgb.map((channel) => {
            const value = channel / 255;
            return value <= 0.04045
              ? value / 12.92
              : ((value + 0.055) / 1.055) ** 2.4;
          });
          return (
            0.2126 * (linear[0] ?? 0) +
            0.7152 * (linear[1] ?? 0) +
            0.0722 * (linear[2] ?? 0)
          );
        };
        const focus = luminance(
          parseHex(styles.getPropertyValue("--waft-focus")),
        );
        const surface = luminance(
          parseHex(styles.getPropertyValue("--waft-surface")),
        );
        return (
          (Math.max(focus, surface) + 0.05) / (Math.min(focus, surface) + 0.05)
        );
      });

      expect(ratio, `${theme} focus ring contrast`).toBeGreaterThanOrEqual(3);
    }
  });

  test("keeps primary pages inside the viewport", async ({ page }) => {
    for (const pagePath of pagesToCheck) {
      await page.goto(sitePath(pagePath));
      await expect(page.getByRole("main")).toBeVisible();
      const hasHorizontalOverflow = await page.evaluate(
        () => document.documentElement.scrollWidth > window.innerWidth + 1,
      );
      expect(
        hasHorizontalOverflow,
        `${pagePath} should not overflow horizontally`,
      ).toBe(false);
    }
  });

  test("manages the mobile navigation expanded state accessibly", async ({
    page,
  }) => {
    await page.goto(sitePath("/"));

    const toggle = page.locator("[data-nav-toggle]");
    const compactNavigation = (page.viewportSize()?.width ?? 0) <= 900;
    if (!compactNavigation) {
      await expect(toggle).toBeHidden();
      return;
    }
    await expect(toggle).toBeVisible();

    const panel = page.locator("[data-nav-panel]");
    await expect(toggle).toHaveAttribute("aria-controls", "mobile-nav");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(panel).toBeHidden();

    await toggle.click();
    await expect(toggle).toHaveAttribute("aria-expanded", "true");
    await expect(panel).toBeVisible();
    await expect(
      page.getByRole("navigation", { name: "Mobile navigation" }),
    ).toBeVisible();

    const firstLink = panel.getByRole("link").first();
    await firstLink.focus();
    await expect(firstLink).toBeFocused();
    await page.keyboard.press("Escape");
    await expect(toggle).toHaveAttribute("aria-expanded", "false");
    await expect(panel).toBeHidden();
    await expect(toggle).toBeFocused();
  });

  test("keeps a theme control on the mobile 404 page", async ({ page }) => {
    const response = await page.goto(sitePath("missing-page/"));
    expect(response?.status()).toBe(404);
    await expect(
      page.getByRole("radiogroup", {
        name: "Color theme",
        exact: true,
      }),
    ).toBeVisible();
  });

  test("prints documentation with the light palette and no controls", async ({
    page,
  }) => {
    await page.addInitScript(
      (key) => localStorage.setItem(key, "dark"),
      themeStorageKey,
    );
    await page.emulateMedia({ media: "print", colorScheme: "dark" });
    await page.goto(sitePath(docsPages[0]?.href));

    await expect(page.locator("html")).toHaveCSS("color-scheme", "light");
    await expect(page.locator(".waft-doc-actions")).toBeHidden();
    await expect(page.locator(".waft-doc-title")).toHaveCSS(
      "color",
      "rgb(31, 35, 40)",
    );
  });

  test("validates rendered links and internal link targets", async ({
    page,
    request,
  }) => {
    const failures: string[] = [];

    for (const pagePath of pagesToCheck) {
      const response = await page.goto(sitePath(pagePath));
      expect(response?.status(), `${pagePath} should load`).toBeLessThan(400);

      const links = await page.locator("a[href]").evaluateAll((anchors) =>
        anchors.map((anchor) => ({
          href: anchor.getAttribute("href") ?? "",
          label: anchor.textContent?.trim() ?? "",
        })),
      );

      for (const link of links) {
        if (isSkippableHref(link.href)) {
          continue;
        }

        const resolved = new URL(link.href, `${origin}${sitePath(pagePath)}`);
        if (!["http:", "https:"].includes(resolved.protocol)) {
          failures.push(
            `${pagePath}: unsupported link protocol in ${link.href}`,
          );
          continue;
        }

        if (resolved.origin !== origin) {
          if (!link.label) {
            failures.push(
              `${pagePath}: external link ${link.href} has no text label`,
            );
          }
          continue;
        }

        if (
          normalizedBasePath &&
          resolved.pathname !== normalizedBasePath &&
          !resolved.pathname.startsWith(`${normalizedBasePath}/`)
        ) {
          failures.push(
            `${pagePath}: internal link escapes base path: ${link.href}`,
          );
          continue;
        }

        const targetPath = `${resolved.pathname}${resolved.search}`;
        const targetResponse = await request.get(targetPath);
        if (targetResponse.status() >= 400) {
          failures.push(
            `${pagePath}: ${link.href} returned ${targetResponse.status()}`,
          );
          continue;
        }

        if (resolved.hash) {
          await page.goto(`${targetPath}${resolved.hash}`);
          const targetExists = await page.evaluate((hash) => {
            const id = decodeURIComponent(hash.slice(1));
            return Boolean(
              document.getElementById(id) ||
              document.querySelector(`[name="${id}"]`),
            );
          }, resolved.hash);
          if (!targetExists) {
            failures.push(
              `${pagePath}: ${link.href} hash target does not exist`,
            );
          }
        }
      }
    }

    expect(failures).toEqual([]);
  });

  for (const theme of ["light", "dark"] as const) {
    test(`has no detectable accessibility violations in ${theme} mode`, async ({
      page,
    }) => {
      await page.addInitScript(
        ({ key, value }) => localStorage.setItem(key, value),
        { key: themeStorageKey, value: theme },
      );

      for (const pagePath of pagesToAudit) {
        await page.goto(sitePath(pagePath));
        await expect(page.locator("html")).toHaveAttribute("data-theme", theme);

        const results = await new AxeBuilder({ page }).analyze();
        expect(
          results.violations,
          `${pagePath} should have no Axe violations in ${theme} mode`,
        ).toEqual([]);
      }
    });
  }
});
