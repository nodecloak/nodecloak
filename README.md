# NodeCloak

NodeCloak 管理独立浏览器工作空间，为每个副本设置代理、语言和隐私选项，并检查网络连接与浏览器环境。

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

使用浏览器副本：

1. 先安装 Chrome、Edge 或 Firefox。NodeCloak 不内置浏览器；未找到 Firefox 时，应用会提供[官方下载入口](https://www.firefox.com/en-US/download/all/desktop-release/)。
2. 在「浏览器副本」点击「新建副本」，填写名称，选择浏览器和代理。在高级设置中选择语言、隐私保护和网站权限。
3. 测试代理后启动副本，在新窗口中登录需要使用的网站。副本有独立的配置目录，不复制日常浏览器的登录信息。
4. 从该副本打开「本地复检」或官网检测页，查看网页实际读到的值。需要保存结果时，将报告导入对应副本。

运行中修改代理或浏览器设置，会显示「待重启」。关闭该副本的全部窗口，再从 NodeCloak 启动，新设置才会应用。只改名称、标签或备注不需要重启。

## 账号与云同步（2.0.0）

从左下角登录入口使用 Google 或邮箱登录。头像菜单可进入个人资料、授权设备和偏好设置；个人资料支持编辑头像、用户名、设置密码及两步验证。Google OAuth 登录使用提供方验证流程，邮箱和密码登录在启用两步验证后需要验证第二因素。

云同步在客户端加密后上传，支持浏览器副本配置和可迁移的网站登录状态。首次启用需保存恢复密钥；新设备需完成恢复或设备授权。可在偏好设置中关闭自动同步、选择是否同步网站登录状态及拉取云端新增副本，也可手动同步并处理冲突。服务端管理后台只能查看存储元数据，无法读取加密的浏览器内容。

网站可能要求重新登录；系统绑定或不可导出的登录数据无法保证跨设备沿用。恢复密钥应由用户自行保管。

## 浏览器副本能设置什么

「副本」指独立的浏览器配置目录，不是复制一套浏览器程序。各副本的登录信息、设置和修复记录分开保存。

| 项目 | 当前支持 |
| --- | --- |
| 副本管理 | 名称、标签、备注、搜索、标签筛选、同时启动多个副本、定位窗口、关闭和重启 |
| 代理 | 直连、系统代理、HTTP、HTTPS、SOCKS5、VLESS、VMess、Hysteria2；支持代理链接导入和前置代理 |
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

- **直连**只关闭浏览器的 HTTP / SOCKS 代理。软路由、VPN、TUN 仍可能改变出口。
- **节点代理**可导入 VLESS、VMess、Hysteria2（`hy2://`）链接，每个副本独立使用，不修改系统代理或 DNS。Hysteria2 前置代理需要 SOCKS5 UDP 转发；暂不支持混淆和端口跳跃。
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

先按工具卡片中的官方指南安装 CLI，再点击「重新检测」。工作目录可选择文件夹或填写已存在的完整路径，留空使用用户主目录。点击打开后，在终端中完成 CLI 自己的登录流程。

在「启动环境」为各工具分别选择「继承系统环境」或「自定义环境」。自定义模式可填写时区、语言及任意环境变量，例如代理地址、API 地址和密钥；空白时区/语言字段沿用已有值。默认自定义配置为：

```text
TZ=Asia/Singapore
LANG=en_US.UTF-8
LC_ALL=en_US.UTF-8
```

点击「保存环境」将该工具的配置在本机加密保存，加密密钥保存在系统凭据存储，下次打开时恢复；「打开」直接使用当前表单。变量值默认隐藏，启动预览仅显示变量名称。配置只作用于新终端及其子进程；未覆盖的代理、API 密钥和 CLI 认证配置继续继承。浏览器副本的独立代理不会自动应用到终端。

## 界面、后台运行与更新

- 界面支持简体中文和 English，默认跟随系统语言；在「偏好设置 → 界面语言」切换并保存。界面语言不改变副本语言或终端的语言变量。
- 外观支持浅色、深色和跟随系统。
- 关闭主窗口会收起到托盘，正在运行的副本和代理继续工作。真正退出使用托盘「退出」。
- 0.5.9 及后续版本在启动时和每 6 小时检查更新，也可手动检查。点击「立即更新」后下载、校验、安装并重启；配置和修复记录保留。
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

## MCP 自动化

通过本机 MCP 连接，让 AI 管理授权副本，并在 Chrome、Edge、Firefox 中读取网页、填写表单、截图及处理文件。可在偏好设置中选择副本、网站与操作权限，查看待确认操作和任务记录。见 [MCP 使用指南](docs/mcp.md)。

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

Windows 上生成 NSIS 安装包和可执行文件；macOS 上生成应用与 DMG。DMG 使用 macOS 构建，GitHub Actions 分别使用 Windows、Apple Silicon Mac 和 Intel Mac。

## 维护者发布

1. 同步 `package.json`、`package-lock.json`、`src-tauri/Cargo.toml`、`src-tauri/Cargo.lock` 和 `src-tauri/tauri.conf.json` 的版本，并更新 `scripts/prepare-release.mjs` 中的发布说明。更新提示只记录本版实际功能变更与修复。
2. 执行 `node scripts/check-release-version.mjs`、`node --test scripts/release-checks.mjs` 和上述代码检查。
3. 提交代码，创建并推送与版本一致的 `v<版本>` 标签。三个平台全部构建成功后，GitHub Actions 保存 Release 草稿；完成验证后手动公开，已公开的 Release 不会被覆盖。

Release 包含 Windows 安装版、便携版、两种 macOS DMG、应用更新文件、`latest.json`、`SHA256SUMS.txt` 和第三方许可归档。主分支、PR 和手动运行只生成构建产物，不发布 Release；官网需单独同步和部署。

可在 **Actions → Desktop checks and packages → Run workflow** 选择 `main`，生成各平台构建产物。手动运行不会覆盖线上 Release。

下载全部 Release 附件后，可以核验三端更新文件和篡改拒绝：

```sh
cargo run --manifest-path src-tauri/Cargo.toml --example verify-updates -- <附件目录>
```

自行发行时，需要更换更新端点、公钥和下载链接，并配置自己的更新凭据，否则构建仍会检查本项目的更新。

## 许可与反馈

原创源码与本仓库原创图标使用 [MIT License](LICENSE)。依赖许可见[第三方许可说明](THIRD_PARTY_NOTICES.md)，Chrome、Edge 和 Firefox 图标来源见[浏览器图标说明](public/browsers/README.md)。

问题反馈可使用 [GitHub Issues](https://github.com/nodecloak/nodecloak/issues) 或 [Telegram](https://t.me/nodecloak_official)。请说明系统、应用版本、浏览器类型和复现步骤。不要提交代理密码、API 密钥或浏览器登录数据。

[贡献指南](CONTRIBUTING.md) · [安全反馈](SECURITY.md)

## 友情链接

[LINUX DO — 新的理想型社区](https://linux.do/)
