import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {projectRoot,releaseVersion,verifyReleaseAssets} from './release-utils.mjs';
if(!process.env.RELEASE_TAG) throw new Error('A version tag is required to publish');
const version=releaseVersion();
const directory=path.join(projectRoot,'release-assets');
const files=verifyReleaseAssets(directory,version);
const names={
  'windows-x86_64':`NodeCloak_${version}_windows_x64_setup.exe`,
  'darwin-aarch64':`NodeCloak_${version}_macos_arm64_update.app.tar.gz`,
  'darwin-x86_64':`NodeCloak_${version}_macos_x64_update.app.tar.gz`,
};
const platforms={};
for(const [platform,name] of Object.entries(names)) {
  const signature=fs.readFileSync(path.join(directory,`${name}.sig`),'utf8').trim();
  const decoded=Buffer.from(signature,'base64').toString('utf8');
  if(!decoded.startsWith('untrusted comment:') || !decoded.includes('trusted comment:') || !decoded.includes(`\tversion:${version}`)) throw new Error(`Invalid or mismatched updater signature file: ${platform}`);
  platforms[platform]={url:`https://github.com/nodecloak/nodecloak/releases/download/v${version}/${name}`,signature};
}
// User-facing notices only: no admin features, server internals or build/deployment details.
const releaseNotes=[
  "前置代理新增独立开关，可在节点卡片或编辑页启用、关闭；关闭时保留地址、协议和节点密码，重新开启即可复用。检测节点与实际连接均遵循开关状态。 / First-hop proxies now have an explicit switch on node cards and in the editor. Turning it off retains the address, protocol and node credentials for reuse. Node checks and live connections both follow the switch.",
  "完善 SOCKS5 连接错误提示，区分握手中断、协议或端口不匹配、认证方式及认证失败，方便核对节点配置。 / SOCKS5 errors now distinguish interrupted negotiation, protocol or port mismatches, authentication methods and authentication failures to make troubleshooting easier.",
  "macOS 新增应用代理，支持为 Codex、ChatGPT、Claude 或自选 .app 设置节点，并匹配应用包内的网络 Helper；Windows 应用代理继续保留。 / Application Proxy is now available on macOS for Codex, ChatGPT, Claude or a selected .app, including network helpers inside its bundle. Windows Application Proxy remains available.",
  "应用代理默认关闭，开启时请求系统管理员授权；停止或退出后释放本次接口和路由。macOS 网络切换或从休眠唤醒后会停止分流并提示重新开启。 / Application Proxy is off by default and requests administrator permission when started. Stopping or quitting releases its interface and routes. On macOS, changing networks or waking from sleep stops routing and prompts you to restart it.",
  "修复关闭前置代理后更换 HY2 节点仍受旧前置协议限制的问题；更改前置配置后清除旧检测结果，避免显示过时出口。 / A disabled first hop no longer prevents switching to HY2 because of its cached protocol. Changing first-hop settings clears outdated node-check results."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n已有浏览器副本和修复记录继续保留。0.5.9 及后续版本可在应用内检查更新。\n\n提供 Windows x64 安装版和便携版，以及 macOS Apple Silicon 和 Intel 安装包。\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
