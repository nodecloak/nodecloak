<p align="center">
  <img src="public/brand/app-icon.svg" width="88" height="88" alt="NodeCloak logo">
</p>

<h1 align="center">NodeCloak</h1>

<p align="center">
  <strong>把浏览器账号分开用，让不同应用走各自的代理。</strong>
</p>

<p align="center">
  An open-source desktop app for separate browser profiles and per-app proxies.
</p>

<p align="center">
  <a href="https://github.com/nodecloak/nodecloak/releases/latest"><img src="https://img.shields.io/github/v/release/nodecloak/nodecloak?style=flat-square&color=2f8f68" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/nodecloak/nodecloak?style=flat-square&color=2f8f68" alt="MIT License"></a>
  <img src="https://img.shields.io/badge/platform-Windows%20%7C%20macOS-4b5563?style=flat-square" alt="Windows and macOS">
</p>

<p align="center">
  <a href="https://github.com/nodecloak/nodecloak/releases/latest">下载</a> ·
  <a href="#快速上手">快速上手</a> ·
  <a href="#文档">文档</a> ·
  <a href="https://nodecloak.com">官网</a> ·
  <a href="https://github.com/nodecloak/nodecloak/issues">反馈问题</a> ·
  <a href="https://t.me/nodecloak_official">Telegram</a>
</p>

## 这是做什么的？

**NodeCloak 是一个开源桌面工具，用来管理多个互相分开的浏览器环境，也能给指定应用设置代理。**

比如，你想在同一台电脑上分别使用工作账号、个人账号和测试账号，就可以为它们各建一个「浏览器副本」。每个副本都有自己的 Cookie、网站登录状态和设置，也可以使用不同的代理。打开 NodeCloak，选择需要的副本启动即可。

这里的「副本」是一份独立的浏览器配置，不需要再安装一整套浏览器。NodeCloak 使用电脑上已有的 **Chrome、Edge 或 Firefox**。

如果你还想让 **Codex、ChatGPT、Claude 或其他桌面程序**分别使用指定节点，可以使用独立的「应用代理」功能。

## 能做什么？

| 功能 | 用起来是什么样 |
| --- | --- |
| **浏览器副本** | 工作、个人和测试环境分开保存，支持命名、标签、搜索、复制配置、最近删除和一键重置 |
| **每个副本单独设置代理** | 支持 HTTP、HTTPS、SOCKS5、VLESS、VMess、Hysteria2；也可先经过本机或软路由代理，再连接指定节点 |
| **稳定指纹与区域设置** | 新副本使用独立指纹种子，同一副本重启后保持稳定；可重新生成，并设置语言、时区及 Canvas、字体、WebGL、音频、媒体设备噪音 |
| **应用代理** | 为指定程序选择节点，按程序路径分流，并查看实际匹配的连接与流量 |
| **多设备同步** | 加密同步副本配置和可迁移的网站登录状态，在同步页面查看进度和处理冲突 |
| **MCP 自动化** | 授权 AI 客户端管理副本、读取网页、点击、填表、截图，以及上传和下载文件 |
| **终端启动器** | 从指定目录打开 Claude Code、Codex CLI、Gemini CLI 或普通终端，为各工具保存时区、语言和环境变量 |
| **环境检测与修复** | 查看网络出口和浏览器环境，检查系统时间等设置，查看启动日志及修改记录 |

浏览器副本和应用代理均支持 Windows 和 macOS。不同浏览器对高级设置的支持方式有所不同。

## 下载

