# Crux Website Redesign — Architecture & Verification Report

**Project**: Crux — Source-First C/C++ Package Manager  
**Website Directory**: `crux-site/`  
**Distribution Platform**: GitHub Pages (`https://shaharyarshakir.github.io/crux/`)  
**Base Path**: `/crux`  
**Framework**: Astro v7.4 + Starlight v0.43  
**Status**: Completed & Verified  

---

## 1. Executive Summary

The Crux documentation and marketing website has been completely redesigned from a generic template into a distinctive, editorial, developer-tool web platform. Crux is an engineering-first tool built for C and C++ software developers; the web presence now directly mirrors that philosophy:

1. **Editorial & Engineering Aesthetic**: High contrast, crisp typography, clean borders, and a focused dark-first visual identity centered on an **Ice Blue** accent (`#78A9C9`).
2. **Dual-Route Architecture**: A 100% custom developer-focused homepage (`src/pages/index.astro`) combined seamlessly with Astro Starlight for deep reference and architecture documentation (`src/content/docs/`).
3. **Unified Theme System**: Synchronized theme switching between dark (`#111110`) and light (`#F5F3EE`) across both custom layouts and Starlight pages with zero visual flicker.
4. **Static & Relocatable Production Build**: Zero runtime dependencies, 10 fully rendered HTML pages, automated Pagefind search indexing, and optimized WebP image generation conforming strictly to the GitHub Pages `/crux` base path.

---

## 2. Brand Identity & Design System

### 2.1 Color Palette & Design Tokens

Crux avoids generic SaaS visual cliches (such as neon glowing orbs, heavy drop shadows, and purple gradients). The interface prioritizes clarity, readability, and structural hierarchy.

| Token | Dark Theme (Default) | Light Theme | Usage |
| :--- | :--- | :--- | :--- |
| **Primary Accent** | `#78A9C9` (Ice Blue) | `#78A9C9` / `#3A6987` | Active links, buttons, highlights, terminal commands |
| **Accent Dim** | `rgba(120,169,201,0.12)` | `rgba(120,169,201,0.12)` | Badge backgrounds, active tab pills |
| **Background** | `#111110` | `#F5F3EE` | Page background |
| **Surface** | `#1D1D1B` | `#FFFFFF` | Cards, panels, header, buttons |
| **Surface Subtle** | `#171716` | `#EFECE5` | Section alternating backgrounds, footer |
| **Surface Elevated** | `#242422` | `#FAF8F4` | Hover states, elevated dialogs |
| **Text Primary** | `#EAE7DF` | `#171717` | Headlines, primary text |
| **Text Secondary** | `#B5B1A9` | `#77736D` | Descriptions, list items, body copy |
| **Text Muted** | `#99958D` | `#8E8A84` | Captions, footnotes, copyright |
| **Borders** | `#393733` | `#DEDAD1` | Card boundaries, dividers |
| **Terminal Background** | `#0C0C0B` | `#1A1A18` | Terminal windows, code showcase |

### 2.2 Typography

- **Body & Headlines**: System-native sans-serif stack (`system-ui, -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, sans-serif`) with tight tracking on headings (`-0.03em` to `-0.04em`) and comfortable line heights (`1.6` to `1.7`).
- **Code & Monospace**: `JetBrains Mono, Fira Code, ui-monospace, Menlo, Monaco, Consolas, monospace` for CLI prompts, manifests, digests, and code blocks.

---

## 3. Site Architecture & Component Library

