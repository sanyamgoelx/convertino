# Code signing for Windows (free, via SignPath Foundation)

Unsigned installers make Windows SmartScreen say "Windows protected your PC".
SignPath Foundation signs open-source projects for free with its own
certificate, from GitHub Actions builds.

## Applying (once, by the maintainer)

Apply at https://signpath.org/apply (SignPath Foundation, "Open Source").
What to enter:

- **Project name:** Convertino
- **Repository:** https://github.com/sanyamgoelx/convertino
- **Licence:** GPL-3.0-or-later (OSI-approved)
- **Description:** A free, offline Windows/macOS tool: select files in File
  Explorer or Finder (or an app's Open/Save window), press a shortcut, and pick
  a format on a radial wheel to convert them. Uses FFmpeg, ImageMagick,
  Poppler, Ghostscript, Pandoc, LibreOffice and 7-Zip.
- **Build system:** GitHub Actions (`.github/workflows/release.yml`), Tauri 2
  (Rust); artifacts: NSIS installer (`*-setup.exe`) and MSI.
- **What gets signed:** the installer and `convertino.exe`.
- **Maintainer:** Crofty (GitHub: sanyamgoelx)

Their conditions, in short: the software is open source, has no malware or
unwanted behaviour, is built in public CI from the public repo, and the
project page says which certificate signs it ("Free code signing provided by
SignPath.io, certificate by SignPath Foundation").

## After approval

SignPath gives an organisation id, a project slug and an API token. Then:

1. Add the token to the repo: Settings › Secrets › Actions › `SIGNPATH_API_TOKEN`.
2. Tell Claude the organisation id and project slug: the release workflow gets a
   step with `signpath/github-action-submit-signing-request` that sends the
   Windows installer for signing before it's uploaded.
