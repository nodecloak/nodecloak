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
  "Windows 新增独立「应用代理」，可为 Codex、ChatGPT、Claude 或自选程序设置代理节点，按程序路径分流。应用代理暂仅支持 Windows。 / Windows gains an independent Application Proxy workspace. Assign proxy nodes to Codex, ChatGPT, Claude or a selected executable using path-based rules. Application Proxy is currently Windows-only.",
  "应用代理默认关闭。先保存节点并为已安装应用选择节点，再点击开启并完成 Windows 管理员授权；临时使用 TUN 路由，未匹配的应用直连，系统 HTTP 代理和 DNS 设置保持不变。 / Application Proxy is off by default. Save a node, assign it to an installed app, then start and approve Windows administrator permission. Temporary TUN routing sends unmatched applications directly while preserving system HTTP proxy and DNS settings.",
  "支持 HTTP、HTTPS、SOCKS5、VLESS、VMess 和 Hysteria2 节点，保留可选的本机或软路由前置代理。Hysteria2 链式连接需要 SOCKS5 UDP 支持。 / Supports HTTP, HTTPS, SOCKS5, VLESS, VMess and Hysteria2 nodes, with an optional local or router first hop. Hysteria2 chaining requires SOCKS5 UDP support.",
  "新增匹配连接与运行记录，可查看应用实际命中的节点、目标及收发流量。节点不可用时，已匹配的连接不会自动切换直连。 / Matching connections and activity records show the selected node, destination and traffic for observed app connections. Matched connections do not automatically fall back to direct when their node fails.",
  "停止应用代理后释放本次接口和路由；修改规则前需先停止，退出 NodeCloak 或应用内更新时也会停止。应用自身代理、本机回环和共享宿主流量不保证由所选程序规则接管。 / Stopping releases the session interface and routes. Stop before editing rules; quitting NodeCloak or preparing an in-app update also stops routing. App-specific proxies, loopback and shared-host traffic are not guaranteed to follow the selected executable rule.",
  "浏览器副本继续使用原来的独立代理方式，Chrome、Edge、Firefox 的节点代理、稳定指纹、MCP、同步和分享功能继续保留。统一应用代理页面的标题与标签样式。 / Browser profiles retain their independent proxy behavior. Chrome, Edge and Firefox node proxies, stable fingerprints, MCP, sync and sharing remain available. Application Proxy headings and labels now follow the shared visual style."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n已有浏览器副本和修复记录继续保留。0.5.9 及后续版本可在应用内检查更新。\n\n提供 Windows x64 安装版和便携版，以及 macOS Apple Silicon 和 Intel 安装包。\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