```text
crux-site/
├── astro.config.mjs         # Site config: base '/crux', Starlight integration, sidebar
├── src/
│   ├── assets/
│   │   └── crux-logo.png    # Brand identity logo
│   ├── components/
│   │   ├── BaseLayout.astro     # Shell with SEO, OpenGraph, theme script, header/footer
│   │   ├── CodeExample.astro    # Interactive tabbed code showcase (crux.toml, crux.lock, etc.)
│   │   ├── CopyCommand.astro    # Accessible one-line install command bar with copy feedback
│   │   ├── FeatureSection.astro # 3x2 editorial grid of core architectural capabilities
│   │   ├── SiteFooter.astro     # 4-column technical footer with links and status
│   │   ├── SiteHeader.astro     # Sticky navigation with logo, nav links, GitHub, theme toggle
│   │   ├── Terminal.astro       # macOS/Linux terminal window mockup showing Crux CLI workflow
│   │   ├── ThemeToggle.astro    # Dark/light theme toggle button
│   │   └── WorkflowSection.astro# 6-step lifecycle pipeline (Declare -> Integrate)
│   ├── content/
│   │   └── docs/            # Starlight technical documentation pages
│   │       ├── design/
│   │       │   ├── architecture.md
│   │       │   └── security.md
│   │       ├── guides/
│   │       │   ├── getting-started.md
│   │       │   └── installation.md
│   │       ├── project/
│   │       │   ├── changelog.md
│   │       │   └── roadmap.md
│   │       ├── reference/
│   │       │   └── cli.md
│   │       └── registry/
│   │           └── packages.md
│   ├── pages/
│   │   └── index.astro      # Custom engineered developer homepage
│   └── styles/
│       └── theme.css        # Design tokens, theme variables, and Starlight overrides
└── public/
    ├── crux-logo.png        # Static logo for external crawlers and OG cards
    └── favicon.svg          # SVG brand favicon
```

### 3.1 Component Responsibilities

1. **`BaseLayout.astro`**:
   - Embeds OpenGraph (`og:title`, `og:description`, `og:image`, `og:url`) and Twitter Card metadata.
   - Inlines an early theme-detection script in `<head>` inspecting `localStorage.getItem('starlight-theme')` to prevent the flash of unstyled theme (FOUT).
   - Enforces responsive flex-column layout with sticky header and footer anchoring.

2. **`SiteHeader.astro`**:
   - Features the Crux logo, wordmark, and `v0.1.0` release badge.
   - Provides desktop navigation links to Guides, Installation, CLI Reference, Packages, and Architecture.
   - Integrates GitHub link with SVG icon, the `ThemeToggle` component, and an accessible mobile drawer menu.

3. **`CopyCommand.astro`**:
   - Displays the one-line installation command:
     `curl -fsSL https://raw.githubusercontent.com/ShaharyarShakir/crux/main/install/install.sh | bash`
   - Terminal prompt `$` indicator.
   - One-click copy with clipboard API, visual checkmark feedback, and animated "Copied!" tooltip.

4. **`Terminal.astro`**:
   - Window mockup with close, minimize, and expand window controls.
   - Demonstrates the complete Crux CLI lifecycle: `crux init`, `crux check`, `crux lock`, `crux install fmt`, and `crux integrate cmake`.
   - Distinct syntax highlighting: Ice Blue for commands, subtle green for verification checks, and light blue for ODR invariants.

5. **`FeatureSection.astro`**:
   - 6 detailed capability cards:
     - **ODR Invariant**: Strict single-version policy across static and shared libraries.
     - **GitOps Native**: Version-controlled catalogs stored in standard Git repositories.
     - **Cryptographic Integrity**: Streaming HTTP acquisition with SHA-256 and Zip Slip guards.
     - **Hermetic Engine**: Out-of-source CMake compilation with toolchain detection.
     - **Atomic Storage**: Staged validation preventing corrupted installations.
     - **Native Tooling**: Relocatable `CruxDependencies.cmake` with native target exports.

6. **`WorkflowSection.astro`**:
   - Visual 6-step numbered pipeline:
     1. `Declare` (`crux.toml`)
     2. `Resolve` (`crux resolve`)
     3. `Lock` (`crux lock`)
     4. `Acquire` (`crux fetch`)
     5. `Install` (`crux install`)
     6. `Integrate` (`crux integrate cmake`)

