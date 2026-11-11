import fs from 'node:fs';
import path from 'node:path';
import crypto from 'node:crypto';
import {projectRoot,releaseVersion,verifyReleaseAssets,updaterPackages} from './release-utils.mjs';
if(!process.env.RELEASE_TAG) throw new Error('A version tag is required to publish');
const version=releaseVersion();
const directory=path.join(projectRoot,'release-assets');
const files=verifyReleaseAssets(directory,version);
const names=updaterPackages(version);
const platforms={};
for(const [platform,name] of Object.entries(names)) {
  const signature=fs.readFileSync(path.join(directory,`${name}.sig`),'utf8').trim();
  const decoded=Buffer.from(signature,'base64').toString('utf8');
  if(!decoded.startsWith('untrusted comment:') || !decoded.includes('trusted comment:') || !decoded.includes(`\tversion:${version}`)) throw new Error(`Invalid or mismatched updater signature file: ${platform}`);
  platforms[platform]={url:`https://github.com/nodecloak/nodecloak/releases/download/v${version}/${name}`,signature};
}
// User-facing notices only: no admin features, server internals or build/deployment details.
const releaseNotes=[
  "新增 Ubuntu 24.04 及以上 x64 桌面版，提供 deb 和 AppImage，可使用独立浏览器副本、节点代理与 MCP。 / Adds an Ubuntu 24.04+ x64 desktop app in deb and AppImage formats, with independent browser profiles, proxy nodes and MCP.",
  "Linux 应用代理暂不可用。AppImage 支持应用内更新；deb 用户请下载安装新版。已有副本、代理配置和修复记录继续保留。 / Application Proxy is not yet available on Linux. AppImage supports in-app updates; deb users can download and install newer versions. Existing profiles, proxy settings and repair history are preserved.",
  "Ubuntu 需要 X11 / XWayland 桌面及原生 Chrome、Edge 或 Mozilla Firefox（deb / tar 版）；暂不支持 Snap / Flatpak 浏览器。 / Ubuntu requires an X11 / XWayland desktop and native Chrome, Edge or Mozilla Firefox (deb / tar); Snap / Flatpak browsers are not supported."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n提供 Windows x64 安装版和便携版、macOS Apple Silicon 和 Intel 安装包，以及 Ubuntu x64 deb 和 AppImage。 / Downloads are available for Windows x64 (installer and portable), macOS (Apple Silicon and Intel), and Ubuntu x64 (deb and AppImage).\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
