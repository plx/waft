(function () {
  document.querySelectorAll(".component-command__copy[data-copy-target]").forEach((button) => {
    button.addEventListener("click", async () => {
      const target = document.querySelector(button.getAttribute("data-copy-target"));
      const text = target ? target.textContent.trim() : "";
      const original = button.textContent || "Copy";

      try {
        if (navigator.clipboard && window.isSecureContext) {
          await navigator.clipboard.writeText(text);
        }
        button.textContent = "Copied";
      } catch {
        button.textContent = "Copy";
      }

      window.setTimeout(() => {
        button.textContent = original;
      }, 1200);
    });
  });
})();