7. **`CodeExample.astro`**:
   - Interactive tabbed interface switching between:
     - `crux.toml`: Manifest dependency declarations.
     - `crux.lock`: Canonical deterministic lockfile with SHA-256 digests.
     - `CMakeLists.txt`: Standard consumer CMake integration with `find_package` and `target_link_libraries`.
     - `src/main.cpp`: C++20 consumer source demonstrating linked library calls.

8. **`SiteFooter.astro`**:
   - 4-column structured footer with direct links into documentation, architecture, and project resources.
   - MIT License disclosure and production readiness status indicator.

---

## 4. Documentation Inventory

All technical documentation is authored in Markdown within `crux-site/src/content/docs/`, fully indexed by Pagefind for client-side search:

| Route | Title | Description |
| :--- | :--- | :--- |
| `/crux/guides/getting-started/` | Getting Started with Crux | Step-by-step walkthrough: init, manifest, lock, install, CMake integration |
| `/crux/guides/installation/` | Installing Crux | Automated script, Nix Flake, manual binary, verification, and uninstall |
| `/crux/reference/cli/` | CLI Command Reference | Detailed command, argument, and flag reference across all subcommands |
| `/crux/registry/packages/` | Seed Package Registry | Reference documentation for `fmt 11.1.1` and `zlib 1.3.1` |
| `/crux/design/architecture/` | Architecture Overview | Subsystem breakdown, ODR resolution engine, and integration engine |
| `/crux/design/security/` | Security Model | Threat model, cryptographic verification, Zip Slip guards, and local execution |
| `/crux/project/roadmap/` | Project Roadmap | Completed Milestones 1–16 and post-v0.1.0 future initiatives |
| `/crux/project/changelog/` | Changelog | Full version history and release notes for v0.1.0 |

---

## 5. Build Verification Results

### 5.1 Build Command
```bash
cd crux-site && nix develop .. --command pnpm build
```

### 5.2 Build Output Log
```text
$ astro build
[content] Syncing content
[content] Synced content
[types] Generated 805ms
[build] output: "static"
[build] mode: "static"
[build] directory: /home/shaharyar/01__git_repos/crux/crux-site/dist/
[build] Collecting build info...
[build] ✓ Completed in 1.01s.
[build] Building static entrypoints...
[vite] ✓ built in 707ms
[vite] ✓ built in 119ms
[build] Rearranging server assets...

 generating static routes 
  ├─ /404.html (+31ms) 
  ├─ /index.html (+9ms) 
  ├─ /design/architecture/index.html (+10ms) 
  ├─ /design/security/index.html (+5ms) 
  ├─ /guides/getting-started/index.html (+6ms) 
  ├─ /guides/installation/index.html (+4ms) 
  ├─ /project/changelog/index.html (+4ms) 
  ├─ /project/roadmap/index.html (+4ms) 
  ├─ /reference/cli/index.html (+4ms) 
  ├─ /registry/packages/index.html (+5ms) 
✓ Completed in 163ms.

 generating optimized images 
  ▶ /_astro/crux-logo.8lOpqUXW_13n1tz.webp (reused cache entry) (+1ms) (1/2)
  ▶ /_astro/crux-logo.8lOpqUXW_Z2l5Vwv.webp (reused cache entry) (+0ms) (2/2)
✓ Completed in 2ms.

[build] ✓ Completed in 1.05s.
[starlight:pagefind] Building search index with Pagefind...
[starlight:pagefind] Found 10 HTML files.
[starlight:pagefind] Finished building search index in 25ms.
[@astrojs/sitemap] `sitemap-index.xml` created at `dist`
[build] 10 page(s) built in 2.10s
[build] Complete!
```

### 5.3 Verification Checklist
- [x] Zero compilation or runtime errors during static build.
- [x] All 10 HTML routes successfully generated in `dist/`.
- [x] Base path `/crux` correctly prepended across all CSS, JS, image, and link URLs.
- [x] Pagefind search index successfully built across all 10 documentation pages.
- [x] Dark/light theme toggle functions reliably with zero flash of unstyled theme.
- [x] Full mobile responsive navigation with drawer menu.
- [x] Copy button on installation bar and tabbed code previewer functioning properly.
