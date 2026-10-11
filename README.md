# NodeCloak

NodeCloak 帮助检查和调整使用 Claude 等 AI 服务时的语言、时区、字体和网络环境。**目标是减少可避免的环境暴露，降低被限制账号或封号的几率。**

NodeCloak 在 Windows 和 macOS 上管理独立的 Chrome、Edge、Firefox 配置。每个副本可以单独设置代理、语言和网站权限，也可以从指定项目目录启动 Claude Code、Codex CLI（ChatGPT）和 Gemini CLI。

[下载安装包](https://github.com/nodecloak/nodecloak/releases/latest) · [官网与环境检测](https://nodecloak.com) · [Telegram 群](https://t.me/nodecloak_official)

## 使用后的环境检测截图

下面是用户提供的使用后复检截图：IPPure 和 FuckClaude 两个环境检测页面均显示 **3 分**。截图展示的是第三方检测页面读到的环境信号，不是 AI 服务的官方账号风控分数，也不能换算成实际封号概率。

### IPPure：3 / 100

该次复检显示时区为 UTC、浏览器语言为 `en-US, en`，剩余计分项为 Emoji 风格（+3）。

![使用 NodeCloak 后的 IPPure 环境检测结果：3 / 100](docs/screenshots/environment-after-ippure.png)

### FuckClaude：3 分，LOW RISK

该次复检中，时区、浏览器语言、中文字体、Intl 区域设置和 UTC 偏移均为 0 分；剩余计分项为 Emoji 风格（+2）和 WebGL 渲染器（+1）。

![使用 NodeCloak 后的 FuckClaude 环境检测结果：3 分，LOW RISK](docs/screenshots/environment-after-fuckclaude.png)

两张截图的分数按各自检测网站的规则计算。复检请使用 NodeCloak 启动的对应副本，具体设置步骤见下文。

## 下载与使用

在 [GitHub Releases](https://github.com/nodecloak/nodecloak/releases/latest) 选择对应的安装包。文件名中的版本号随发布更新。

| 平台 | 文件名后缀 | 用途 |
| --- | --- | --- |
| Windows x64 | `_windows_x64_setup.exe` | 安装到电脑，后续可在应用内更新 |
| Windows x64 | `_windows_x64_portable.exe` | 直接运行；应用内更新时会安装安装版，原便携文件保留 |
| macOS M 系列 | `_macos_arm64.dmg` | 打开 DMG，将应用拖入 Applications |
| macOS Intel | `_macos_x64.dmg` | 打开 DMG，将应用拖入 Applications |

1.0.0 的 Windows 安装版、便携版及安装后的程序已使用提供的证书做 Authenticode 签名。证书发布者为 `Anneng electronic Co. Ltd.`，已于 2014 年 5 月 6 日过期；该签名不能获得 Windows 有效证书信任，也不保证消除 SmartScreen 提示。macOS Apple Silicon 与 Intel 的 DMG 已使用 Nodeloc LLC 的 Developer ID 签名并通过 Apple 公证；应用内更新归档仍保留原发布文件。

使用浏览器副本：

1. 先安装 Chrome、Edge 或 Firefox。NodeCloak 不内置浏览器；未找到 Firefox 时，应用会提供[官方下载入口](https://www.firefox.com/en-US/download/all/desktop-release/)。
2. 在「浏览器副本」点击「新建副本」，填写名称，选择浏览器和代理。在高级设置中选择语言、隐私保护和网站权限。
3. 测试代理后启动副本，在新窗口中登录需要使用的网站。副本有独立的配置目录，不复制日常浏览器的登录信息。
4. 从该副本打开「本地复检」或官网检测页，查看网页实际读到的值。需要保存结果时，将报告导入对应副本。

运行中修改代理或浏览器设置，会显示「待重启」。关闭该副本的全部窗口，再从 NodeCloak 启动，新设置才会应用。只改名称、标签或备注不需要重启。

## 浏览器副本能设置什么

「副本」指独立的浏览器配置目录，不是复制一套浏览器程序。各副本的登录信息、设置和修复记录分开保存。

| 项目 | 当前支持 |
| --- | --- |
| 副本管理 | 名称、标签、备注、搜索、标签筛选、同时启动多个副本、定位窗口、关闭和重启 |
| 代理 | 直连、系统代理、HTTP、HTTPS、SOCKS5；支持账号密码和代理链接填写 |
| 语言 | 跟随出口 IP 匹配，或自定义语言列表 |
| 时区 | 普通模式跟随电脑系统；Firefox 严格指纹保护统一为 UTC。系统时区可在「电脑环境」中修改 |
| 系统时钟 | 独立比较电脑实际时间与 HTTPS 服务端时间；超出 2 分钟容差时提示同步系统时间，取不到参考时显示尚未确认 |
| 定位 | 询问、允许或禁用；位置由浏览器提供，不覆盖坐标 |
| 网站设置 | 启动页面、WebRTC、DNS 隐私、DNT / GPC、通知、摄像头、麦克风和图片加载设置 |
| Firefox 字体 | 限制该副本可见的系统字体，保留电脑上的中文字体 |
| Firefox 其他设置 | 可选严格指纹保护、禁用 WebGL；严格指纹保护与语言的 IP 匹配互斥 |
| 检测与修复 | 检查网络、语言、时区、字体等；按项或一键应用支持的设置，修改前备份 |
| 修复记录 | 查看修改内容，按记录恢复 |
| 删除 | 移至最近删除、恢复、彻底删除；彻底删除会清理该副本数据及代理凭据 |

副本字体可见性和「电脑环境」中的卸载用户字体是两种操作。前者只影响 Firefox 副本；后者会影响使用这些字体的其他应用，需要单独确认并备份。系统时区也属于电脑级设置，会影响所有应用。

复制副本只复制设置和代理，不复制 Cookie、缓存或登录信息。彻底删除的副本不能恢复。详细操作见[浏览器副本指南](docs/browser-profiles.md)。

## 代理和检测结果怎么看

- **直连**只关闭浏览器的 HTTP / SOCKS 代理。软路由、VPN、TUN 仍可能改变出口。VLESS 和服务器上的链式代理需要在原来的网络工具中配置。
- **系统代理**由浏览器读取系统设置。PAC、浏览器策略和域名分流可能让浏览器与应用的检测请求走不同路径。
- **自定义代理**通过该副本的本地转发端口连接指定上游。代理连接失败不会自动切换为直连；它不接管其他应用的流量。
- **Cloudflare 检测出口**是访问检测服务时的出口，不能代表所有网站的出口。应用还会分别检查 `claude.ai`、`claude.com` 的页面与同域名 TCP 出口。
- **HTTP 200**只说明收到页面。地区不可用页面也可能返回 200；应用会识别这些页面和验证挑战，但不会代替浏览器登录检查账号。
- **TCP 检测与 HTTP/3**可能走不同的分流规则。浏览器仍提示地区不可用时，应在报错的副本里确认 Claude 的实际出口、IPv4 / IPv6 和连接协议。

「设置已写入」表示配置已保存；网页实际值需要在重启后的副本中复检。网页检测分数是环境信号的加权结果，不能据此判断账号会不会被封，也不能证明 Claude Code 进程被标记。

遇到 `Incorrect device time`，先在「电脑环境」查看系统时钟，打开系统日期与时间，启用自动设置时间并立即同步。修改时区不会校准时钟，也不要按代理国家手动增减电脑时间。电脑环境显示系统当前值，不采用导入的浏览器报告。若只有挂代理时无法完成验证，再分别检查代理连通性和浏览器设置；语言的 IP 匹配启用了自动化接口，可改为自定义语言后完全退出并重启副本。

Chrome / Edge 使用真实的硬件指纹，没有提供任意 User Agent、Canvas / Audio 噪音或虚拟内存、核心数。选择语言「跟随 IP 匹配」时，浏览器通过原生自动化接口应用设置，网页可以读取自动化标记（`navigator.webdriver=true`）；自定义语言使用浏览器配置，不需要该接口。Firefox 严格指纹保护也可能影响网页显示、Canvas 和媒体功能。

## 终端

侧边栏中文显示「终端」，英文显示「Terminal」。支持以下入口：

| 工具 | 启动命令 |
| --- | --- |
| Claude Code | `claude` |
| Codex CLI（ChatGPT / OpenAI） | `codex` |
| Gemini CLI | `gemini` |
| 普通终端 | Windows PowerShell / macOS Terminal（zsh） |

先按工具卡片中的官方指南安装 CLI，再点击「重新检测」。工作目录填写已存在的完整路径，留空使用用户主目录。点击打开后，在终端中完成 CLI 自己的登录流程。

NodeCloak 为该终端及其子进程设置：

```text
TZ=Asia/Singapore
LANG=en_US.UTF-8
LC_ALL=en_US.UTF-8
```

这些变量不修改电脑时区。已有代理环境变量、API 密钥和 CLI 认证配置保留；浏览器副本的独立代理不会自动应用到终端。NodeCloak 不负责安装 CLI、代登录或提供模型账号。

## 界面、后台运行与更新

- 界面支持简体中文和 English，默认跟随系统语言；在「偏好设置 → 界面语言」切换并保存。界面语言不改变副本语言或终端的语言变量。
- 外观支持浅色、深色和跟随系统。
- 关闭主窗口会收起到托盘，正在运行的副本和代理继续工作。真正退出使用「偏好设置 → 退出应用」或托盘「退出」。
- 0.5.9 及后续版本在启动时和每 6 小时检查更新，也可手动检查。点击「立即更新」后下载、校验更新签名、安装并重启；配置和修复记录保留。
- 更新前需保存浏览器中未完成的输入，并关闭专用副本。0.5.8 及更早版本需要手动下载升级一次。

## 数据保存在哪里

新安装使用以下目录：

| 系统 | 应用数据目录 |
| --- | --- |
| Windows | `%APPDATA%\com.nodecloak` |
| macOS | `~/Library/Application Support/com.nodecloak` |

已有 `com.claudedone` 或 `com.claudeready.desktop` 数据时，升级会继续使用原目录，不移动已有副本和备份。

代理密码保存在 Windows 凭据管理器或 macOS 钥匙串。副本清单记录代理地址、用户名和凭据引用，不保存密码。修复备份保存在本机，不上传。

网络检测会请求 Claude 和 Cloudflare，按 IP 匹配区域会请求 IPWhois，更新检查和下载请求 GitHub。这些服务会收到检测或下载请求及出口 IP。

## 开发与构建

项目使用 Tauri 2、React、TypeScript 和 Rust。

需要 Node.js 24、Rust stable。Windows 还需要 Visual Studio C++ Build Tools 和 WebView2；macOS 需要 Xcode Command Line Tools。

```sh
npm ci
npm run tauri dev
```

只预览界面可运行 `npm run dev`，打开终端输出的本地地址。浏览器预览使用演示数据，不能修改系统、启动真实副本或 CLI。

检查代码：

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

需要启动真实浏览器、访问外部区域服务或读写系统凭据的测试默认跳过，手动运行方法见[浏览器副本指南](docs/browser-profiles.md#验证)。

本机打包：

```sh
node scripts/build-desktop.mjs
```

Windows 上生成 NSIS 安装包和可执行文件；macOS 上生成应用与 DMG。普通本地构建不生成签名更新包。DMG 使用 macOS 构建，GitHub Actions 分别使用 Windows、Apple Silicon Mac 和 Intel Mac。

## 维护者发布

1. 同步 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json` 的版本，并更新 `scripts/prepare-release.mjs` 中的发布说明。更新提示只记录本版实际功能变更与修复，不包含签名、公证、未完成事项或未更新的平台。
2. 执行 `node scripts/check-release-version.mjs`、`node --test scripts/release-checks.mjs` 和上述代码检查。
3. 提交代码，创建并推送与版本一致的 `v<版本>` 标签。三个平台全部构建成功后，GitHub Actions 保存 Release 草稿；完成平台签名和验证后手动公开，已公开的 Release 不会被覆盖。

Release 包含 Windows 安装版、便携版、两种 macOS DMG、签名更新文件、`latest.json`、`SHA256SUMS.txt` 和第三方许可归档。主分支、PR 和手动运行只生成构建产物，不发布 Release；官网需单独同步和部署。

Windows 先签应用程序，再生成安装包并签安装包；更改安装包后必须重新生成 Tauri 更新签名。macOS 应用与 DMG 完成 Developer ID 签名和 Apple 公证后，再从最终应用生成更新归档及签名。未完成公证的新版不替换官网已公证的下载包。

macOS 签名和公证在 GitHub Actions 的 Apple Silicon / Intel runner 上自动完成，不需要本地 Mac。推送版本标签会自动使用证书；也可在 **Actions → Desktop checks and packages → Run workflow** 选择 `main`，保持 `notarize_macos` 勾选，生成两种已公证 DMG 和签名更新归档供下载。手动运行不会覆盖线上 Release。Windows Authenticode 签名与官网同步仍按现有发布流程处理。

仓库 Actions Secrets 需配置 `APPLE_CERTIFICATE`（仅 Developer ID Application 身份的加密 PKCS#12，经 base64 编码）、`APPLE_CERTIFICATE_PASSWORD`、`APPLE_ID`、`APPLE_PASSWORD`（Apple App 专用密码）、`APPLE_TEAM_ID`。每次运行会导入临时钥匙串，验证 Apple 公证凭证，完成后清理；普通主分支与 PR 构建不会导入证书。应用公证票据和更新归档中的票据都会验证，DMG 公证回执单独保存在 Actions artifact 中。

签名私钥放在仓库 Actions Secret `TAURI_SIGNING_PRIVATE_KEY`，公钥配置在 `src-tauri/tauri.conf.json`。Tauri 更新签名用于校验更新文件，不是 Windows Authenticode 签名或 Apple 公证。

下载全部 Release 附件后，可以核验三端更新签名和篡改拒绝：

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example verify-updates -- <附件目录>
```

自行发行时，需要更换更新端点、公钥和下载链接，并在自己的仓库设置签名私钥，否则构建仍会检查本项目的更新。

## 许可与反馈

原创源码与本仓库原创图标使用 [MIT License](LICENSE)。依赖许可见[第三方许可说明](THIRD_PARTY_NOTICES.md)，Chrome、Edge 和 Firefox 图标来源见[浏览器图标说明](public/browsers/README.md)。

问题反馈可使用 [GitHub Issues](https://github.com/nodecloak/nodecloak/issues) 或 [Telegram](https://t.me/nodecloak_official)。请说明系统、应用版本、浏览器类型和复现步骤。不要提交代理密码、API 密钥或浏览器登录数据。

[贡献指南](CONTRIBUTING.md) · [安全反馈](SECURITY.md)
