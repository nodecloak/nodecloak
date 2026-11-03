import fs from 'node:fs';
import path from 'node:path';
import {execFileSync} from 'node:child_process';
import {projectRoot,releaseVersion} from './release-utils.mjs';
const platforms={'windows-x64':'x86_64-pc-windows-msvc','macos-arm64':'aarch64-apple-darwin','macos-x64':'x86_64-apple-darwin'};
const [platform,target]=process.argv.slice(2);
if(platforms[platform]!==target) throw new Error('Unknown license target');
const version=releaseVersion();
const output=path.join(projectRoot,'release-notices',platform);
fs.mkdirSync(output,{recursive:true});
const corePlatform={'windows-x64':'win32-x64','macos-arm64':'darwin-arm64','macos-x64':'darwin-x64'}[platform];
const coreNotices=path.join(output,'sing-box');fs.mkdirSync(coreNotices,{recursive:true});
fs.copyFileSync(path.join(projectRoot,'docs/third-party/sing-box-LICENSE.txt'),path.join(coreNotices,'LICENSE'));
fs.copyFileSync(path.join(projectRoot,'.tools/singbox/source/sing-box-1.14.4-source.tar.gz'),path.join(coreNotices,'sing-box-1.14.4-source.tar.gz'));
fs.copyFileSync(path.join(projectRoot,'.tools/singbox',corePlatform,'provenance.json'),path.join(coreNotices,'provenance.json'));
fs.writeFileSync(path.join(coreNotices,'README.txt'),'Unmodified upstream sing-box 1.14.4, separately licensed under GPL-3.0-or-later with its upstream naming condition. Exact upstream source is included. Source: https://github.com/SagerNet/sing-box/tree/v1.14.4\n');
fs.copyFileSync(path.join(projectRoot,'LICENSE'),path.join(output,'NodeCloak-LICENSE'));
fs.copyFileSync(path.join(projectRoot,'docs/branding.md'),path.join(output,'branding.md'));
fs.copyFileSync(path.join(projectRoot,'public/browsers/README.md'),path.join(output,'browser-logos.md'));
for (const name of ['Sora-OFL.txt','JetBrainsMono-OFL.txt']) fs.copyFileSync(path.join(projectRoot,'public/brand/fonts',name),path.join(output,name));
const lines=[`# NodeCloak ${version} — ${platform}`, '', 'Dependency license declarations for this build. License and notice files found in installed packages are included below. Development/build dependencies are also listed. Dependencies are not relicensed by NodeCloak.', '', '| Package | Version | License |', '| --- | --- | --- |'];
function notices(source,destination,extra) {
  if(!fs.existsSync(source)) return;
  const files=fs.readdirSync(source).filter(name=>/^(license|licence|copying|notice)/i.test(name) && fs.statSync(path.join(source,name)).isFile()).map(name=>path.join(source,name));
  if(extra) {
    const resolved=path.resolve(source,extra);
    if(resolved.startsWith(source+path.sep) && fs.existsSync(resolved) && !files.includes(resolved)) files.push(resolved);
  }
  for(const file of files) {
    fs.mkdirSync(destination,{recursive:true});
    fs.copyFileSync(file,path.join(destination,path.basename(file)));
  }
}
const lock=JSON.parse(fs.readFileSync(path.join(projectRoot,'package-lock.json'),'utf8'));
for(const [relative,pkg] of Object.entries(lock.packages)) {
  if(!relative || !fs.existsSync(path.join(projectRoot,relative))) continue;
  const name=relative.split('node_modules/').at(-1);
  lines.push(`| npm: ${name} | ${pkg.version} | ${pkg.license || 'See upstream license'} |`);
  notices(path.join(projectRoot,relative),path.join(output,'licenses/npm',`${name.replaceAll('/','__')}-${pkg.version}`));
}
const metadata=JSON.parse(execFileSync('cargo',['metadata','--manifest-path','src-tauri/Cargo.toml','--locked','--filter-platform',target,'--format-version','1'],{cwd:projectRoot,encoding:'utf8',maxBuffer:32*1024*1024}));
for(const pkg of metadata.packages) {
  if(!pkg.source) continue;
  lines.push(`| cargo: ${pkg.name} | ${pkg.version} | ${pkg.license || 'See upstream license'} |`);
  notices(path.dirname(pkg.manifest_path),path.join(output,'licenses/cargo',`${pkg.name}-${pkg.version}`),pkg.license_file);
}
fs.writeFileSync(path.join(output,'THIRD_PARTY_NOTICES.md'),lines.join('\n')+'\n');
console.log(`Collected licenses for ${platform}`);
