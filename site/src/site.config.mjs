// Shared copy and navigation. Keep synchronized with ../site-template.json.
// Visual and content guidance lives under ../design-system/.
// prettier-ignore
export const siteConfig = {
  "repository": {
    "owner": "plx",
    "name": "waft",
    "url": "https://github.com/plx/waft",
    "defaultBranch": "main"
  },
  "project": {
    "name": "waft",
    "title": "waft",
    "packageName": "waft-site",
    "category": "Git worktree file copier",
    "tagline": "Copy ignored files into a linked worktree.",
    "description": "waft copies selected Git-ignored files from the main worktree to a linked worktree.",
    "installCommand": "cargo install --git https://github.com/plx/waft --rev REVIEWED_COMMIT_SHA --locked waft"
  },
  "site": {
    "host": "https://plx.github.io",
    "basePath": "/waft",
    "url": "https://plx.github.io/waft/",
    "dir": "site",
    "language": "en"
  },
  "landing": {
    "headline": "Copy ignored files.",
    "lede": [
      {
        "text": "waft",
        "code": true
      },
      {
        "text": " copies Git-ignored files from the main worktree to a linked worktree. Select them with "
      },
      {
        "text": ".worktreeinclude",
        "code": true
      },
      {
        "text": "."
      }
    ],
    "body": "By default, it copies missing files and leaves existing destination files alone.",
    "nav": [
      {
        "label": "Overview",
        "href": "#main"
      },
      {
        "label": "Usage",
        "href": "usage/"
      },
      {
        "label": ".worktreeinclude",
        "href": "worktreeinclude/"
      },
      {
        "label": "Safety",
        "href": "safety/"
      },
      {
        "label": "Profiles",
        "href": "profiles/"
      },
      {
        "label": "GitHub",
        "href": "https://github.com/plx/waft"
      }
    ],
    "footerLinks": [
      {
        "label": "Usage",
        "href": "usage/"
      },
      {
        "label": ".worktreeinclude",
        "href": "worktreeinclude/"
      },
      {
        "label": "Safety",
        "href": "safety/"
      },
      {
        "label": "Architecture",
        "href": "architecture/"
      },
      {
        "label": "Configuration",
        "href": "configuration/"
      },
      {
        "label": "GitHub",
        "href": "https://github.com/plx/waft"
      }
    ],
    "primaryCta": {
      "label": "Usage",
      "href": "usage/"
    },
    "secondaryCta": {
      "label": "Source",
      "href": "https://github.com/plx/waft"
    },
    "terminal": {
      "title": "Quick start",
      "copy": "waft copy --dry-run",
      "lines": [
        "# Main worktree: .worktreeinclude",
        ".env",
        "*.env.local",
        "",
        "# Linked worktree: preview",
        "$ waft copy --dry-run",
        "",
        "# After reviewing the output",
        "$ waft copy"
      ]
    }
  },
  "docs": {
    "sidebar": [
      {
        "label": "Guides",
        "items": [
          {
            "label": "Usage",
            "slug": "usage"
          },
          {
            "label": ".worktreeinclude",
            "slug": "worktreeinclude"
          },
          {
            "label": "Safety",
            "slug": "safety"
          },
          {
            "label": "Profiles",
            "slug": "profiles"
          },
          {
            "label": "Configuration",
            "slug": "configuration"
          },
          {
            "label": "Architecture",
            "slug": "architecture"
          }
        ]
      }
    ],
    "pages": [
      {
        "title": "Usage",
        "description": "Installation, first copy, and command reference.",
        "slug": "usage",
        "href": "usage/"
      },
      {
        "title": ".worktreeinclude",
        "description": "Which files to copy and how patterns match.",
        "slug": "worktreeinclude",
        "href": "worktreeinclude/"
      },
      {
        "title": "Safety",
        "description": "Existing files, symlinks, and concurrent changes.",
        "slug": "safety",
        "href": "safety/"
      },
      {
        "title": "Profiles",
        "description": "The differences between claude, git, and wt.",
        "slug": "profiles",
        "href": "profiles/"
      },
      {
        "title": "Configuration",
        "description": "Config files, environment variables, and CLI overrides.",
        "slug": "configuration",
        "href": "configuration/"
      },
      {
        "title": "Architecture",
        "description": "How the code selects and copies files.",
        "slug": "architecture",
        "href": "architecture/"
      }
    ]
  }
};
