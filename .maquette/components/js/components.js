(function () {
  function loadScript(src) {
    const script = document.createElement("script");
    script.src = src;
    script.defer = true;
    document.head.appendChild(script);
  }

  const current = document.currentScript;
  const base = current && current.src ? current.src.replace(/[^/]+$/, "") : "js/";

  loadScript(base + "core-actions.components.js");
  loadScript(base + "navigation-layout.components.js");
  loadScript(base + "docs-composites.components.js");
})();