前往 **[GitHub Releases](https://github.com/nodecloak/nodecloak/releases/latest)**，选择适合电脑的文件：

| 你的电脑 | 选择的文件后缀 | 安装方式 |
| --- | --- | --- |
| Windows x64 | `_windows_x64_setup.exe` | 运行安装程序 |
| Windows x64，想直接运行 | `_windows_x64_portable.exe` | 下载后直接打开 |
| macOS，Apple Silicon / M 系列 | `_macos_arm64.dmg` | 将应用拖入 Applications |
| macOS，Intel | `_macos_x64.dmg` | 将应用拖入 Applications |

已安装的用户可以在应用内检查更新。便携版通过应用内更新时会安装安装版，原便携文件保留。

## 快速上手

### 创建一个独立浏览器环境

1. 在电脑上安装 **Chrome、Edge 或 Firefox**。
2. 打开 NodeCloak，在「浏览器副本」点击「新建副本」，填写名称并选择浏览器。
3. 按需设置代理、语言、时区和隐私选项；无需代理时可选择直连。
4. 点击「启动」，在新打开的浏览器窗口里登录和使用网站。以后从同一个副本打开，继续使用它自己的登录数据。

例如，你可以建立「工作」「个人」「测试」三个副本，分别保存各自的账号和代理配置。

修改运行中的代理或浏览器设置后，需要关闭该副本的全部窗口，再从 NodeCloak 启动，才会应用新设置。只改名称、标签或备注不需要重启。

### 给应用指定代理

1. 打开侧栏的「应用代理」，添加一个节点。
2. 在已识别的应用旁选择节点，或点击「添加应用」选择程序（Windows 选 `.exe`，macOS 选 `.app`）。
3. 点击「开启应用代理」，完成系统管理员授权。
4. 让目标应用重新建立连接，在「连接与诊断」查看是否匹配了指定节点。

应用代理默认关闭，开启后临时创建 TUN 接口；停止时释放对应接口和路由。未匹配的程序直连，系统 HTTP 代理和 DNS 配置保持不变。应用自身的代理和本机回环连接可能不走该规则；其他 VPN / TUN 已接管默认路由时，先停止它再开启应用代理。详见[应用代理说明](docs/application-proxy.md)。

### 让 AI 通过 MCP 操作浏览器

打开侧栏的「MCP」，添加客户端，选择允许访问的副本、网站和操作，再把生成的连接配置复制到 AI 客户端。保持 NodeCloak 运行，即可让 AI 操作授权的浏览器。

例如，你可以对 AI 说：

> 启动我的测试副本，打开指定网站，填写这份表单并截图，提交前让我确认。

Chrome、Edge、Firefox 均支持。连接方式、权限和工具清单见 [MCP 使用指南](docs/mcp.md)。

## 常见问题

**会复制我日常浏览器的登录数据吗？**

新建副本使用独立的数据目录，不会导入日常浏览器的 Cookie 和登录信息。「复制配置」复制设置和代理，同时生成独立指纹，不复制 Cookie、缓存或登录数据。

**浏览器代理和应用代理有什么区别？**

浏览器代理只用于对应副本，不需要接管系统网络。应用代理使用独立的 TUN 分流，为指定程序的新连接选择节点。两者分别设置；副本代理不会自动应用到终端或其他应用。

**多台电脑能继续使用同一个环境吗？**

可以登录账号并启用云同步。首次启用时保存恢复密钥，新设备按提示恢复或授权。同步支持配置和可迁移的网站登录状态，部分网站或设备绑定的数据仍需要重新登录。

**关掉主窗口就退出了吗？**

关闭主窗口会收起到托盘，运行中的副本和代理继续工作。需要完全退出时，在托盘选择「退出」。

**数据保存在什么地方？**

Windows 默认使用 `%APPDATA%\com.nodecloak`，macOS 默认使用 `~/Library/Application Support/com.nodecloak`。已有旧版数据时继续使用原目录。代理密码保存在系统凭据存储，云同步内容在客户端加密后上传。

## 文档

| 你想了解什么 | 入口 |
| --- | --- |
| 浏览器副本的操作与排查 | [浏览器副本指南](docs/browser-profiles.md) |
| 应用代理、节点和前置代理 | [应用代理说明](docs/application-proxy.md) |
| 连接 AI 客户端、权限与工具调用 | [MCP 使用指南](docs/mcp.md) |
| 当前版本的变化 | [更新记录](https://github.com/nodecloak/nodecloak/releases) |
| 查询出口 IP、纯净度和网络信息 | [IP.NodeCloak](https://ip.nodecloak.com) |

<details>
<summary>查看历史环境检测截图</summary>

以下是用户提供的第三方环境检测结果，仅展示该次检测读到的环境信号。分数按各检测网站的规则计算，不能换算为账号风控结果。

![环境检测示例：IPPure 3 / 100](docs/screenshots/environment-after-ippure.png)

![环境检测示例：FuckClaude 3 分](docs/screenshots/environment-after-fuckclaude.png)

</details>

## 本地开发

技术栈：**Tauri 2 · React · TypeScript · Rust**。

需要 Node.js 24 和 Rust stable。Windows 需要 Visual Studio C++ Build Tools 和 WebView2；macOS 需要 Xcode Command Line Tools。

```sh
git clone https://github.com/nodecloak/nodecloak.git
cd nodecloak
npm ci
npm run tauri dev
```

只预览界面可运行 `npm run dev`；浏览器预览使用演示数据。真实副本、代理和终端功能需要桌面端。

检查与打包：

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
node scripts/build-desktop.mjs
```

打包命令生成当前系统的桌面包；macOS 包需要在 macOS 上构建。涉及真实浏览器、网络和系统凭据的部分测试默认跳过，运行方法见[浏览器测试说明](docs/browser-profiles.md#验证)。自行发行时，请配置自己的更新端点、公钥和更新凭据。

## 参与贡献

欢迎提交 [Issue](https://github.com/nodecloak/nodecloak/issues) 和 [Pull Request](https://github.com/nodecloak/nodecloak/pulls)。反馈问题时请附上系统、NodeCloak 版本、浏览器类型、复现步骤及去除敏感信息的日志。

请勿上传 Cookie、浏览器登录数据、代理密码或 API 密钥。贡献方式见 [CONTRIBUTING.md](CONTRIBUTING.md)，安全问题见 [SECURITY.md](SECURITY.md)。也可以加入 [Telegram 群](https://t.me/nodecloak_official)交流。

## 开源许可

本仓库原创源码与原创图标使用 [MIT License](LICENSE)。依赖许可见[第三方许可说明](THIRD_PARTY_NOTICES.md)，浏览器图标来源见[浏览器图标说明](public/browsers/README.md)。

## 友情链接

[LINUX DO — 新的理想型社区](https://linux.do/)
