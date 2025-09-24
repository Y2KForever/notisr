# Notisr

<div align="center">
  <img src="https://raw.githubusercontent.com/y2kforever/notisr/main/src-tauri/icons/icon.png" alt="Logo" width="200"/>

[![Release](https://github.com/Y2KForever/notisr/workflows/Release/badge.svg)](https://github.com/Y2KForever/notisr/actions?query=workflow:"Release")
![GitHub Release](https://img.shields.io/github/v/release/Y2KForever/Notisr?display_name=release)
[![License](https://img.shields.io/badge/License-GPL--3.0-blueviolet)](https://github.com/Y2KForever/notisr/blob/main/LICENSE)
[![Greater Good](https://good-labs.github.io/greater-good-affirmation/assets/images/badge.svg)](https://good-labs.github.io/greater-good-affirmation)

> **A beautiful desktop notifications app built with Tauri and React**

</div>

---

## Quick Start

### Download & Install

**Latest Release: v0.1.0**

|  Platform   |                                  Download                                  |      Instructions       |
| :---------: | :------------------------------------------------------------------------: | :---------------------: |
|  **macOS**  |  [notisr-macos.dmg](https://github.com/Y2KForever/notisr/releases/latest)  | Double-click to install |
| **Windows** | [notisr-windows.exe](https://github.com/Y2KForever/notisr/releases/latest) |    Run the installer    |

### Quick Start

1. Download for your platform above
2. Install and launch the app
3. Follow the setup wizard
4. Start receiving notifications!

---

## Features

- **Smart Notifications** - Customizable notification system with rich content support
- **Beautiful UI** - Modern, dark/light theme support with smooth animations
- **Lightning Fast** - Built with Tauri for native performance and minimal resource usage
- **Privacy First** - Your data stays on your device, no telemetry or tracking
- **Cross-Platform** - Seamless experience on macOS and Windows
- **Real-time Updates** - Instant notification delivery with minimal latency
- **Customizable** - Adjustable notification duration, position, and behavior

---

## Building from Source

### Prerequisites

- **Node.js** (20.x or higher)
- **Rust** (1.80.x or higher)
- **Yarn** (1.x) or npm
- **Platform-specific build tools** (Xcode Command Line Tools for macOS, Visual Studio Build Tools for Windows)

### Build Instructions

```bash
# Clone the repository
git clone https://github.com/Y2KForever/notisr.git
cd notisr

# Install dependencies
yarn install

# Set up environment configuration
create a .env file (see configuration)

# Start development server
yarn tauri dev

# Build for production
yarn tauri build
```

## Platform-Specific Setup

<details><summary><strong>MacOS</strong></summary>

```bash
# Install Xcode Command Line Tools
xcode-select --install

# Verify Rust installation
rustc --version
```

</details>
<details><summary><strong>Windows</strong></summary>

```bash
# Install Visual Studio Build Tools
# Download from: https://visualstudio.microsoft.com/visual-cpp-build-tools/

# Install Rust via rustup
rustup default stable
```

</details>

## Configuration
### Environment Setup
Create a .env file in `src-tauri` based on the provided template:
```
# Authentication
REDIRECT_URI=your_redirect_uri_here
CLIENT_ID=your_client_id_here
CLIENT_SECRET=your_client_secret_here
SCOPE=your_scope_here

# API Endpoints
APPSYNC_HTTP_URI=your_appsync_http_endpoint
APPSYNC_REALTIME_URI=your_appsync_realtime_endpoint
BASE_URI=your_base_api_uri
```
*N.B: Scopes need to be defined with quoation marks, i.e. "user:read:follows"*

## Release Notes
Detailed release notes and changelog are available in [CHANGELOG.md](https://github.com/Y2KForever/notisr/blob/main/CHANGELOG.md).

## Frequently Asked Questions

**Q: Is Notisr free to use?**  
**A:** Yes! Notisr is completely free and open source under the GPL-3.0 license.

**Q: Which platforms are supported?**  
**A:** Currently macOS and Windows, with Linux support possible in the future.

**Q: Does Notisr collect any personal data?**  
**A:** No! Notisr is privacy-focused and does not collect any telemetry or personal data.

**Q: Why do I need to create a .env file for development?**   
**A:** The .env file contains configuration for API endpoints and authentication. For security, these aren't included in the repository.

**Q: Can I build Notisr without Rust installed?**   
**A:** No, since Notisr uses Tauri which requires Rust for the backend.

**Q: How do I update to the latest version?**   
**A:** Download the latest release from GitHub, or if you built from source, pull the latest changes and rebuild.

## License
This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](https://github.com/Y2KForever/notisr/blob/main/license) file for details.

The GPL-3.0 license means you can:
* Use the software commercially
* Modify and distribute the software
* Place warranty on the software

With the requirements that you:
* Include copyright and license notices
* Disclose source code when distributing
* License derivative works under GPL-3.0

---
Made with ❤️ by [Y2KForever](https://twitch.tv/Y2KForever)