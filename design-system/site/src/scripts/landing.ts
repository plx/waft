const navToggle =
  document.querySelector<HTMLButtonElement>("[data-nav-toggle]");
const navPanel = document.querySelector<HTMLElement>("[data-nav-panel]");
const navLinks = document.querySelectorAll<HTMLElement>("[data-nav-link]");

type ThemeChoice = "light" | "dark" | "auto";
const THEME_KEY = "waft-theme";

function readTheme(): ThemeChoice {
  try {
    const stored = localStorage.getItem(THEME_KEY);
    if (stored === "light" || stored === "dark") {
      return stored;
    }
  } catch {
    // localStorage unavailable — fall through to auto.
  }
  return "auto";
}

const themeButtons = document.querySelectorAll<HTMLButtonElement>(
  "[data-theme-choice]",
);

function applyTheme(choice: ThemeChoice): void {
  document.documentElement.setAttribute("data-theme", choice);
  themeButtons.forEach((button) => {
    button.setAttribute(
      "aria-checked",
      String(button.dataset.themeChoice === choice),
    );
  });
}

applyTheme(readTheme());

themeButtons.forEach((button) => {
  button.addEventListener("click", () => {
    const choice = button.dataset.themeChoice as ThemeChoice | undefined;
    if (choice !== "light" && choice !== "dark" && choice !== "auto") {
      return;
    }
    try {
      localStorage.setItem(THEME_KEY, choice);
    } catch {
      // Persisting is best-effort.
    }
    applyTheme(choice);
  });
});

function setNavOpen(open: boolean): void {
  if (!navToggle || !navPanel) {
    return;
  }

  navToggle.setAttribute("aria-expanded", String(open));
  navToggle.setAttribute(
    "aria-label",
    open ? "Close navigation" : "Open navigation",
  );
  navPanel.hidden = !open;
  navPanel.dataset.open = String(open);
}

navToggle?.addEventListener("click", () => {
  setNavOpen(navToggle.getAttribute("aria-expanded") !== "true");
});

navLinks.forEach((link) => {
  link.addEventListener("click", () => setNavOpen(false));
});

document.addEventListener("keydown", (event) => {
  if (event.key === "Escape") {
    setNavOpen(false);
  }
});

async function copyText(text: string): Promise<void> {
  if (!navigator.clipboard) {
    throw new Error("Clipboard API is unavailable.");
  }

  await navigator.clipboard.writeText(text);
}

document
  .querySelectorAll<HTMLButtonElement>("[data-copy-text]")
  .forEach((button) => {
    let resetTimer: number | undefined;
    const visibleLabel =
      button.querySelector<HTMLElement>("span:not(.sr-only)");
    const status = button.querySelector<HTMLElement>("[data-copy-status]");
    const defaultVisibleText = visibleLabel?.textContent || "Copy";

    button.addEventListener("click", async () => {
      const text = button.dataset.copyText;
      if (!text) {
        return;
      }

      window.clearTimeout(resetTimer);
      try {
        await copyText(text);
        if (visibleLabel) {
          visibleLabel.textContent = "Copied";
        }
        if (status) {
          status.textContent = "Command copied to clipboard.";
        }
      } catch {
        if (visibleLabel) {
          visibleLabel.textContent = "Copy failed";
        }
        if (status) {
          status.textContent = "Copy failed.";
        }
      }

      resetTimer = window.setTimeout(() => {
        if (visibleLabel) {
          visibleLabel.textContent = defaultVisibleText;
        }
        if (status) {
          status.textContent = "";
        }
      }, 2200);
    });
  });
