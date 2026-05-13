(function () {
  const navs = document.querySelectorAll(".component-site-nav");

  navs.forEach((nav) => {
    const button = nav.querySelector(".component-site-nav__menu-button");
    const panel = nav.querySelector(".component-site-nav__panel");

    if (button && panel) {
      button.addEventListener("click", () => {
        const isOpen = nav.getAttribute("data-open") === "true";
        nav.setAttribute("data-open", String(!isOpen));
        button.setAttribute("aria-expanded", String(!isOpen));
      });
    }
  });

  document.querySelectorAll(".component-theme-toggle").forEach((button) => {
    button.addEventListener("click", () => {
      const pressed = button.getAttribute("aria-pressed") === "true";
      const nextPressed = !pressed;
      document.documentElement.setAttribute("data-theme", nextPressed ? "dark" : "light");
      document.querySelectorAll(".component-theme-toggle").forEach((toggle) => {
        toggle.setAttribute("aria-pressed", String(nextPressed));
      });
    });
  });
})();
