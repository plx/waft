import { defineConfig } from "astro/config";
import starlight from "@astrojs/starlight";
import { siteConfig } from "./src/site.config.mjs";

const basePath =
  siteConfig.site.basePath === "/" ? "" : siteConfig.site.basePath;
/** @param {string} path */
const siteAsset = (path) => `${basePath}/${path.replace(/^\/+/, "")}`;

export default defineConfig({
  site: siteConfig.site.host,
  base: siteConfig.site.basePath,
  trailingSlash: "always",
  integrations: [
    starlight({
      title: siteConfig.project.title,
      description: siteConfig.project.description,
      components: {
        Header: "./src/components/StarlightHeader.astro",
        MobileMenuFooter: "./src/components/StarlightMobileMenuFooter.astro",
        SiteTitle: "./src/components/StarlightSiteTitle.astro",
        ThemeProvider: "./src/components/ThemeProvider.astro",
        ThemeSelect: "./src/components/ThemeToggle.astro",
      },
      customCss: ["./src/styles/starlight.css"],
      head: [
        {
          tag: "link",
          attrs: { rel: "preconnect", href: "https://fonts.googleapis.com" },
        },
        {
          tag: "link",
          attrs: {
            rel: "preconnect",
            href: "https://fonts.gstatic.com",
            crossorigin: "",
          },
        },
        {
          tag: "link",
          attrs: {
            rel: "icon",
            href: siteAsset("favicon.svg"),
            type: "image/svg+xml",
          },
        },
      ],
      editLink: {
        baseUrl: `${siteConfig.repository.url}/edit/${siteConfig.repository.defaultBranch}/site/`,
      },
      sidebar: siteConfig.docs.sidebar,
    }),
  ],
});
