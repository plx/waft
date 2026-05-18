(function () {
  const buttons = document.querySelectorAll("[data-copy-text]");

  buttons.forEach((button) => {
    button.addEventListener("click", async () => {
      const text = button.getAttribute("data-copy-text") || "";
      const original = button.getAttribute("data-label") || button.textContent || "Copy";

      try {
        if (navigator.clipboard && window.isSecureContext) {
          await navigator.clipboard.writeText(text);
        }
        button.textContent = "Copied";
        button.setAttribute("aria-live", "polite");
      } catch {
        button.textContent = "Copy";
      }

      window.setTimeout(() => {
        button.textContent = original;
      }, 1200);
    });
  });
})();
