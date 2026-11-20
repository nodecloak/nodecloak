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
  "新增团队工作区，可邀请已注册的成员加入，为指定成员分配浏览器副本并设置有效期。 / Create team workspaces, invite existing accounts and assign browser profiles to selected members with an expiration date.",
  "成员接受分配后可启动独立的配置快照。团队副本使用系统代理，不包含原副本的登录数据、代理设置或备注。 / Members can run independent configuration snapshots after accepting. Team profiles use the system proxy and exclude the original profile’s sign-in data, proxy settings and notes.",
  "移除成员或退出团队会撤销对应的副本授权。正在运行的团队副本将在下一次授权检查时关闭。 / Removing a member or leaving a team revokes the corresponding grants. Running team profiles close at the next authorization check."
];
const manifest={version,notes:releaseNotes.map(note=>`- ${note}`).join('\n'),pub_date:new Date().toISOString(),platforms};
fs.writeFileSync(path.join(directory,'latest.json'),JSON.stringify(manifest,null,2)+'\n');
files.push('latest.json');
const sums=files.map(name=>`${crypto.createHash('sha256').update(fs.readFileSync(path.join(directory,name))).digest('hex')}  ${name}`).join('\n')+'\n';
fs.writeFileSync(path.join(directory,'SHA256SUMS.txt'),sums);
fs.writeFileSync(path.join(projectRoot,'.release-notes.md'),`NodeCloak ${version}\n\n${manifest.notes}\n\n提供 Windows x64 安装版和便携版、macOS Apple Silicon 和 Intel 安装包，以及 Ubuntu x64 deb 和 AppImage。 / Downloads are available for Windows x64 (installer and portable), macOS (Apple Silicon and Intel), and Ubuntu x64 (deb and AppImage).\n`);
console.log(`Verified complete release ${version}; checksums and release notes written.`);
