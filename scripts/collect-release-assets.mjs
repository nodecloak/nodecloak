import fs from 'node:fs';
import path from 'node:path';
import {projectRoot,releaseVersion,packageNames} from './release-utils.mjs';
const version=releaseVersion();
const platform=process.argv[2];
const native={'windows-x64':['win32','x64'],'macos-arm64':['darwin','arm64'],'macos-x64':['darwin','x64']}[platform];
if(!native || native[0]!==process.platform || native[1]!==process.arch) throw new Error(`Runner does not match release platform: ${platform}`);
const release=path.join(projectRoot,'src-tauri/target/release');
const names=packageNames(version);
const files=platform==='windows-x64'
  ? [[`bundle/nsis/NodeCloak_${version}_x64-setup.exe`,names[0]],['nodecloak.exe',names[1]]]
  : [[`bundle/dmg/NodeCloak_${version}_${platform==='macos-arm64'?'aarch64':'x64'}.dmg`,names[platform==='macos-arm64'?2:3]]];
if(process.env.RELEASE_TAG || (process.platform==='darwin' && process.env.MACOS_NOTARIZE==='true')) {
  if(platform==='windows-x64') files.push([`${files[0][0]}.sig`,`${names[0]}.sig`]);
  else {
    const name=`NodeCloak_${version}_${platform.replace('-','_')}_update.app.tar.gz`;
    files.push(['bundle/macos/NodeCloak.app.tar.gz',name],['bundle/macos/NodeCloak.app.tar.gz.sig',`${name}.sig`]);
  }
}
const output=path.join(projectRoot,'release-assets');
fs.mkdirSync(output,{recursive:true});
for(const [relative,name] of files) {
  const source=path.join(release,relative);
  if(!fs.statSync(source).isFile() || fs.statSync(source).size===0) throw new Error(`Missing build package: ${source}`);
  fs.copyFileSync(source,path.join(output,name));
  console.log(`Collected ${name}`);
}
