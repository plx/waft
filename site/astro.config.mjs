import { defineConfig } from "astro/config";
import mdx from "@astrojs/mdx";
import starlight from "@astrojs/starlight";

export default defineConfig({
  site: "https://plx.github.io",
  base: "/waft",
  integrations: [
    starlight({
      title: "waft",
      description: "Copy .worktreeinclude-selected ignored files between Git worktrees.",
      social: [
        { icon: "github", label: "GitHub", href: "https://github.com/plx/waft" }
      ],
      customCss: ["./src/styles/waft.css"],
      sidebar: [
        {
          label: "Guides",
          items: [
            { label: "Usage", slug: "usage" },
            { label: ".worktreeinclude", slug: "worktreeinclude" },
            { label: "Safety", slug: "safety" },
            { label: "Profiles", slug: "profiles" },
            { label: "Configuration", slug: "configuration" }
          ]
        },
        {
          label: "Reference",
          items: [{ label: "Architecture", slug: "architecture" }]
        }
      ]
    }),
    mdx()
  ]
});
