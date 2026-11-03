# 浏览器副本（0.6.0）

在“浏览器副本”中新建 Chrome、Edge 或 Firefox 环境。浏览器需要事先安装；Firefox 提供全球版官方下载入口。每个副本有独立目录，不复制日常浏览器或其他副本的 Cookie、缓存或登录信息。复制配置只复制设置和代理，认证凭据另存为新条目。

名称、标签、备注用于区分用途。列表可搜索和按标签筛选。“启动”打开启动页；运行中点击“打开窗口”定位原窗口。更多操作中可检测环境、编辑设置、测试代理、查看修复记录、关闭、重启和移至最近删除。

## 代理

- 系统代理：浏览器使用自身支持的系统配置。应用进程测试会读取系统/环境代理，但 PAC、浏览器策略和分流可能不同，请在实际副本内复检。
- 直连：明确关闭浏览器代理。系统 VPN、TUN 和网络路由仍可能改变出口。
- HTTP、HTTPS、SOCKS5：按副本使用本地转发端口连接指定上游；HTTPS 指的是连接代理服务器使用 TLS，证书正常校验。目标 HTTPS 始终端到端传输，不安装拦截证书。
- VLESS、VMess、Hysteria2：在「代理设置」选择协议，粘贴节点链接并点击「导入节点」。Hysteria2 同时接受 `hysteria2://` 和 `hy2://`，支持单端口、SNI 和链接中的证书验证选项；暂不支持混淆和端口跳跃。节点认证信息使用系统凭据存储保存。同步节点配置的设备需升级至 4.0.0。
- 节点代理仅作用于当前副本，不修改系统代理或系统 DNS，不启用 TUN。需要经过本机或软路由时开启「链式代理」；Hysteria2 的前置代理必须支持 SOCKS5 UDP 转发，HTTP / HTTPS 前置不适用。
- 认证密码使用 Windows 凭据管理器或 macOS 钥匙串保存。SOCKS5 目标域名交给上游解析；本地仍需解析代理服务器本身的域名。

可粘贴 `socks5://user:password@host:1080` 等链接自动填写；特殊字符按 URI 规则编码。编辑时密码留空保留原值，选中“清除已保存的密码”可清除。地址、用户名和密码不会显示在命令行参数中；副本清单保留代理地址、用户名及随机凭据引用，不保存密码。

自定义代理配置的网页请求失败时不切换为直连。Chrome / Edge 禁用 QUIC 并设置非代理 UDP 隐私策略；Firefox 自定义代理关闭 WebRTC 并禁止代理失败后直连。它们不接管浏览器之外的系统流量，也不修改硬件指纹。系统代理/PAC 本身可能包含直连规则。

代理测试显示 Cloudflare 出口 IP、国家、请求耗时和 Claude HTTP 状态。403、429 等状态不能简单视为代理无效。系统分流可能让不同目的地址使用不同出口；官网检测页的出口是在当前浏览器访问本站时测得。代理提供者和检测服务会看到普通网络请求。

## 设置、检测和修复

副本编辑分为基础、代理、高级三个页签，右侧实时显示计划配置。预览不代表网站已经检测通过；启动后使用环境复检检查实际结果。运行中修改设置会标记“待重启”，保持当前连接，重启后应用最新配置。只改变名称、标签或备注无需重启。

高级设置面向个人使用，不含团队、成员权限、云同步或账号共享：

- 副本区域：语言支持跟随 IP 或自定义；定位权限支持询问、允许、禁用。普通模式时区跟随电脑，位置由浏览器提供，不再按 IP 或自定义坐标覆盖。
- 启动页面：支持 HTTP、HTTPS 与空白页；区域设置准备完成后再打开目标页面。
- 隐私与指纹：WebRTC 保护与隐私偏好；Firefox 另提供字体可见性限制、可选严格指纹保护、WebGL 禁用。
- 网站权限与内容：通知、摄像头、麦克风的默认询问或禁止，以及默认不加载图片。已有网站例外不会被清除，允许过的网站仍可采用自己的设置。

