# PromptMan Distribution & Direct Download Guide

This guide details the pre-built distribution export files created for **Windows**, **macOS**, and **Linux**, how end users run them, and how to host them on your official website.

---

## 1. Export Files Summary

| Operating System | Export File | Format | Target Users | How to Run |
|---|---|---|---|---|
| **Windows** | `PromptMan-v1.0.0-windows-x64.zip` | Portable Zip (~15 MB) | Windows 10 & 11 (64-bit) | Extract zip, double-click `promptman.exe` |
| **macOS** | `PromptMan-v1.0.0-macos-universal.dmg` | Apple Disk Image (~25 MB) | Apple Silicon (M1–M4) & Intel Macs | Open DMG, drag `PromptMan.app` to `/Applications` |
| **macOS (Archive)** | `PromptMan-v1.0.0-macos-universal.tar.gz` | Tarball (~20 MB) | Terminal / Homebrew / Scripts | Extract and run `PromptMan.app/Contents/MacOS/promptman` |
| **Linux (Debian/Ubuntu)** | `PromptMan-v1.0.0-linux-amd64.deb` | Debian Package (~18 MB) | Ubuntu, Debian, Mint, Pop!_OS | `sudo apt install ./PromptMan-v1.0.0-linux-amd64.deb` |
| **Linux (Portable)** | `PromptMan-v1.0.0-linux-x86_64.tar.gz` | Tarball (~18 MB) | Any x86_64 Linux distribution | Extract and run `./promptman` (or `./install.sh`) |

All packages include both:
- **`promptman`**: The native desktop GUI application.
- **`promptman-cli`**: The headless CLI for batch evaluation, scripts, and CI pipelines.

---

## 2. Windows Export Package (Built & Ready on Disk)

The Windows release package has already been compiled in release mode and packaged into the `dist/` directory:

```
dist/
├── PromptMan-v1.0.0-windows-x64.zip     <-- Ready to upload to your website!
├── PromptMan-v1.0.0-windows-x64/        <-- Unpacked contents for testing
│   ├── promptman.exe                    <-- Desktop GUI application
│   ├── promptman-cli.exe                <-- Command-line interface
│   ├── icon.ico                         <-- High-resolution Windows app icon
│   ├── LICENSE.txt                      <-- MIT License
│   └── README.txt                       <-- User instructions
└── SHA256SUMS-windows.txt               <-- Cryptographic checksum
```

### To rebuild the Windows package at any time:
```powershell
powershell -ExecutionPolicy Bypass -File scripts\package-windows.ps1
```

---

## 3. macOS & Linux Export Packages (Automated via GitHub Actions)

Because native macOS DMG creation and Linux Debian packages require native operating system SDKs, we have configured a GitHub Actions Release workflow:

- Workflow file: `.github/workflows/release.yml`

### Two Ways to Generate Mac & Linux Packages:

#### Method A: One-Click in GitHub Web UI (`workflow_dispatch`)
1. Go to your GitHub repository: `https://github.com/TIRK3Y/promptman`
2. Click the **Actions** tab.
3. In the left sidebar, click **Release**.
4. Click **Run workflow**, choose your branch (`main`), enter the version number (e.g. `1.0.0`), and click the green button.
5. In ~3 minutes, GitHub builds Windows, macOS (Universal), and Linux packages. You can download all packages directly from the workflow run artifacts or from the automatically created GitHub Release!

#### Method B: Git Tag Trigger
Simply push a release tag:
```bash
git tag v1.0.0
git push origin v1.0.0
```
GitHub Actions will automatically build all platforms and attach the download files to the `v1.0.0` Release page.

### Local Packaging on Mac or Linux (If You Have Access to Those Machines):
- **On macOS**:
  ```bash
  chmod +x scripts/package-macos.sh
  ./scripts/package-macos.sh 1.0.0 universal
  ```
- **On Linux**:
  ```bash
  chmod +x scripts/package-linux.sh
  ./scripts/package-linux.sh 1.0.0
  ```

---

## 4. Hosting on Your Official Website

### Option 1: Direct CDN / Web Server Hosting
Upload the files from `dist/` to your web server or storage bucket (e.g. AWS S3, Cloudflare R2, Firebase Storage, or public directory):
- `https://yourdomain.com/downloads/PromptMan-v1.0.0-windows-x64.zip`
- `https://yourdomain.com/downloads/PromptMan-v1.0.0-macos-universal.dmg`
- `https://yourdomain.com/downloads/PromptMan-v1.0.0-linux-amd64.deb`
- `https://yourdomain.com/downloads/PromptMan-v1.0.0-linux-x86_64.tar.gz`

### Option 2: Link Directly to GitHub Releases
Use permanent, high-speed GitHub Release download URLs:
- `https://github.com/TIRK3Y/promptman/releases/latest/download/PromptMan-v1.0.0-windows-x64.zip`
- `https://github.com/TIRK3Y/promptman/releases/latest/download/PromptMan-v1.0.0-macos-universal.dmg`
- `https://github.com/TIRK3Y/promptman/releases/latest/download/PromptMan-v1.0.0-linux-amd64.deb`
- `https://github.com/TIRK3Y/promptman/releases/latest/download/PromptMan-v1.0.0-linux-x86_64.tar.gz`

---

## 5. End-User Installation & Execution Instructions

Include these instructions on your website's download page (or use the pre-built HTML snippet in `dist/WEBSITE_DOWNLOAD_SECTION.html`):

### Windows Users
1. Download `PromptMan-v1.0.0-windows-x64.zip`.
2. Extract the ZIP archive.
3. Double-click `promptman.exe`.
   > **Note on Windows SmartScreen**: Because this is a newly downloaded open-source executable without a paid commercial certificate, Windows SmartScreen may show *"Windows protected your PC"*. Users simply click **"More info"** and then **"Run anyway"**.

### macOS Users
1. Download `PromptMan-v1.0.0-macos-universal.dmg`.
2. Double-click the DMG and drag `PromptMan.app` into your **Applications** folder.
3. Launch PromptMan from Spotlight or Applications.
   > **Note on macOS Gatekeeper**: On first run on modern macOS (macOS 12+), if macOS reports *"Apple could not verify the developer"*:
   > - Open **System Settings** -> **Privacy & Security** -> scroll down and click **Open Anyway**.
   > - Or run in Terminal: `xattr -cr /Applications/PromptMan.app`

### Linux Users
- **Debian / Ubuntu / Mint / Pop!_OS**:
  ```bash
  sudo apt install ./PromptMan-v1.0.0-linux-amd64.deb
  promptman
  ```
- **Other Linux Distributions (Arch, Fedora, openSUSE)**:
  ```bash
  tar -xzf PromptMan-v1.0.0-linux-x86_64.tar.gz
  cd PromptMan-v1.0.0-linux-x86_64
  ./promptman
  ```
  *(To install desktop icons into your application launcher, run `./install.sh` inside the extracted folder).*
