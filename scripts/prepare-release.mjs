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
  "新增 VLESS、VMess 和 Hysteria2 节点代理，可直接粘贴节点链接导入，并用于 Chrome、Edge 和 Firefox 浏览器副本。 / Added VLESS, VMess and Hysteria2 node proxies. Paste a node link to use it with Chrome, Edge or Firefox profiles.",
  "每个运行中的副本使用独立代理，不修改系统代理或系统 DNS，也不启用 TUN；副本关闭后自动停止对应代理。 / Each running profile uses an independent proxy without changing system proxy or DNS settings or enabling TUN. Its proxy stops when the profile closes.",
  "节点支持前置代理链路，保留本机或软路由代理设置。Hysteria2 链式连接需要支持 UDP 的 SOCKS5 前置代理，暂不支持混淆和端口跳跃。 / Node connections support a local or router first hop. Hysteria2 chaining requires SOCKS5 UDP forwarding; obfuscation and port hopping are not supported yet.",
  "节点认证信息安全保存在系统凭据存储，支持副本复制和加密同步；已有 HTTP、HTTPS、SOCKS5 和 API 代理继续使用。 / Node credentials are kept in the system credential store and support profile copying and encrypted sync. Existing HTTP, HTTPS, SOCKS5 and API proxies remain available. 同步节点配置的设备需升级至 4.0.0。 / Devices syncing node configurations need version 4.0.0.",
  "改善浏览器关闭与重启时的进程回收和等待，代理失败时不会自动切换直连。 / Improved browser process cleanup and restart waiting. Proxy failures never automatically fall back to a direct connection.",
  "改善 macOS 浏览器副本关闭和再次启动时的兼容性。 / Improved compatibility when closing and reopening browser profiles on macOS."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n已有浏览器副本和修复记录继续保留。0.5.9 及后续版本可在应用内检查更新。\n\n提供 Windows x64 安装版和便携版，以及 macOS Apple Silicon 和 Intel 安装包。\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