Firefox 严格指纹保护使用原生 `privacy.resistFingerprinting`，默认关闭。[Mozilla 说明](https://support.mozilla.org/en-US/kb/resist-fingerprinting)描述了它对时区（UTC / Icelandic）、Canvas、语言、屏幕、硬件和媒体信息的统一处理。它并不按代理 IP 自动匹配时区，也不支持单独指定这些信号；部分网页、图片读取、视频会议或显示设置可能受影响。关闭后重启副本可恢复标准行为。该选项仅改变此 Firefox 副本，不改变系统时区或 Claude Code。

Chrome / Edge 使用真实硬件与浏览器指纹；没有提供 Canvas、Audio、WebGPU 噪音、虚拟内存/核心数或任意 UA。这些自定义信号需要另外评估定制浏览器内核。Firefox 字体白名单和严格保护也不会删除电脑字体。

语言「跟随 IP 匹配」使用 Chrome / Edge CDP 及新版 Firefox WebDriver BiDi 原生接口，语言优先级同时写入浏览器配置；Firefox 将网页语言和请求语言收敛到首选语言。该模式与 Firefox 严格指纹保护互斥。自定义语言直接写入浏览器配置，不启用自动化接口。高级设置已移除独立时区和位置来源；旧副本会恢复系统时区及浏览器定位，旧模拟位置的「允许」权限恢复为「询问」，运行中的副本需要重启。

“匹配当前 IP”通过副本的代理路径预览；实际启动从空白页通过浏览器自己的网络路径查询 [IPWhois HTTPS 服务](https://ipwhois.io/documentation)，按实际出口重新匹配语言。若浏览器出口与预览不同，以实际浏览器结果为准；语言需要变更时在打开目标页面前重启一次。国家语言采用常用模板，不代表个人语言，多语言地区可使用自定义。按域名分流时，检测服务与目标网站可能使用不同出口。服务有配额与可用性限制；查询失败会关闭尚未打开目标页面的副本，提示重试或使用自定义语言。修改代理或出口后应重启重新匹配。

启用语言的 IP 匹配时，浏览器会进入可被网站读取的自动化模式（实测 `navigator.webdriver=true`）。界面提供明确提示。控制端口只监听本机回环地址，仅连接该副本目录记录的端口；请保持 NodeCloak 后台运行。本地控制连接断开时，应用先等待浏览器退出，再最多尝试 3 次重连。正常关闭不会显示断线错误；恢复后继续使用原有指纹和区域配置，已中断的点击、输入等操作需确认页面结果后重新发起。持续断开或设置无法恢复时关闭副本并保留错误。部分 Firefox 版本的直接 BiDi 会话无法重新加入，这种情况会在尝试后提示重启。如果应用本身异常退出，需关闭并重启副本。该模式不会改变电脑时区或终端的运行环境。

禁用定位时，Firefox 关闭此副本的定位接口；Chrome / Edge 清除该副本已保存的定位授权，并禁止定位，其他权限与 Cookie 保留。改回询问后 Chrome / Edge 已清除的网站需重新授权。通知、摄像头、麦克风与图片仍采用默认权限，保留已有网站例外。

默认网站权限采用浏览器原生配置：[Firefox 权限参考](https://firefox-admin-docs.mozilla.org/reference/policies/permissions/)、[Chromium 内容设置定义](https://chromium.googlesource.com/chromium/src/+/master/chrome/common/extensions/api/content_settings.json)。新字段兼容旧版副本清单，复制配置保留高级设置而不复制 Cookie；修改和还原仅写受控字段，保留其他设置与登录数据。

### 控制连接日志与 Google 登录

每个副本的浏览器目录保存 `nodecloak-control.jsonl`，记录 UTC 时间、浏览器类型、断开原因、重连次数和恢复结果。单文件超过 1 MiB 时轮换为 `nodecloak-control.previous.jsonl`。日志记录传输错误类型和系统错误码，不记录网页内容、输入值、代理密码或会话密钥。

Google 的 “This browser or app may not be secure” 表示本次登录未通过其浏览器安全检查。[Google 官方说明](https://support.google.com/accounts/answer/7675428?co=GENIE.Platform%3DDesktop&hl=zh-Hans)列出了软件自动化控制等限制。稳定指纹、语言 IP 匹配、部分区域/硬件/视口设置或 MCP 启动会启用控制接口；普通手动启动在不需要这些控制功能时不会启用接口。

复制配置时继承原副本的稳定指纹开关状态，并生成独立种子。未启用稳定指纹的默认副本，复制后继续保持关闭。旧版本已经复制出的副本，如稳定指纹被自动开启，可在高级设置中关闭“稳定指纹”，保存后完全关闭并重新启动。

排查时先在同一网络下使用普通 Chrome 对比同一登录流程，再使用关闭稳定指纹、语言设为自定义且其他受控设置恢复默认的独立副本，通过界面手动启动。MCP 启动会重新启用自动化控制。这样可区分受控浏览器兼容性和账号/网络因素；需要登录后的现场结果才能确认具体原因。更改副本设置后完全关闭并重新启动，保留已有副本作为对照。

### 时间错误与代理验证

「电脑环境」读取系统当前时区和 UTC 偏移；浏览器报告只用于副本检测，不会覆盖这些系统值。窄窗口也会保留实际值。

系统时钟检测复用出口检测请求，通过当前网络路径取得 Cloudflare HTTPS trace 的服务端时间。比较的是实际 UTC 时间，不按时区增减小时；容许网络延迟和 2 分钟误差。响应异常、缓存、网络延迟过长或检测期间时钟变化时，显示尚未确认。检测不会修改电脑时间或关闭 TLS 验证。

网页提示 `Incorrect device time` 时，打开系统日期与时间，启用自动时间并同步，再重启副本。Windows 使用系统日期与时间设置；macOS 使用 [Apple 日期与时间设置](https://support.apple.com/en-gb/guide/mac-help/mchlp2996/mac)。更改时区只改变本地显示时间。

如果仅代理连接时验证失败，先测试该副本代理并检查实际浏览器出口。自定义语言不需要自动化接口；IP 匹配会启用接口。[Cloudflare 不支持自动化浏览器完成生产验证](https://developers.cloudflare.com/cloudflare-challenges/reference/supported-browsers/)，可切换为自定义语言后重启副本。代理协议连通不代表网站验证或账户访问一定成功。

副本环境检测与修复记录按 ID 隔离。从工具打开本地复检页或官网检测页后，报告带副本标记；新副本只接受同一 ID 的报告，防止同类浏览器之间混用结果。报告标记不构成身份验证，不读取账号或 Cookie。

“电脑环境”里的系统时区、UTC 偏移、用户字体卸载和 Claude Code 启动器是电脑级设置；时区或字体修改前需关闭全部专用副本。Firefox 字体可见性只针对该副本，电脑字体保留。修复或撤销后的实际值不会在下次启动时被旧默认值覆盖。

## 迁移、后台运行和删除

首次启动自动登记原来的三个默认专用浏览器。原目录保留，不移动已有登录数据。新副本位于应用数据目录的 `profiles/<随机 ID>/browser-<类型>`，记录位于同一 ID 的目录中。

关闭主窗口只隐藏到托盘，保持浏览器和代理运行。点击托盘“打开 NodeCloak”或再次启动应用可恢复窗口。真正退出使用“偏好设置 → 退出应用”或托盘“退出”，先保存输入，再关闭专用副本。应用内更新也会在检测到运行中的副本时提示关闭。

移至最近删除保留配置和数据，支持恢复。彻底删除清理该副本目录、修复记录和已保存的代理凭据，无法恢复；删除原默认副本时保留电脑级记录和其他副本数据。浏览器正在运行时不能删除。

## 验证

```sh
npm test
npm run build
cargo test --manifest-path src-tauri/Cargo.toml --locked
```

以下测试使用真实本机资源，默认不在 CI 中启动窗口：

```sh
cargo test --manifest-path src-tauri/Cargo.toml native_password_lifecycle -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml real_browser_profiles -- --ignored
cargo test --manifest-path src-tauri/Cargo.toml advanced_browser_preferences -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml native_regions_are_observed -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_ip_matching -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml live_https_clock_reference -- --ignored --nocapture
cargo test --manifest-path src-tauri/Cargo.toml real_socks_browser -- --ignored --nocapture
```

第二项需要 Chrome，创建两个临时窗口和测试代理，验证 Cookie、代理及关闭动作隔离，完成后清理临时副本。不要同时重复运行同一个测试。

第三项需要 Chrome 与 Firefox，使用临时副本和本机测试页，验证浏览器实际读到的通知/定位权限、图片加载，以及 Firefox 的 UTC 偏移和 WebGL 禁用；完成后关闭并清理测试副本，不改日常浏览器。

`native_regions_are_observed` 保留对底层旧区域接口的验证，不代表当前界面提供独立时区或坐标设置。`live_ip_matching` 通过真实 HTTPS 服务验证语言 IP 匹配的完整启动流程，包括语言变化时的重新启动；需联网。浏览器测试均使用临时配置并在结束后清理。

时钟测试取得真实 HTTPS 时间参考，不修改系统时间。SOCKS5 浏览器测试使用本地模拟代理和临时 Chrome 副本，确认旧时区设置已停用、时间戳和偏移来自系统、没有启用自动化接口；不登录账户。
