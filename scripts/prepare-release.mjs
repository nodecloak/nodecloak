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
  "Windows 应用代理增加断线保护，节点或代理程序故障时阻止指定应用直连。 / Windows application proxy blocks direct connections for assigned apps if a node or proxy process fails.",
  "Windows 的 Claude Code、Codex CLI 和 VS Code Claude 扩展沿用对应应用的节点，兼容 npm／pnpm 安装和手动选择程序路径的已有配置。 / On Windows, Claude Code, Codex CLI and the VS Code Claude extension share their corresponding application nodes, including npm/pnpm installations and existing manual executable selections.",
  "新增 WSL 接入：在应用代理中接入 Ubuntu、开启代理并重开终端或扩展会话后生效。 / Connect Ubuntu from Application Proxy, enable the proxy and reopen terminal or extension sessions to use WSL integration.",
  "改善 WSL 重试、配置恢复和 Ctrl+C 兼容性。受保护会话禁用 Windows 程序互操作；Linux 桌面版应用代理仍不可用。 / Improved WSL retries, configuration restoration and Ctrl+C handling. Protected sessions disable Windows-program interoperability; Linux desktop application proxy remains unavailable."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n提供 Windows x64 安装版和便携版、macOS Apple Silicon 和 Intel 安装包，以及 Ubuntu x64 deb 和 AppImage。 / Downloads are available for Windows x64 (installer and portable), macOS (Apple Silicon and Intel), and Ubuntu x64 (deb and AppImage).\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
